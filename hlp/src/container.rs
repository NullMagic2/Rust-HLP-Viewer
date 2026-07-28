//! Top-level HLP container parser and internal stream access.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::btree::parse_directory;
use crate::reader::Reader;
use crate::{FontTable, HlpError, NavigationMetadata, SystemInfo, TopicStore};

/// Fixed magic found at the beginning of classic Windows HLP files.
pub const HLP_MAGIC: u32 = 0x0003_5F3F;

/// Signature used by a different legacy Microsoft help/index family.
///
/// Files beginning with ASCII `LN` followed by `0x02 0x00` include MS-DOS/QBasic help
/// databases and related legacy help index files. They are not the directory/stream-based
/// Windows WinHelp container parsed by this crate, even when their filename ends in `.HLP`.
const LEGACY_LN_HELP_MAGIC: u32 = 0x0002_4E4C;

/// The 16-byte fixed HLP file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HlpHeader {
    /// Offset of the FILEHEADER containing the internal directory B+ tree.
    pub directory_start: u32,
    /// Offset of the free-list head, or -1 when no free list exists.
    pub first_free_block: i32,
    /// Logical size of the complete HLP file.
    pub entire_file_size: u32,
}

/// One filename-to-offset mapping from the internal HLP directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    /// Internal stream name such as `|SYSTEM` or `|TOPIC`.
    pub name: String,
    /// Absolute byte offset of this stream's FILEHEADER.
    pub file_offset: u32,
}

/// Borrowed view of an internal HLP file following its FILEHEADER.
#[derive(Debug, Clone, Copy)]
pub struct InternalFile<'a> {
    /// Total bytes reserved for the internal file, including the nine-byte FILEHEADER.
    pub reserved_space: u32,
    /// Number of bytes used by the internal file payload.
    pub used_space: u32,
    /// Legacy internal-file flags byte.
    pub flags: u8,
    /// Payload bytes immediately following the FILEHEADER.
    pub content: &'a [u8],
}

/// Parsed top-level Windows HLP container.
#[derive(Debug, Clone)]
pub struct HlpFile {
    bytes: Arc<[u8]>,
    header: HlpHeader,
    directory: Vec<DirectoryEntry>,
}

impl HlpFile {
    /// Loads an HLP file from disk and parses its container directory.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, HlpError> {
        let bytes = fs::read(path)?;
        Self::from_bytes(bytes)
    }

    /// Parses an HLP file from owned bytes.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self, HlpError> {
        let bytes: Arc<[u8]> = Arc::from(bytes.into());
        let header = parse_header(&bytes)?;
        let logical_size = usize::try_from(header.entire_file_size)
            .map_err(|_| HlpError::invalid("HLP header", "file size does not fit usize"))?;
        let logical_bytes = bytes
            .get(..logical_size)
            .ok_or_else(|| HlpError::invalid("HLP header", "logical file size exceeds input"))?;
        let directory_file = parse_internal_at(logical_bytes, header.directory_start)?;
        let directory = parse_directory(directory_file.content)?;

        for entry in &directory {
            let _ = parse_internal_at(logical_bytes, entry.file_offset)?;
        }

        Ok(Self {
            bytes,
            header,
            directory,
        })
    }

    /// Returns the parsed fixed HLP header.
    pub const fn header(&self) -> HlpHeader {
        self.header
    }

    /// Returns all internal directory records in their B+ tree leaf order.
    pub fn directory(&self) -> &[DirectoryEntry] {
        &self.directory
    }

    /// Finds an internal stream by ASCII-case-insensitive name.
    pub fn internal_file(&self, name: &str) -> Result<InternalFile<'_>, HlpError> {
        let entry = self
            .directory
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| HlpError::MissingInternalFile(name.to_owned()))?;
        parse_internal_at(self.logical_bytes(), entry.file_offset)
    }

    /// Parses the mandatory `|SYSTEM` stream into high-level metadata.
    pub fn system_info(&self) -> Result<SystemInfo, HlpError> {
        let system = self.internal_file("|SYSTEM")?;
        SystemInfo::parse(system.content)
    }

    /// Parses the optional `|FONT` stream, using a conservative fallback when absent.
    pub fn fonts(&self) -> Result<FontTable, HlpError> {
        FontTable::load(self)
    }

    /// Loads optional context-id, map-id, topic-name, and default-window navigation indexes.
    pub fn navigation_metadata(&self) -> Result<NavigationMetadata, HlpError> {
        NavigationMetadata::load(self)
    }

    /// Decodes the `|TOPIC` stream into topic headers, regions, and searchable text.
    pub fn topics(&self) -> Result<TopicStore, HlpError> {
        let system = self.system_info()?;
        TopicStore::parse(self, &system)
    }

    /// Returns only the logical bytes covered by the HLP header's file-size field.
    fn logical_bytes(&self) -> &[u8] {
        let end = usize::try_from(self.header.entire_file_size)
            .expect("validated HLP logical size must fit usize");
        &self.bytes[..end]
    }
}

