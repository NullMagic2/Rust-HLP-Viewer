//! Parser and discovery for classic WinHelp `.CNT` contents files and compiled `.GID` caches.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::btree::leaf_pages;
use crate::encoding::decode_windows_1252;
use crate::reader::Reader;
use crate::{HlpError, HlpFile};

/// Parsed WinHelp Contents metadata, sourced from either an authored `.CNT` or cached `.GID`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentsFile {
    pub source_path: PathBuf,
    pub title: Option<String>,
    pub base: Option<ContentsBase>,
    pub items: Vec<ContentsEntry>,
    pub index_links: Vec<ContentsLink>,
    pub search_links: Vec<ContentsLink>,
    pub warnings: Vec<String>,
}

/// The default help file/window selected by `.CNT` `:Base` or the equivalent GID record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentsBase {
    pub help_file: String,
    pub window_name: Option<String>,
}

/// One hierarchical book or topic row from `.CNT` text or compiled GID data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentsEntry {
    pub level: u16,
    pub title: String,
    pub target: Option<ContentsTarget>,
}

/// Target attached to a Contents topic row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentsTarget {
    pub context: String,
    pub help_file: Option<String>,
    pub window_name: Option<String>,
}

/// An additional help file declared by `.CNT` `:Index`/`:Link` or cached by GID `|FILES`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentsLink {
    pub label: Option<String>,
    pub help_file: String,
}

impl ContentsFile {
    /// Discovers the authored Contents source for one HLP. A readable `.CNT` remains
    /// authoritative when present; otherwise a same-basename WinHelp `.GID` cache is
    /// accepted when it contains the compiled `|CntText`/`|Flags` hierarchy. Failures
    /// remain non-fatal so an otherwise valid HLP can still be opened.
    pub(crate) fn discover(
        help_path: &Path,
        authored_name: Option<&str>,
    ) -> (Option<Self>, Option<String>) {
        let mut warnings = Vec::new();

        if let Some(path) = discover_contents_path(help_path, authored_name) {
            match fs::read(&path) {
                Ok(bytes) => return (Some(Self::parse_bytes(path, &bytes)), None),
                Err(error) => warnings.push(format!(
                    "could not read contents file '{}': {error}",
                    path.display()
                )),
            }
        }

        if let Some(path) = discover_gid_path(help_path) {
            match Self::parse_gid(path.clone()) {
                Ok(contents) => {
                    return (
                        Some(contents),
                        (!warnings.is_empty()).then(|| warnings.join("; ")),
                    );
                }
                Err(error) => warnings.push(format!(
                    "could not use cached WinHelp contents from '{}': {error}",
                    path.display()
                )),
            }
        }

        (
            None,
            (!warnings.is_empty()).then(|| warnings.join("; ")),
        )
    }

    /// Parses the Contents subset of a WinHelp `.GID` cache. GID uses the same outer
    /// container/B+tree machinery as HLP. `|CntText` stores ordered titles plus the
    /// special title/base records, `|CntJump` stores only clickable destinations, and
    /// a trailing `0x0C` record in `|Flags` carries one hierarchy byte per normal row.
    fn parse_gid(source_path: PathBuf) -> Result<Self, HlpError> {
        let file = HlpFile::open(&source_path)?;
        let text = file.internal_file("|CntText")?;
        let text_records = parse_gid_lz_records(text.content, "GID |CntText")?;

        let jump_records = match file.internal_file("|CntJump") {
            Ok(jumps) => parse_gid_lz_records(jumps.content, "GID |CntJump")?,
            Err(HlpError::MissingInternalFile(_)) => Vec::new(),
            Err(error) => return Err(error),
        };
        let jump_map: BTreeMap<u32, String> = jump_records.into_iter().collect();

        let mut result = Self {
            source_path,
            ..Self::default()
        };
        let mut normal_rows = Vec::new();
        for (key, text) in text_records {
            match key {
                70_000 => result.title = (!text.is_empty()).then_some(text),
                70_001 => {
                    if !text.is_empty() {
                        let (help_file, window_name) = split_window(&text);
                        result.base = Some(ContentsBase {
                            help_file: help_file.to_owned(),
                            window_name: window_name
                                .filter(|name| !name.is_empty())
                                .map(str::to_owned),
                        });
                    }
                }
                _ => normal_rows.push((key, text)),
            }
        }
        if normal_rows.is_empty() {
            return Err(HlpError::invalid(
                "GID Contents",
                "|CntText contains no ordinary Contents rows",
            ));
        }

        let flags = file.internal_file("|Flags")?;
        let hierarchy = parse_gid_hierarchy(flags.content, normal_rows.len())?;
        result.items.reserve(normal_rows.len());
        for ((key, title), level) in normal_rows.into_iter().zip(hierarchy) {
            if title.is_empty() {
                result
                    .warnings
                    .push(format!("GID Contents key {key} has an empty title"));
                continue;
            }
            result.items.push(ContentsEntry {
                level,
                title,
                target: jump_map.get(&key).map(|target| parse_target(target)),
            });
        }

        if let Ok(files) = file.internal_file("|FILES") {
            match parse_gid_files(files.content) {
                Ok((index_links, search_links)) => {
                    result.index_links = index_links;
                    result.search_links = search_links;
                }
                Err(error) => result
                    .warnings
                    .push(format!("GID |FILES metadata could not be decoded: {error}")),
            }
        }

        Ok(result)
    }

