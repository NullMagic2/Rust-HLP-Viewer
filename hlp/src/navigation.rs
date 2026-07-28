//! WinHelp navigation indexes: context hashes, numeric context maps, topic names, and windows.
//!
//! These structures are deliberately independent of GUI state. They expose the legacy lookup
//! data exactly as stored so the document layer can resolve navigation requests without executing macros.

use std::collections::HashSet;

use crate::encoding::decode_windows_1252;
use crate::reader::Reader;
use crate::{HlpError, HlpFile, TopicOffset, TopicPos};

const BTREE_MAGIC: u16 = 0x293B;
const BTREE_HEADER_SIZE: usize = 38;
const MAX_NAVIGATION_ENTRIES: usize = 1_000_000;

/// One hash-to-topic mapping from the `|CONTEXT` B+ tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextEntry {
    pub hash: i32,
    pub offset: TopicOffset,
}

/// One numeric map-id-to-topic mapping from `|CTXOMAP`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextMapEntry {
    pub map_id: i32,
    pub offset: TopicOffset,
}

/// Optional symbolic context name recorded by HCW 4.0's `|TopicId` tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicIdEntry {
    pub offset: TopicOffset,
    pub name: String,
}

/// Optional default secondary-window number from HCW 4.0's `|Viola` tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultWindowEntry {
    pub offset: TopicOffset,
    pub window_number: i32,
}

/// All navigation-oriented auxiliary streams currently understood by the viewer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NavigationMetadata {
    topic_map: Vec<TopicPos>,
    contexts: Vec<ContextEntry>,
    context_map: Vec<ContextMapEntry>,
    topic_ids: Vec<TopicIdEntry>,
    default_windows: Vec<DefaultWindowEntry>,
}

impl NavigationMetadata {
    /// Loads every optional navigation stream present in an HLP file.
    pub(crate) fn load(file: &HlpFile) -> Result<Self, HlpError> {
        let topic_map = if has_stream(file, "|TOMAP") {
            parse_topic_map(file.internal_file("|TOMAP")?.content)?
        } else {
            Vec::new()
        };

        let contexts = if has_stream(file, "|CONTEXT") {
            let stream = file.internal_file("|CONTEXT")?;
            parse_long_key_btree(stream.content, "|CONTEXT", |reader| {
                Ok(ContextEntry {
                    hash: reader.read_i32()?,
                    offset: TopicOffset(reader.read_i32()?),
                })
            })?
        } else {
            Vec::new()
        };

        let context_map = if has_stream(file, "|CTXOMAP") {
            parse_context_map(file.internal_file("|CTXOMAP")?.content)?
        } else {
            Vec::new()
        };

        let topic_ids = if has_stream(file, "|TopicId") {
            let stream = file.internal_file("|TopicId")?;
            parse_long_key_btree(stream.content, "|TopicId", |reader| {
                Ok(TopicIdEntry {
                    offset: TopicOffset(reader.read_i32()?),
                    name: decode_windows_1252(reader.read_c_string()?),
                })
            })?
        } else {
            Vec::new()
        };

        let default_windows = if has_stream(file, "|Viola") {
            let stream = file.internal_file("|Viola")?;
            parse_long_key_btree(stream.content, "|Viola", |reader| {
                Ok(DefaultWindowEntry {
                    offset: TopicOffset(reader.read_i32()?),
                    window_number: reader.read_i32()?,
                })
            })?
        } else {
            Vec::new()
        };

        Ok(Self {
            topic_map,
            contexts,
            context_map,
            topic_ids,
            default_windows,
        })
    }

    pub fn topic_map(&self) -> &[TopicPos] {
        &self.topic_map
    }

    pub fn contexts(&self) -> &[ContextEntry] {
        &self.contexts
    }

    pub fn context_map(&self) -> &[ContextMapEntry] {
        &self.context_map
    }

    pub fn topic_ids(&self) -> &[TopicIdEntry] {
        &self.topic_ids
    }

    pub fn default_windows(&self) -> &[DefaultWindowEntry] {
        &self.default_windows
    }

    /// Resolves a Windows 3.0 topic number through `|TOMAP`.
    ///
    /// HC30 stores the topic number as the direct array index; do not subtract the historical
    /// first normal topic number (16). Index zero is also meaningful as the project's INDEX topic.
    pub fn topic_pos_for_hc30_number(&self, topic_number: i32) -> Option<TopicPos> {
        usize::try_from(topic_number)
            .ok()
            .and_then(|index| self.topic_map.get(index))
            .copied()
            .filter(|position| position.0 >= 12)
    }