/// Parses the fixed 16-byte file header and validates global bounds.
fn parse_header(bytes: &[u8]) -> Result<HlpHeader, HlpError> {
    let mut reader = Reader::new(bytes, "HLP header");
    let magic = reader.read_u32()?;
    if magic == LEGACY_LN_HELP_MAGIC {
        return Err(HlpError::Unsupported {
            context: "HLP container",
            detail: String::from(
                "signature 0x00024E4C (bytes `4C 4E 02 00`, commonly called `LN 02`) identifies a different legacy Microsoft help/index family (including MS-DOS/QBasic HELP.HLP files), not the classic Windows WinHelp container",
            ),
        });
    }
    if magic != HLP_MAGIC {
        return Err(HlpError::InvalidMagic {
            context: "HLP file",
            expected: HLP_MAGIC,
            actual: magic,
        });
    }

    let directory_start = reader.read_u32()?;
    let first_free_block = reader.read_i32()?;
    let entire_file_size = reader.read_u32()?;

    if entire_file_size < 16 {
        return Err(HlpError::invalid(
            "HLP header",
            format!("logical file size {entire_file_size} is below header size"),
        ));
    }
    let actual_len = u64::try_from(bytes.len())
        .map_err(|_| HlpError::invalid("HLP header", "input length does not fit u64"))?;
    if u64::from(entire_file_size) > actual_len {
        return Err(HlpError::invalid(
            "HLP header",
            format!(
                "logical file size {entire_file_size} exceeds actual size {actual_len}"
            ),
        ));
    }
    if directory_start < 16 || directory_start >= entire_file_size {
        return Err(HlpError::invalid(
            "HLP header",
            format!("directory offset {directory_start} is outside the logical file"),
        ));
    }
    if first_free_block != -1
        && (first_free_block < 16
            || u32::try_from(first_free_block).map_or(true, |offset| offset >= entire_file_size))
    {
        return Err(HlpError::invalid(
            "HLP header",
            format!("free-list offset {first_free_block} is outside the logical file"),
        ));
    }

    Ok(HlpHeader {
        directory_start,
        first_free_block,
        entire_file_size,
    })
}

/// Reads one internal FILEHEADER and returns a bounded payload view.
fn parse_internal_at(bytes: &[u8], offset: u32) -> Result<InternalFile<'_>, HlpError> {
    let offset = usize::try_from(offset)
        .map_err(|_| HlpError::invalid("internal file", "offset does not fit usize"))?;
    let data = bytes
        .get(offset..)
        .ok_or_else(|| HlpError::invalid("internal file", "offset is outside HLP data"))?;
    let mut reader = Reader::new(data, "internal FILEHEADER");
    let reserved_space = reader.read_u32()?;
    let used_space = reader.read_u32()?;
    let flags = reader.read_u8()?;

    if reserved_space < 9 {
        return Err(HlpError::invalid(
            "internal FILEHEADER",
            format!("reserved size {reserved_space} is below the 9-byte header"),
        ));
    }
    if u64::from(used_space) + 9 > u64::from(reserved_space) {
        return Err(HlpError::invalid(
            "internal FILEHEADER",
            format!("used size {used_space} does not fit reserved size {reserved_space}"),
        ));
    }

    let reserved_end = offset
        .checked_add(usize::try_from(reserved_space).map_err(|_| {
            HlpError::invalid("internal FILEHEADER", "reserved size does not fit usize")
        })?)
        .ok_or_else(|| HlpError::invalid("internal FILEHEADER", "reserved range overflow"))?;
    if reserved_end > bytes.len() {
        return Err(HlpError::invalid(
            "internal FILEHEADER",
            "reserved range extends beyond HLP file",
        ));
    }

    let content_start = offset + 9;
    let content_end = content_start
        .checked_add(usize::try_from(used_space).map_err(|_| {
            HlpError::invalid("internal FILEHEADER", "used size does not fit usize")
        })?)
        .ok_or_else(|| HlpError::invalid("internal FILEHEADER", "content range overflow"))?;
    let content = bytes
        .get(content_start..content_end)
        .ok_or(HlpError::UnexpectedEof {
            context: "internal file payload",
        })?;

    Ok(InternalFile {
        reserved_space,
        used_space,
        flags,
        content,
    })
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_ln_help_signature_is_reported_as_a_different_help_family() {
        let error = HlpFile::from_bytes(LEGACY_LN_HELP_MAGIC.to_le_bytes().to_vec())
            .expect_err("LN02 help files must not be parsed as Windows WinHelp");
        match error {
            HlpError::Unsupported { context, detail } => {
                assert_eq!(context, "HLP container");
                assert!(detail.contains("MS-DOS/QBasic"));
                assert!(detail.contains("not the classic Windows WinHelp container"));
            }
            other => panic!("expected a classified unsupported-container error, got {other}"),
        }
    }

    #[test]
    fn unrelated_bad_magic_remains_an_invalid_magic_error() {
        let actual: u32 = 0x1234_5678;
        let error = HlpFile::from_bytes(actual.to_le_bytes().to_vec())
            .expect_err("unknown signatures must still be rejected as invalid WinHelp");
        match error {
            HlpError::InvalidMagic { expected, actual: found, .. } => {
                assert_eq!(expected, HLP_MAGIC);
                assert_eq!(found, actual);
            }
            other => panic!("expected InvalidMagic, got {other}"),
        }
    }
}