    /// Parses one `.CNT` byte stream. Unknown directives and malformed lines are
    /// retained as non-fatal warnings rather than aborting the whole navigation pane.
    pub fn parse_bytes(source_path: PathBuf, bytes: &[u8]) -> Self {
        let mut result = Self {
            source_path,
            ..Self::default()
        };
        let mut text = decode_windows_1252(bytes);
        if text.starts_with('\u{feff}') {
            text.remove(0);
        }

        for (line_index, raw_line) in text.lines().enumerate() {
            let line_number = line_index + 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if let Some(directive) = line.strip_prefix(':') {
                parse_directive(&mut result, line_number, directive.trim());
            } else if let Some(entry) = parse_entry(line, line_number, &mut result.warnings) {
                result.items.push(entry);
            }
        }
        result
    }
}

/// Finds the same-basename WinHelp GID cache without assuming case-sensitive host semantics.
fn discover_gid_path(help_path: &Path) -> Option<PathBuf> {
    let candidate = help_path.with_extension("gid");
    if candidate.is_file() {
        return Some(candidate);
    }
    find_case_insensitive_sibling(&candidate)
}

/// Parses a GID `Lz` B+tree whose leaf records are `u32 key` + NUL-terminated text.
fn parse_gid_lz_records(
    tree: &[u8],
    context: &'static str,
) -> Result<Vec<(u32, String)>, HlpError> {
    let mut result = Vec::new();
    for page in leaf_pages(tree)? {
        let mut page_reader = Reader::new(page, context);
        let unused = usize::from(page_reader.read_u16()?);
        let entry_count = page_reader.read_i16()?;
        let _previous = page_reader.read_i16()?;
        let _next = page_reader.read_i16()?;
        if entry_count < 0 {
            return Err(HlpError::invalid(context, "negative leaf entry count"));
        }
        if unused > page.len().saturating_sub(8) {
            return Err(HlpError::invalid(
                context,
                "unused-byte count exceeds leaf capacity",
            ));
        }
        let logical_end = page.len() - unused;
        let records = page.get(8..logical_end).ok_or(HlpError::UnexpectedEof { context })?;
        let mut reader = Reader::new(records, context);
        for _ in 0..entry_count {
            let key = reader.read_u32()?;
            let text = decode_windows_1252(reader.read_c_string()?);
            result.push((key, text));
        }
    }
    Ok(result)
}

/// Decodes the empirically verified WinHelp 4.x GID Contents tail. `WORDPAD.GID`
/// produced by Windows 95 stores byte `0x0C` followed by exactly one node byte per
/// ordinary `|CntText` row. The high nibble is the authored hierarchy level. The low
/// nibble is retained by WinHelp as node/type state; clickability here is determined
/// independently by whether the same key exists in `|CntJump`.
fn parse_gid_hierarchy(flags: &[u8], row_count: usize) -> Result<Vec<u16>, HlpError> {
    let tail_len = row_count
        .checked_add(1)
        .ok_or_else(|| HlpError::invalid("GID |Flags", "hierarchy length overflow"))?;
    if flags.len() < tail_len {
        return Err(HlpError::invalid(
            "GID |Flags",
            format!(
                "stream has {} bytes but {tail_len} trailing hierarchy bytes are required",
                flags.len()
            ),
        ));
    }
    let start = flags.len() - tail_len;
    if flags[start] != 0x0C {
        return Err(HlpError::invalid(
            "GID |Flags",
            format!(
                "expected trailing Contents tag 0x0C at offset {start}, got 0x{:02X}",
                flags[start]
            ),
        ));
    }

    flags[start + 1..]
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            let level = u16::from(byte >> 4);
            if level == 0 {
                Err(HlpError::invalid(
                    "GID |Flags",
                    format!("Contents row {index} has hierarchy level zero"),
                ))
            } else {
                Ok(level)
            }
        })
        .collect()
}