    /// Returns the HC30 INDEX/contents topic from `|TOMAP[0]`.
    pub fn hc30_index_topic_pos(&self) -> Option<TopicPos> {
        self.topic_map.first().copied().filter(|position| position.0 >= 12)
    }

    /// Finds a topic offset by the compiler's context-name hash.
    pub fn offset_for_hash(&self, hash: i32) -> Option<TopicOffset> {
        // Do not assume the producer used Rust's signed-i32 ordering for the serialized long key.
        // A linear lookup is tiny compared with topic decoding and remains correct for unusual compilers.
        self.contexts
            .iter()
            .find(|entry| entry.hash == hash)
            .map(|entry| entry.offset)
    }

    /// Hashes and resolves a symbolic context id.
    pub fn offset_for_context_name(&self, name: &str) -> Option<TopicOffset> {
        self.offset_for_hash(context_hash(name.as_bytes()))
    }

    /// Resolves a numeric `[MAP]` id.
    pub fn offset_for_map_id(&self, map_id: i32) -> Option<TopicOffset> {
        self.context_map
            .iter()
            .find(|entry| entry.map_id == map_id)
            .map(|entry| entry.offset)
    }

    /// Returns the symbolic context name assigned to this exact topic offset, when recorded.
    pub fn context_name_for_offset(&self, offset: TopicOffset) -> Option<&str> {
        self.topic_ids
            .iter()
            .find(|entry| entry.offset == offset)
            .map(|entry| entry.name.as_str())
    }

    /// Returns the HCW default window number explicitly assigned to this topic offset.
    pub fn default_window_for_offset(&self, offset: TopicOffset) -> Option<i32> {
        self.default_windows
            .iter()
            .find(|entry| entry.offset == offset)
            .map(|entry| entry.window_number)
    }
}

fn has_stream(file: &HlpFile, name: &str) -> bool {
    file.directory()
        .iter()
        .any(|entry| entry.name.eq_ignore_ascii_case(name))
}


/// Parses HC30 `|TOMAP`, an array of little-endian TOPICPOS values.
fn parse_topic_map(content: &[u8]) -> Result<Vec<TopicPos>, HlpError> {
    if content.len() % 4 != 0 {
        return Err(HlpError::invalid(
            "|TOMAP",
            format!("stream length {} is not divisible by four", content.len()),
        ));
    }
    let count = content.len() / 4;
    if count > MAX_NAVIGATION_ENTRIES {
        return Err(HlpError::invalid(
            "|TOMAP",
            format!("entry count {count} exceeds safety limit {MAX_NAVIGATION_ENTRIES}"),
        ));
    }
    let mut reader = Reader::new(content, "|TOMAP");
    let mut result = Vec::with_capacity(count);
    while reader.remaining() >= 4 {
        result.push(TopicPos(reader.read_i32()?));
    }
    Ok(result)
}

/// Parses `|CTXOMAP`: a 16-bit count followed by `(MapID, TOPICOFFSET)` pairs.
fn parse_context_map(content: &[u8]) -> Result<Vec<ContextMapEntry>, HlpError> {
    let mut reader = Reader::new(content, "|CTXOMAP");
    let count = usize::from(reader.read_u16()?);
    if count > MAX_NAVIGATION_ENTRIES {
        return Err(HlpError::invalid(
            "|CTXOMAP",
            format!("entry count {count} exceeds safety limit {MAX_NAVIGATION_ENTRIES}"),
        ));
    }
    let required = count
        .checked_mul(8)
        .ok_or_else(|| HlpError::invalid("|CTXOMAP", "entry storage overflow"))?;
    if reader.remaining() < required {
        return Err(HlpError::invalid(
            "|CTXOMAP",
            format!("{count} entries need {required} bytes but only {} remain", reader.remaining()),
        ));
    }

    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(ContextMapEntry {
            map_id: reader.read_i32()?,
            offset: TopicOffset(reader.read_i32()?),
        });
    }
    Ok(entries)
}

#[derive(Debug, Clone, Copy)]
struct NavigationBTreeHeader {
    page_size: usize,
    root_page: i16,
    total_pages: i16,
    levels: i16,
    total_entries: usize,
}

