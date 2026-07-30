//! WinHelp authored keyword indexes (`|?WBTREE` / `|?WDATA`).

use crate::btree::leaf_pages;
use crate::encoding::decode_windows_1252;
use crate::reader::Reader;
use crate::{HlpError, HlpFile, TopicOffset};

/// All authored keyword tables found in one HLP file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeywordIndex {
    pub tables: Vec<KeywordTable>,
    pub warnings: Vec<String>,
}

/// One keyword table identified by its WinHelp footnote/table character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeywordTable {
    pub id: char,
    pub entries: Vec<KeywordEntry>,
}

/// One authored keyword and every associated topic offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeywordEntry {
    pub keyword: String,
    pub topic_offsets: Vec<TopicOffset>,
    /// Some old compilers encode a keyword target as -1 for macro-driven lookup.
    pub has_macro_target: bool,
}

impl KeywordIndex {
    /// Loads every directory stream matching the classic `|?WBTREE` family.
    /// Missing keyword indexes are valid and produce an empty result.
    pub(crate) fn load(file: &HlpFile) -> Result<Self, HlpError> {
        let mut result = Self::default();
        let mut table_ids: Vec<char> = file
            .directory()
            .iter()
            .filter_map(|entry| keyword_table_id(&entry.name))
            .collect();
        table_ids.sort_unstable();
        table_ids.dedup();

        for id in table_ids {
            let tree_name = format!("|{id}WBTREE");
            let data_name = format!("|{id}WDATA");
            let tree = match file.internal_file(&tree_name) {
                Ok(tree) => tree,
                Err(error) => {
                    result.warnings.push(format!(
                        "keyword table {id} tree {tree_name} could not be opened: {error}"
                    ));
                    continue;
                }
            };
            let data = match file.internal_file(&data_name) {
                Ok(data) => data,
                Err(error) => {
                    result.warnings.push(format!(
                        "keyword table {id} data {data_name} could not be opened: {error}"
                    ));
                    continue;
                }
            };
            match parse_table(id, tree.content, data.content) {
                Ok(table) => result.tables.push(table),
                Err(error) => result
                    .warnings
                    .push(format!("keyword table {id} could not be decoded: {error}")),
            }
        }
        Ok(result)
    }

    /// Returns a table by its identifier, case-insensitively for ASCII letters.
    pub fn table(&self, id: char) -> Option<&KeywordTable> {
        self.tables
            .iter()
            .find(|table| table.id.eq_ignore_ascii_case(&id))
    }

    /// Returns the standard K keyword table when present.
    pub fn default_table(&self) -> Option<&KeywordTable> {
        self.table('K')
    }

    /// Resolves an exact semicolon-delimited lookup against one authored keyword table.
    ///
    /// ALink/KLink names are compared case-sensitively. Duplicate topic offsets are removed
    /// while preserving authored lookup order.
    pub fn lookup_exact(&self, id: char, names: &str) -> Vec<TopicOffset> {
        let Some(table) = self.table(id) else {
            return Vec::new();
        };
        let mut result = Vec::new();
        for name in names.split(';').map(str::trim).filter(|name| !name.is_empty()) {
            for entry in table.entries.iter().filter(|entry| entry.keyword == name) {
                for &offset in &entry.topic_offsets {
                    if !result.contains(&offset) {
                        result.push(offset);
                    }
                }
            }
        }
        result
    }
}

fn keyword_table_id(name: &str) -> Option<char> {
    let bytes = name.as_bytes();
    if bytes.len() != 8 || bytes[0] != b'|' || !bytes[2..].eq_ignore_ascii_case(b"WBTREE") {
        return None;
    }
    let id = bytes[1];
    id.is_ascii_graphic().then_some(char::from(id))
}