/// Decodes the GID file catalog (`L4z`: key, cached metadata dword, text). Labeled
/// rows correspond to CNT `:Index` entries; unlabeled rows correspond to `:Link`.
/// Key 10000 is the cached CNT pathname and is intentionally not exposed as a help link.
fn parse_gid_files(tree: &[u8]) -> Result<(Vec<ContentsLink>, Vec<ContentsLink>), HlpError> {
    let mut index_links = Vec::new();
    let mut search_links = Vec::new();
    for page in leaf_pages(tree)? {
        let mut page_reader = Reader::new(page, "GID |FILES leaf");
        let unused = usize::from(page_reader.read_u16()?);
        let entry_count = page_reader.read_i16()?;
        let _previous = page_reader.read_i16()?;
        let _next = page_reader.read_i16()?;
        if entry_count < 0 {
            return Err(HlpError::invalid("GID |FILES", "negative leaf entry count"));
        }
        if unused > page.len().saturating_sub(8) {
            return Err(HlpError::invalid(
                "GID |FILES",
                "unused-byte count exceeds leaf capacity",
            ));
        }
        let logical_end = page.len() - unused;
        let records = page.get(8..logical_end).ok_or(HlpError::UnexpectedEof {
            context: "GID |FILES records",
        })?;
        let mut reader = Reader::new(records, "GID |FILES records");
        for _ in 0..entry_count {
            let key = reader.read_u32()?;
            let _cached_metadata = reader.read_u32()?;
            let text = decode_windows_1252(reader.read_c_string()?);
            if key == 10_000 || text.is_empty() {
                continue;
            }
            let (label, help_file) = text
                .split_once('=')
                .map(|(label, file)| (label.trim(), file.trim()))
                .unwrap_or(("", text.trim()));
            if help_file.is_empty() {
                continue;
            }
            let link = ContentsLink {
                label: (!label.is_empty()).then(|| label.to_owned()),
                help_file: portable_gid_help_path(help_file),
            };
            if link.label.is_some() {
                index_links.push(link);
            } else {
                search_links.push(link);
            }
        }
    }
    Ok((index_links, search_links))
}

/// GID catalogs cache absolute Win9x paths. For portable viewing, keep only the final
/// Windows path component when the cache contains a drive/UNC path; relative records are
/// left untouched. Explicit Contents jump strings still retain their authored file names.
fn portable_gid_help_path(value: &str) -> String {
    let value = value.trim();
    let absolute_windows = value.starts_with("\\\\")
        || value.starts_with("//")
        || value.as_bytes().get(1).is_some_and(|byte| *byte == b':');
    if absolute_windows {
        value
            .rsplit(|character| character == '\\' || character == '/')
            .find(|component| !component.is_empty())
            .unwrap_or(value)
            .to_owned()
    } else {
        value.to_owned()
    }
}

/// Finds a matching sidecar without assuming case-sensitive host filesystem semantics.
fn discover_contents_path(help_path: &Path, authored_name: Option<&str>) -> Option<PathBuf> {
    let parent = help_path.parent().unwrap_or_else(|| Path::new("."));
    if let Some(name) = authored_name.map(str::trim).filter(|name| !name.is_empty()) {
        let normalized = name.replace('\\', std::path::MAIN_SEPARATOR_STR);
        let candidate = PathBuf::from(&normalized);
        let candidate = if candidate.is_absolute() {
            candidate
        } else {
            parent.join(candidate)
        };
        if candidate.is_file() {
            return Some(candidate);
        }
        if let Some(found) = find_case_insensitive_sibling(&candidate) {
            return Some(found);
        }
    }

    let candidate = help_path.with_extension("cnt");
    if candidate.is_file() {
        return Some(candidate);
    }
    find_case_insensitive_sibling(&candidate)
}

fn find_case_insensitive_sibling(candidate: &Path) -> Option<PathBuf> {
    let parent = candidate.parent().unwrap_or_else(|| Path::new("."));
    let name = candidate.file_name()?.to_string_lossy();
    fs::read_dir(parent).ok()?.filter_map(Result::ok).find_map(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(&name)
            .then(|| entry.path())
    })
}