/// Parses a B+ tree whose leaf records begin with a 32-bit numeric key.
fn parse_long_key_btree<T, F>(
    content: &[u8],
    context: &'static str,
    mut parse_entry: F,
) -> Result<Vec<T>, HlpError>
where
    F: FnMut(&mut Reader<'_>) -> Result<T, HlpError>,
{
    let header = parse_navigation_btree_header(content, context)?;
    let mut page_number = header.root_page;

    // Every index-page header stores its leftmost child as PreviousPage. Following it once per
    // level reaches the first leaf without needing to interpret the index key payload.
    for _ in 1..header.levels {
        let bytes = navigation_page(content, header, page_number, context)?;
        let mut reader = Reader::new(bytes, context);
        let _unused = reader.read_u16()?;
        let entries = reader.read_i16()?;
        let previous = reader.read_i16()?;
        if entries < 0 || previous < 0 {
            return Err(HlpError::invalid(context, "invalid index-page header"));
        }
        page_number = previous;
    }

    let mut result = Vec::with_capacity(header.total_entries);
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(page_number) {
            return Err(HlpError::invalid(
                context,
                format!("leaf-page cycle detected at page {page_number}"),
            ));
        }
        let bytes = navigation_page(content, header, page_number, context)?;
        let mut header_reader = Reader::new(bytes, context);
        let unused = usize::from(header_reader.read_u16()?);
        let entry_count = header_reader.read_i16()?;
        let _previous = header_reader.read_i16()?;
        let next = header_reader.read_i16()?;
        if entry_count < 0 {
            return Err(HlpError::invalid(context, "negative leaf entry count"));
        }
        if unused > bytes.len().saturating_sub(8) {
            return Err(HlpError::invalid(context, "leaf unused-byte count exceeds page"));
        }
        let logical_end = bytes.len() - unused;
        let records = bytes.get(8..logical_end).ok_or(HlpError::UnexpectedEof { context })?;
        let mut reader = Reader::new(records, context);
        for _ in 0..entry_count {
            if result.len() >= MAX_NAVIGATION_ENTRIES {
                return Err(HlpError::invalid(context, "navigation entry safety limit exceeded"));
            }
            result.push(parse_entry(&mut reader)?);
        }
        if next < 0 {
            break;
        }
        page_number = next;
    }

    if result.len() != header.total_entries {
        return Err(HlpError::invalid(
            context,
            format!(
                "B+ tree declares {} entries but leaf chain contains {}",
                header.total_entries,
                result.len()
            ),
        ));
    }
    Ok(result)
}

fn parse_navigation_btree_header(
    content: &[u8],
    context: &'static str,
) -> Result<NavigationBTreeHeader, HlpError> {
    let mut reader = Reader::new(content, context);
    let magic = reader.read_u16()?;
    if magic != BTREE_MAGIC {
        return Err(HlpError::InvalidMagic {
            context,
            expected: u32::from(BTREE_MAGIC),
            actual: u32::from(magic),
        });
    }
    let _flags = reader.read_u16()?;
    let page_size = usize::from(reader.read_u16()?);
    let _structure = reader.read_bytes(16)?;
    let must_be_zero = reader.read_i16()?;
    let _splits = reader.read_i16()?;
    let root_page = reader.read_i16()?;
    let must_be_neg_one = reader.read_i16()?;
    let total_pages = reader.read_i16()?;
    let levels = reader.read_i16()?;
    let total_entries = usize::try_from(reader.read_u32()?)
        .map_err(|_| HlpError::invalid(context, "entry count does not fit usize"))?;

    if must_be_zero != 0 || must_be_neg_one != -1 {
        return Err(HlpError::invalid(context, "invalid B+ tree reserved fields"));
    }
    if page_size < 8 || total_pages <= 0 || levels <= 0 {
        return Err(HlpError::invalid(context, "invalid B+ tree geometry"));
    }
    if root_page < 0 || root_page >= total_pages {
        return Err(HlpError::invalid(context, "B+ tree root page is outside storage"));
    }
    if total_entries > MAX_NAVIGATION_ENTRIES {
        return Err(HlpError::invalid(
            context,
            format!("entry count {total_entries} exceeds safety limit {MAX_NAVIGATION_ENTRIES}"),
        ));
    }
    let pages = usize::try_from(total_pages)
        .map_err(|_| HlpError::invalid(context, "negative B+ tree page count"))?;
    let required = BTREE_HEADER_SIZE
        .checked_add(
            pages
                .checked_mul(page_size)
                .ok_or_else(|| HlpError::invalid(context, "B+ tree page storage overflow"))?,
        )
        .ok_or_else(|| HlpError::invalid(context, "B+ tree storage overflow"))?;
    if required > content.len() {
        return Err(HlpError::invalid(
            context,
            format!("B+ tree needs {required} bytes but stream has {}", content.len()),
        ));
    }

    Ok(NavigationBTreeHeader {
        page_size,
        root_page,
        total_pages,
        levels,
        total_entries,
    })
}