fn parse_table(id: char, tree: &[u8], data: &[u8]) -> Result<KeywordTable, HlpError> {
    let mut entries = Vec::new();
    for page in leaf_pages(tree)? {
        let mut page_reader = Reader::new(page, "keyword B+ tree leaf");
        let unused = usize::from(page_reader.read_u16()?);
        let entry_count = page_reader.read_i16()?;
        let _previous = page_reader.read_i16()?;
        let _next = page_reader.read_i16()?;
        if entry_count < 0 {
            return Err(HlpError::invalid(
                "keyword B+ tree leaf",
                "negative entry count",
            ));
        }
        if unused > page.len().saturating_sub(8) {
            return Err(HlpError::invalid(
                "keyword B+ tree leaf",
                "unused-byte count exceeds page capacity",
            ));
        }
        let logical_end = page.len() - unused;
        let records = page.get(8..logical_end).ok_or(HlpError::UnexpectedEof {
            context: "keyword B+ tree records",
        })?;
        let mut reader = Reader::new(records, "keyword B+ tree records");
        for _ in 0..entry_count {
            let raw_keyword = reader.read_c_string()?;
            let count = usize::from(reader.read_u16()?);
            let data_offset = usize::try_from(reader.read_u32()?).map_err(|_| {
                HlpError::invalid("keyword B+ tree record", "data offset does not fit usize")
            })?;
            let keyword = decode_windows_1252(raw_keyword);
            if keyword.is_empty() {
                return Err(HlpError::invalid(
                    "keyword B+ tree record",
                    "empty keyword",
                ));
            }
            let byte_count = count.checked_mul(4).ok_or_else(|| {
                HlpError::invalid("keyword data", "topic-offset list size overflow")
            })?;
            let target_bytes = data
                .get(data_offset..data_offset.checked_add(byte_count).ok_or_else(|| {
                    HlpError::invalid("keyword data", "topic-offset list end overflow")
                })?)
                .ok_or_else(|| {
                    HlpError::invalid(
                        "keyword data",
                        format!(
                            "keyword '{keyword}' references {count} target(s) at offset {data_offset}, outside {} bytes",
                            data.len()
                        ),
                    )
                })?;
            let mut target_reader = Reader::new(target_bytes, "keyword topic-offset list");
            let mut topic_offsets = Vec::with_capacity(count);
            let mut has_macro_target = false;
            for _ in 0..count {
                let offset = target_reader.read_i32()?;
                if offset == -1 {
                    has_macro_target = true;
                } else if offset >= 0 {
                    topic_offsets.push(TopicOffset(offset));
                }
            }
            entries.push(KeywordEntry {
                keyword,
                topic_offsets,
                has_macro_target,
            });
        }
    }

    Ok(KeywordTable { id, entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_lookup_unions_semicolon_delimited_alink_names_in_authored_order() {
        let index = KeywordIndex {
            tables: vec![KeywordTable {
                id: 'A',
                entries: vec![
                    KeywordEntry {
                        keyword: "FIRST".to_owned(),
                        topic_offsets: vec![TopicOffset(10), TopicOffset(20)],
                        has_macro_target: false,
                    },
                    KeywordEntry {
                        keyword: "SECOND".to_owned(),
                        topic_offsets: vec![TopicOffset(20), TopicOffset(30)],
                        has_macro_target: false,
                    },
                ],
            }],
            warnings: Vec::new(),
        };
        assert_eq!(
            index.lookup_exact('A', "FIRST; SECOND"),
            vec![TopicOffset(10), TopicOffset(20), TopicOffset(30)]
        );
        assert!(index.lookup_exact('A', "first").is_empty());
    }

    #[test]
    fn recognizes_keyword_stream_family_only() {
        assert_eq!(keyword_table_id("|KWBTREE"), Some('K'));
        assert_eq!(keyword_table_id("|xwbtree"), Some('x'));
        assert_eq!(keyword_table_id("|KWDATA"), None);
        assert_eq!(keyword_table_id("KWBTREE"), None);
    }

    #[test]
    fn parses_multiple_topic_offsets_for_one_keyword() {
        const PAGE_SIZE: usize = 96;
        let mut tree = vec![0_u8; 38 + PAGE_SIZE];
        tree[0..2].copy_from_slice(&0x293B_u16.to_le_bytes());
        tree[4..6].copy_from_slice(&(PAGE_SIZE as u16).to_le_bytes());
        tree[22..24].copy_from_slice(&0_i16.to_le_bytes());
        tree[26..28].copy_from_slice(&0_i16.to_le_bytes());
        tree[28..30].copy_from_slice(&(-1_i16).to_le_bytes());
        tree[30..32].copy_from_slice(&1_i16.to_le_bytes());
        tree[32..34].copy_from_slice(&1_i16.to_le_bytes());
        tree[34..38].copy_from_slice(&1_u32.to_le_bytes());
        let page = &mut tree[38..];
        let record = b"Install\0";
        let record_len = record.len() + 2 + 4;
        page[0..2].copy_from_slice(&((PAGE_SIZE - 8 - record_len) as u16).to_le_bytes());
        page[2..4].copy_from_slice(&1_i16.to_le_bytes());
        page[4..6].copy_from_slice(&(-1_i16).to_le_bytes());
        page[6..8].copy_from_slice(&(-1_i16).to_le_bytes());
        page[8..8 + record.len()].copy_from_slice(record);
        let cursor = 8 + record.len();
        page[cursor..cursor + 2].copy_from_slice(&2_u16.to_le_bytes());
        page[cursor + 2..cursor + 6].copy_from_slice(&0_u32.to_le_bytes());
        let mut data = Vec::new();
        data.extend_from_slice(&123_i32.to_le_bytes());
        data.extend_from_slice(&456_i32.to_le_bytes());

        let parsed = parse_table('K', &tree, &data).expect("keyword table parses");
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].keyword, "Install");
        assert_eq!(parsed.entries[0].topic_offsets, vec![TopicOffset(123), TopicOffset(456)]);
    }
}