fn parse_directive(result: &mut ContentsFile, line_number: usize, directive: &str) {
    let (name, value) = directive
        .split_once(char::is_whitespace)
        .map(|(name, value)| (name.trim(), value.trim()))
        .unwrap_or((directive.trim(), ""));
    match name.to_ascii_lowercase().as_str() {
        "title" => {
            if !value.is_empty() {
                result.title = Some(value.to_owned());
            }
        }
        "base" => {
            if value.is_empty() {
                result.warnings.push(format!("line {line_number}: empty :Base directive"));
            } else {
                let (help_file, window_name) = split_window(value);
                result.base = Some(ContentsBase {
                    help_file: help_file.to_owned(),
                    window_name: window_name.map(str::to_owned),
                });
            }
        }
        "index" => match parse_link(value) {
            Some(link) => result.index_links.push(link),
            None => result.warnings.push(format!("line {line_number}: empty :Index directive")),
        },
        "link" => match parse_link(value) {
            Some(link) => result.search_links.push(link),
            None => result.warnings.push(format!("line {line_number}: empty :Link directive")),
        },
        // :Include exists in some authoring toolchains, but recursively opening arbitrary
        // sidecars here would make document loading surprisingly non-local. Preserve it as a
        // diagnostic until include-cycle and trust rules are specified.
        other => result.warnings.push(format!(
            "line {line_number}: unsupported contents directive :{other}"
        )),
    }
}

fn parse_link(value: &str) -> Option<ContentsLink> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let (label, file) = value
        .split_once('=')
        .map(|(label, file)| (Some(label.trim()), file.trim()))
        .unwrap_or((None, value));
    (!file.is_empty()).then(|| ContentsLink {
        label: label.filter(|label| !label.is_empty()).map(str::to_owned),
        help_file: file.to_owned(),
    })
}

fn parse_entry(
    line: &str,
    line_number: usize,
    warnings: &mut Vec<String>,
) -> Option<ContentsEntry> {
    let digit_end = line
        .char_indices()
        .take_while(|(_, character)| character.is_ascii_digit())
        .map(|(index, character)| index + character.len_utf8())
        .last()?;
    let Ok(level) = line[..digit_end].parse::<u16>() else {
        warnings.push(format!("line {line_number}: contents level is out of range"));
        return None;
    };
    if level == 0 {
        warnings.push(format!("line {line_number}: contents level zero is invalid"));
        return None;
    }
    let body = line[digit_end..].trim_start();
    if body.is_empty() {
        warnings.push(format!("line {line_number}: contents entry has no title"));
        return None;
    }
    let (title, target) = body
        .split_once('=')
        .map(|(title, target)| (title.trim(), Some(target.trim())))
        .unwrap_or((body.trim(), None));
    if title.is_empty() {
        warnings.push(format!("line {line_number}: contents entry has an empty title"));
        return None;
    }
    let target = target.filter(|target| !target.is_empty()).map(parse_target);
    Some(ContentsEntry {
        level,
        title: title.to_owned(),
        target,
    })
}

fn parse_target(value: &str) -> ContentsTarget {
    let (before_window, window_name) = split_window(value);
    let (context, help_file) = before_window
        .split_once('@')
        .map(|(context, file)| (context.trim(), Some(file.trim())))
        .unwrap_or((before_window.trim(), None));
    ContentsTarget {
        context: context.to_owned(),
        help_file: help_file.filter(|file| !file.is_empty()).map(str::to_owned),
        window_name: window_name.filter(|name| !name.is_empty()).map(str::to_owned),
    }
}

