//! Safe, bounds-checked parsing primitives for classic Microsoft Windows `.HLP` files.
//!
//! This crate contains the complete GUI-independent WinHelp engine and uses no unsafe Rust. It understands
//! the fixed HLP container, its internal directory B+ tree, internal FILEHEADERs, the mandatory
//! `|SYSTEM` metadata stream, topic-block decompression, phrase dictionaries, and high-level
//! `|TOPIC` extraction, `|FONT`, and semantic LinkData1 formatting records.

mod btree;
mod compression;
mod container;
mod contents;
mod encoding;
mod error;
mod font;
mod formatting;
mod graphics;
mod keywords;
mod navigation;
mod phrases;
mod reader;
mod search;
mod system;
mod topic;

mod document;
mod layout;
mod macros;

pub use document::{HelpDocument, NavigationHistory, NavigationLocation, TopicOffsetAnchor, TopicPresentation, resolve_external_help_path};
pub use layout::{LayoutBox, LayoutEngine, LayoutKind, Point, Rect, RegionLayout, ResolvedFontFamily, ResolvedTextStyle, TextFlow, TextMetrics, TopicLayout};

pub use container::{DirectoryEntry, HLP_MAGIC, HlpFile, HlpHeader, InternalFile};
pub use contents::{ContentsBase, ContentsEntry, ContentsFile, ContentsLink, ContentsTarget};
pub use error::HlpError;
pub use font::{FontDescriptor, FontMetric, FontTable, HlpFontFamily, Rgb};
pub use formatting::{BorderFlags, BorderInfo, BorderStyle, EmbeddedWindowReference, FormattedRecord, FormattedTable, FormattingIssue, Hotspot, HotspotTarget, Inline, Paragraph, ParagraphAlignment, ParagraphFormat, PictureHotspot, PicturePosition, PictureReference, PictureSource, TabAlignment, TabStop, TableCell, TableCellContent, TableColumn, TableInfo, TextRun};
pub use graphics::DecodedPicture;
pub use keywords::{KeywordEntry, KeywordIndex, KeywordTable};
pub use macros::{BlockedHelpMacro, HelpMacro, HelpMacroProgram, MacroArgument, MacroBlockReason, MacroInvocation, MacroParseError, SafeHelpMacro};
pub use navigation::{context_hash, ContextEntry, ContextMapEntry, DefaultWindowEntry, NavigationMetadata, TopicIdEntry};
pub use phrases::{PhraseCompression, PhraseTable};
pub use search::{ResolvedKeyword, SearchHit, SearchMatchKind};
pub use system::{Compression, RawSystemRecord, SystemInfo, WinHelpVersion, WindowDefinition};
pub use topic::{
    Topic, TopicBlockHeader, TopicBlockInfo, TopicId, TopicMetadata, TopicOffset, TopicPos,
    TopicRecord, TopicRecordType, TopicRegion, TopicStore,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the complete container path using a one-page synthetic directory and |SYSTEM.
    #[test]
    fn opens_synthetic_hlp_and_reads_system() {
        let bytes = build_minimal_hlp();
        let hlp = HlpFile::from_bytes(bytes).expect("synthetic HLP should parse");
        assert_eq!(hlp.directory().len(), 1);
        assert_eq!(hlp.directory()[0].name, "|SYSTEM");
        let system = hlp.system_info().expect("synthetic |SYSTEM should parse");
        assert_eq!(system.version, WinHelpVersion::Windows31);
        assert_eq!(system.title.as_deref(), Some("Synthetic Help"));
    }

    /// Verifies that an internal FILEHEADER cannot advertise content beyond its reserved range.
    #[test]
    fn rejects_invalid_internal_file_bounds() {
        let mut bytes = build_minimal_hlp();
        let system_offset = find_system_offset(&bytes);
        bytes[system_offset + 4..system_offset + 8].copy_from_slice(&9999_u32.to_le_bytes());
        let error = HlpFile::from_bytes(bytes).expect_err("oversized stream must fail");
        assert!(error.to_string().contains("used size"));
    }

    /// Produces a minimal, internally consistent HLP byte vector for parser tests.
    fn build_minimal_hlp() -> Vec<u8> {
        const HEADER_SIZE: usize = 16;
        const FILE_HEADER_SIZE: usize = 9;
        const BTREE_HEADER_SIZE: usize = 38;
        const PAGE_SIZE: usize = 128;

        let directory_offset = HEADER_SIZE;
        let directory_payload_size = BTREE_HEADER_SIZE + PAGE_SIZE;
        let directory_reserved = FILE_HEADER_SIZE + directory_payload_size;
        let system_offset = directory_offset + directory_reserved;

        let mut system_payload = Vec::new();
        system_payload.extend_from_slice(&0x036C_u16.to_le_bytes());
        system_payload.extend_from_slice(&21_u16.to_le_bytes());
        system_payload.extend_from_slice(&1_u16.to_le_bytes());
        system_payload.extend_from_slice(&0_u32.to_le_bytes());
        system_payload.extend_from_slice(&0_u16.to_le_bytes());
        push_system_record(&mut system_payload, 1, b"Synthetic Help\0");
        let system_reserved = FILE_HEADER_SIZE + system_payload.len();
        let entire_size = system_offset + system_reserved;

        let mut bytes = vec![0_u8; entire_size];
        bytes[0..4].copy_from_slice(&HLP_MAGIC.to_le_bytes());
        bytes[4..8].copy_from_slice(
            &u32::try_from(directory_offset)
                .expect("test directory offset fits u32")
                .to_le_bytes(),
        );
        bytes[8..12].copy_from_slice(&(-1_i32).to_le_bytes());
        bytes[12..16].copy_from_slice(
            &u32::try_from(entire_size)
                .expect("test file size fits u32")
                .to_le_bytes(),
        );

        write_file_header(
            &mut bytes[directory_offset..],
            directory_reserved,
            directory_payload_size,
        );
        let directory = &mut bytes[directory_offset + FILE_HEADER_SIZE
            ..directory_offset + FILE_HEADER_SIZE + directory_payload_size];
        directory[0..2].copy_from_slice(&0x293B_u16.to_le_bytes());
        directory[2..4].copy_from_slice(&0_u16.to_le_bytes());
        directory[4..6].copy_from_slice(
            &u16::try_from(PAGE_SIZE)
                .expect("test page size fits u16")
                .to_le_bytes(),
        );
        directory[6..22].copy_from_slice(b"z4\0\0\0\0\0\0\0\0\0\0\0\0\0\0");
        directory[22..24].copy_from_slice(&0_i16.to_le_bytes());
        directory[24..26].copy_from_slice(&0_i16.to_le_bytes());
        directory[26..28].copy_from_slice(&0_i16.to_le_bytes());
        directory[28..30].copy_from_slice(&(-1_i16).to_le_bytes());
        directory[30..32].copy_from_slice(&1_i16.to_le_bytes());
        directory[32..34].copy_from_slice(&1_i16.to_le_bytes());
        directory[34..38].copy_from_slice(&1_u32.to_le_bytes());

        let leaf = &mut directory[BTREE_HEADER_SIZE..BTREE_HEADER_SIZE + PAGE_SIZE];
        let record_size = b"|SYSTEM\0".len() + 4;
        let unused = PAGE_SIZE - 8 - record_size;
        leaf[0..2].copy_from_slice(
            &u16::try_from(unused)
                .expect("test unused bytes fit u16")
                .to_le_bytes(),
        );
        leaf[2..4].copy_from_slice(&1_i16.to_le_bytes());
        leaf[4..6].copy_from_slice(&(-1_i16).to_le_bytes());
        leaf[6..8].copy_from_slice(&(-1_i16).to_le_bytes());
        leaf[8..16].copy_from_slice(b"|SYSTEM\0");
        leaf[16..20].copy_from_slice(
            &u32::try_from(system_offset)
                .expect("test system offset fits u32")
                .to_le_bytes(),
        );

        write_file_header(&mut bytes[system_offset..], system_reserved, system_payload.len());
        bytes[system_offset + FILE_HEADER_SIZE..system_offset + system_reserved]
            .copy_from_slice(&system_payload);
        bytes
    }

    /// Finds the synthetic `|SYSTEM` offset from its known directory leaf entry.
    fn find_system_offset(bytes: &[u8]) -> usize {
        let directory_offset = u32::from_le_bytes(bytes[4..8].try_into().expect("slice is four bytes"));
        let leaf_offset = usize::try_from(directory_offset).expect("offset fits usize") + 9 + 38;
        let offset_start = leaf_offset + 16;
        usize::try_from(u32::from_le_bytes(
            bytes[offset_start..offset_start + 4]
                .try_into()
                .expect("slice is four bytes"),
        ))
        .expect("offset fits usize")
    }

    /// Writes a synthetic nine-byte internal FILEHEADER.
    fn write_file_header(target: &mut [u8], reserved: usize, used: usize) {
        target[0..4].copy_from_slice(
            &u32::try_from(reserved)
                .expect("test reserved size fits u32")
                .to_le_bytes(),
        );
        target[4..8].copy_from_slice(
            &u32::try_from(used)
                .expect("test used size fits u32")
                .to_le_bytes(),
        );
        target[8] = 4;
    }

    /// Appends a synthetic modern `|SYSTEM` record.
    fn push_system_record(target: &mut Vec<u8>, record_type: u16, data: &[u8]) {
        target.extend_from_slice(&record_type.to_le_bytes());
        target.extend_from_slice(
            &u16::try_from(data.len())
                .expect("test record size fits u16")
                .to_le_bytes(),
        );
        target.extend_from_slice(data);
    }
}