fn navigation_page<'a>(
    content: &'a [u8],
    header: NavigationBTreeHeader,
    page_number: i16,
    context: &'static str,
) -> Result<&'a [u8], HlpError> {
    if page_number < 0 || page_number >= header.total_pages {
        return Err(HlpError::invalid(context, "B+ tree page number is outside storage"));
    }
    let page_number = usize::try_from(page_number)
        .map_err(|_| HlpError::invalid(context, "negative B+ tree page number"))?;
    let start = BTREE_HEADER_SIZE
        .checked_add(
            page_number
                .checked_mul(header.page_size)
                .ok_or_else(|| HlpError::invalid(context, "B+ tree page offset overflow"))?,
        )
        .ok_or_else(|| HlpError::invalid(context, "B+ tree page offset overflow"))?;
    let end = start
        .checked_add(header.page_size)
        .ok_or_else(|| HlpError::invalid(context, "B+ tree page end overflow"))?;
    content
        .get(start..end)
        .ok_or(HlpError::UnexpectedEof { context })
}

/// Implements the context-id hash used by HC31/HCW. Arithmetic intentionally wraps at 32 bits.
pub fn context_hash(bytes: &[u8]) -> i32 {
    if bytes.is_empty() {
        return 1;
    }
    let mut hash = 0_i32;
    for byte in bytes {
        hash = hash
            .wrapping_mul(43)
            .wrapping_add(i32::from(CONTEXT_HASH_TABLE[usize::from(*byte)]));
    }
    hash
}

// Signed-char table documented by HelpDeco. Letter entries intentionally fold ASCII case.
const CONTEXT_HASH_TABLE: [i8; 256] = [
      0, -47, -46, -45, -44, -43, -42, -41, -40, -39, -38, -37, -36, -35, -34, -33,
    -32, -31, -30, -29, -28, -27, -26, -25, -24, -23, -22, -21, -20, -19, -18, -17,
    -16,  11, -14, -13, -12, -11, -10,  -9,  -8,  -7,  -6,  -5,  -4,  -3,  12,  -1,
     10,   1,   2,   3,   4,   5,   6,   7,   8,   9,  10,  11,  12,  13,  14,  15,
     16,  17,  18,  19,  20,  21,  22,  23,  24,  25,  26,  27,  28,  29,  30,  31,
     32,  33,  34,  35,  36,  37,  38,  39,  40,  41,  42,  11,  12,  13,  14,  13,
     16,  17,  18,  19,  20,  21,  22,  23,  24,  25,  26,  27,  28,  29,  30,  31,
     32,  33,  34,  35,  36,  37,  38,  39,  40,  41,  42,  43,  44,  45,  46,  47,
     80,  81,  82,  83,  84,  85,  86,  87,  88,  89,  90,  91,  92,  93,  94,  95,
     96,  97,  98,  99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111,
    112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127,
   -128,-127,-126,-125,  11,-123,-122,-121,-120,-119,-118,-117,-116,-115,-114,-113,
   -112,-111,-110,-109,-108,-107,-106,-105,-104,-103,-102,-101,-100, -99, -98, -97,
    -96, -95, -94, -93, -92, -91, -90, -89, -88, -87, -86, -85, -84, -83, -82, -81,
    -80, -79, -78, -77, -76, -75, -74, -73, -72, -71, -70, -69, -68, -67, -66, -65,
    -64, -63, -62, -61, -60, -59, -58, -57, -56, -55, -54, -53, -52, -51, -50, -49,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_hash_is_ascii_case_insensitive() {
        assert_eq!(context_hash(b"Getting_Started"), context_hash(b"getting_started"));
        assert_ne!(context_hash(b"topic.one"), context_hash(b"topic.two"));
        assert_eq!(context_hash(b""), 1);
    }

    #[test]
    fn parses_hc30_topic_map_without_subtracting_sixteen() {
        let mut bytes = Vec::new();
        for value in [100_i32, -1, 200, 300] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let map = parse_topic_map(&bytes).expect("synthetic TOMAP should parse");
        assert_eq!(map[0], TopicPos(100));
        assert_eq!(map[3], TopicPos(300));
    }

    #[test]
    fn parses_synthetic_context_map() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&100_i32.to_le_bytes());
        bytes.extend_from_slice(&32770_i32.to_le_bytes());
        bytes.extend_from_slice(&200_i32.to_le_bytes());
        bytes.extend_from_slice(&65540_i32.to_le_bytes());
        let entries = parse_context_map(&bytes).expect("synthetic CTXOMAP should parse");
        assert_eq!(entries[0].map_id, 100);
        assert_eq!(entries[1].offset, TopicOffset(65540));
    }
}