fn split_window(value: &str) -> (&str, Option<&str>) {
    value
        .split_once('>')
        .map(|(before, window)| (before.trim(), Some(window.trim())))
        .unwrap_or((value.trim(), None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hierarchy_targets_and_directives() {
        let parsed = ContentsFile::parse_bytes(
            PathBuf::from("manual.cnt"),
            b":Title Example Help\r\n:Base manual.hlp>main\r\n:Index Glossary=terms.hlp\r\n:Link related.hlp\r\n1 Getting Started\r\n2 Welcome=welcome\r\n2 External=install@setup.hlp>steps\r\n",
        );
        assert_eq!(parsed.title.as_deref(), Some("Example Help"));
        assert_eq!(parsed.base.as_ref().map(|base| base.help_file.as_str()), Some("manual.hlp"));
        assert_eq!(parsed.items.len(), 3);
        assert!(parsed.items[0].target.is_none());
        assert_eq!(parsed.items[1].target.as_ref().map(|target| target.context.as_str()), Some("welcome"));
        assert_eq!(parsed.items[2].target.as_ref().and_then(|target| target.help_file.as_deref()), Some("setup.hlp"));
        assert_eq!(parsed.index_links.len(), 1);
        assert_eq!(parsed.search_links.len(), 1);
    }

    #[test]
    fn malformed_lines_are_warnings_not_fatal() {
        let parsed = ContentsFile::parse_bytes(
            PathBuf::from("manual.cnt"),
            b"0 Invalid\n:Unknown thing\n1 Valid=topic\n",
        );
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.warnings.len(), 2);
    }

    fn single_leaf_tree(structure: &[u8], payload: &[u8], entries: i16) -> Vec<u8> {
        const PAGE_SIZE: usize = 256;
        assert!(structure.len() <= 16);
        assert!(payload.len() <= PAGE_SIZE - 8);
        let mut tree = vec![0_u8; 38 + PAGE_SIZE];
        tree[0..2].copy_from_slice(&0x293B_u16.to_le_bytes());
        tree[2..4].copy_from_slice(&2_u16.to_le_bytes());
        tree[4..6].copy_from_slice(&(PAGE_SIZE as u16).to_le_bytes());
        tree[6..6 + structure.len()].copy_from_slice(structure);
        tree[22..24].copy_from_slice(&0_i16.to_le_bytes());
        tree[26..28].copy_from_slice(&0_i16.to_le_bytes());
        tree[28..30].copy_from_slice(&(-1_i16).to_le_bytes());
        tree[30..32].copy_from_slice(&1_i16.to_le_bytes());
        tree[32..34].copy_from_slice(&1_i16.to_le_bytes());
        tree[34..38].copy_from_slice(&(entries as u32).to_le_bytes());

        let page = &mut tree[38..];
        page[0..2].copy_from_slice(&((PAGE_SIZE - 8 - payload.len()) as u16).to_le_bytes());
        page[2..4].copy_from_slice(&entries.to_le_bytes());
        page[4..6].copy_from_slice(&(-1_i16).to_le_bytes());
        page[6..8].copy_from_slice(&(-1_i16).to_le_bytes());
        page[8..8 + payload.len()].copy_from_slice(payload);
        tree
    }

    #[test]
    fn parses_gid_lz_keyed_text_records() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&1_u32.to_le_bytes());
        payload.extend_from_slice(b"Book\0");
        payload.extend_from_slice(&2_u32.to_le_bytes());
        payload.extend_from_slice(b"Topic\0");
        payload.extend_from_slice(&70_000_u32.to_le_bytes());
        payload.extend_from_slice(b"Example Help\0");
        let tree = single_leaf_tree(b"Lz", &payload, 3);

        let records = parse_gid_lz_records(&tree, "test GID Lz").expect("GID Lz parses");
        assert_eq!(
            records,
            vec![
                (1, "Book".to_owned()),
                (2, "Topic".to_owned()),
                (70_000, "Example Help".to_owned()),
            ]
        );
    }

    #[test]
    fn parses_gid_hierarchy_high_nibbles() {
        let mut flags = vec![0_u8; 20];
        flags.extend_from_slice(&[0x0C, 0x10, 0x22, 0x32]);
        assert_eq!(
            parse_gid_hierarchy(&flags, 3).expect("hierarchy tail parses"),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn rejects_gid_without_verified_hierarchy_tail() {
        let flags = [0_u8; 16];
        let error = parse_gid_hierarchy(&flags, 3).expect_err("missing 0x0C tag must fail");
        assert!(error.to_string().contains("0x0C"));
    }

    #[test]
    fn parses_gid_files_catalog_and_portabilizes_cached_windows_paths() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&1_u32.to_le_bytes());
        payload.extend_from_slice(&0x1234_u32.to_le_bytes());
        payload.extend_from_slice(b"Glossary=C:\\WINDOWS\\HELP\\TERMS.HLP\0");
        payload.extend_from_slice(&2_u32.to_le_bytes());
        payload.extend_from_slice(&0_u32.to_le_bytes());
        payload.extend_from_slice(b"=C:\\WINDOWS\\HELP\\RELATED.HLP\0");
        payload.extend_from_slice(&10_000_u32.to_le_bytes());
        payload.extend_from_slice(&0_u32.to_le_bytes());
        payload.extend_from_slice(b"C:\\WINDOWS\\HELP\\MANUAL.CNT\0");
        let tree = single_leaf_tree(b"L4z", &payload, 3);

        let (index_links, search_links) = parse_gid_files(&tree).expect("GID |FILES parses");
        assert_eq!(index_links.len(), 1);
        assert_eq!(index_links[0].label.as_deref(), Some("Glossary"));
        assert_eq!(index_links[0].help_file, "TERMS.HLP");
        assert_eq!(search_links.len(), 1);
        assert_eq!(search_links[0].help_file, "RELATED.HLP");
        assert_eq!(portable_gid_help_path("subdir/terms.hlp"), "subdir/terms.hlp");
    }
}
