//! Variable-length LinkData1 display formatting decoder.
//!
//! WinHelp stores paragraph formatting and character commands separately from the visible strings
//! in LinkData2. This module synchronizes both streams into a retained, GUI-independent model.

use crate::encoding::{decode_windows_1252, decode_windows_charset};
use crate::graphics::DecodedPicture;
use crate::reader::Reader;
use crate::{FontTable, HlpError, TopicOffset, TopicRecord, TopicRecordType};

const MAX_COLUMNS: usize = 32;
const MAX_TABS: usize = 1024;
const MAX_PARAGRAPHS_PER_RECORD: usize = 65_536;
const MAX_COMPACT_OBJECT_BYTES: usize = 32 * 1024 * 1024;
/// Defensive recursion bound for nested table records. The Microsoft renderer recursively
/// re-enters its generic record dispatcher for table cells; real help files are shallow, while
/// a malicious file could otherwise exhaust the Rust call stack.
const MAX_TABLE_NESTING_DEPTH: usize = 64;

/// Horizontal paragraph alignment inferred from paragraph flag bits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ParagraphAlignment {
    #[default]
    Left,
    Right,
    Center,
}

/// Which sides of a paragraph border are present.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BorderFlags {
    pub box_all: bool,
    pub top: bool,
    pub left: bool,
    pub bottom: bool,
    pub right: bool,
}

/// The three-bit WinHelp paragraph-border style code.
///
/// KB917607 WinHlp32 extracts `(flags >> 5) & 7` as one value (0x415386..0x415392);
/// it does not interpret the three bits independently.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BorderStyle {
    #[default]
    Normal,
    Thick,
    Double,
    Shadow,
    /// Style code 4 follows the normal border geometry in the verified renderer.
    ReferenceStyle4,
    Reserved(u8),
}

impl BorderStyle {
    const fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Normal,
            1 => Self::Thick,
            2 => Self::Double,
            3 => Self::Shadow,
            4 => Self::ReferenceStyle4,
            other => Self::Reserved(other),
        }
    }

    /// Returns the raw three-bit style code.
    pub const fn code(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Thick => 1,
            Self::Double => 2,
            Self::Shadow => 3,
            Self::ReferenceStyle4 => 4,
            Self::Reserved(code) => code & 7,
        }
    }
}

/// Paragraph border information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorderInfo {
    pub flags: BorderFlags,
    pub style: BorderStyle,
    /// The remaining two bytes of WinHelp's three-byte border record. The verified drawing
    /// path does not treat these bytes as a pen width, so retain them losslessly until their
    /// mode-dependent semantics are established.
    pub raw_extra: [u8; 2],
}

/// Alignment of a custom tab stop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TabAlignment {
    #[default]
    Left,
    Right,
    Center,
    Unknown(u16),
}

/// One custom paragraph tab stop in raw WinHelp metric units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabStop {
    pub position: u16,
    pub alignment: TabAlignment,
}

/// Decoded variable-length paragraph information.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParagraphFormat {
    /// Table column, when the paragraph came from a type-0x23 record.
    pub column: Option<i16>,
    pub id: u16,
    pub unknown_value: Option<i32>,
    pub spacing_above: Option<i16>,
    pub spacing_below: Option<i16>,
    pub spacing_lines: Option<i16>,
    pub left_indent: Option<i16>,
    pub right_indent: Option<i16>,
    pub first_line_indent: Option<i16>,
    /// Optional authored default-tab interval. Microsoft WinHlp32 uses 72 format units when absent.
    pub default_tab_interval: Option<i16>,
    pub alignment: ParagraphAlignment,
    /// Paragraph flag bit 12. The reference WinHlp32 suppresses automatic word/tab wrapping.
    pub no_wrap: bool,
    /// Paragraph flag bit 13. The Microsoft layout path uses it for right-to-left paragraphs
    /// and enters its run-reordering logic for Hebrew/Arabic charsets.
    pub right_to_left: bool,
    pub border: Option<BorderInfo>,
    pub tabs: Vec<TabStop>,
}

/// WinHelp table-column sizing metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableColumn {
    /// Authored column width. KB917607 WinHlp32 reads this unsigned word first.
    pub width: u16,
    /// Authored gap before this column. KB917607 WinHlp32 reads this unsigned word second.
    pub gap_before: u16,
}

/// Header that precedes the recursive cell sequence in a table display record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableInfo {
    pub table_type: u8,
    /// Present only for table type 0. This is an unsigned source metric.
    pub minimum_width: Option<u16>,
    pub columns: Vec<TableColumn>,
}

/// One retained cell from a WinHelp table record.
///
/// KB917607 WinHlp32 stores the signed column number outside the nested compact TOPICLINK
/// record. The nested record is then dispatched recursively, so a cell can itself contain a
/// complete table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCell {
    pub column: i16,
    pub content: TableCellContent,
}

/// Retained content of a table cell.
///
/// Display records reference a range in the owning [`FormattedRecord::paragraphs`] vector.
/// Keeping paragraph storage flat avoids duplicating mutable picture/hotspot state while the
/// table tree retains the exact recursive geometry needed by the renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableCellContent {
    Display {
        paragraph_start: usize,
        paragraph_end: usize,
    },
    /// A compact 0x03/0x22 graphics record dispatched by WinHlp32's graphics renderer.
    Picture(PictureReference),
    Table(Box<FormattedTable>),
    /// A compact 0x05/0x24 hosted/custom-window record. The Microsoft renderer creates a native
    /// child window from the descriptor string; this viewer retains it but never executes it.
    EmbeddedWindow(EmbeddedWindowReference),
    /// Compact type 0x06 reaches no renderer in the verified dispatcher. Retaining it explicitly
    /// distinguishes an intentional no-paint record from an unknown/unsupported record.
    NoRender {
        record_type: TopicRecordType,
        payload_size: usize,
    },
    /// A compact record family whose payload is bounded correctly but whose painter is not yet
    /// implemented. This remains in the tree so diagnostics do not lose record identity.
    Unsupported {
        record_type: TopicRecordType,
        payload_size: usize,
    },
}

/// Retained metadata for WinHelp compact hosted/custom-window records (0x05/0x24).
///
/// KB917607 WinHlp32 `0x419281` skips six payload bytes, copies the following NUL-terminated
/// descriptor, then passes it to `0x4240F4`, which may create a native child window. The six-byte
/// prefix is retained losslessly because the traced renderer does not interpret it directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedWindowReference {
    pub record_type: TopicRecordType,
    pub raw_prefix: [u8; 6],
    pub descriptor: String,
    pub payload_size: usize,
}

impl EmbeddedWindowReference {
    /// Returns the label and macro payload for WinHlp32's built-in `!label,macro` button form.
    ///
    /// KB917607 factory `0x4240F4` treats a leading `!` as its standard-button mode and creates
    /// the child with the built-in `BUTTON` window class. The macro text is retained only as
    /// metadata here; the formatting layer never executes it.
    pub fn standard_button_parts(&self) -> Option<(&str, &str)> {
        let descriptor = self.descriptor.strip_prefix('!')?;
        let (label, macro_text) = descriptor.split_once(',')?;
        Some((label, macro_text))
    }
}

/// A recursively retained WinHelp table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedTable {
    pub info: TableInfo,
    pub cells: Vec<TableCell>,
}

/// Target carried by a clickable WinHelp hotspot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotspotTarget {
    Internal {
        offset: TopicOffset,
        popup: bool,
    },
    ContextHash {
        hash: i32,
        popup: bool,
    },
    External {
        opcode: u8,
        type_code: u8,
        offset: TopicOffset,
        window_number: Option<u8>,
        help_file: Option<String>,
        window_name: Option<String>,
    },
    Macro(String),
}

/// Active hotspot state copied onto text runs while LinkData1 and LinkData2 are synchronized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotspot {
    pub target: HotspotTarget,
    /// True when WinHelp's command asks the viewer to apply the usual hyperlink emphasis.
    pub emphasized: bool,
}

/// Text plus its currently selected font and optional hotspot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRun {
    pub text: String,
    pub font_index: u16,
    pub hotspot: Option<Hotspot>,
}

/// Non-text command that must occupy space or be retained by the semantic model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(TextRun),
    LineBreak,
    Tab,
    /// WinHlp32 character command 0x85 followed by a signed 16-bit marker.
    ///
    /// The verified KB917607 scanner consumes exactly three bytes and tokenizer `0x417816`
    /// writes the sign-extended WORD to transient render-state `+0x38` without emitting a glyph.
    /// Paragraph setup and line finalization prove that field is the current horizontal line
    /// origin used by alignment, so the retained layout applies the marker as a device-coordinate
    /// x-origin reset while keeping the command itself zero-width.
    Control85(i16),
    Picture(PictureReference),
    /// Safely retained WinHelp 0x05/0x24 hosted/custom-window object.
    ///
    /// The reference executable creates a native child control and then queries its size. The
    /// cross-platform viewer never executes authored controls; layout uses a bounded placeholder.
    EmbeddedWindow(EmbeddedWindowReference),
}

/// Layout role encoded by the three classic WinHelp picture commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PicturePosition {
    /// `bmc`: picture participates in the ordinary character flow.
    Inline,
    /// `bml`: picture floats at the left edge while subsequent text wraps on its right.
    FloatLeft,
    /// `bmr`: picture floats at the right edge while subsequent text wraps on its left.
    FloatRight,
}

/// Resolved clickable rectangle attached to a decoded WinHelp picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureHotspot {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub hotspot: Hotspot,
}

/// Where a topic picture command obtains its logical WinHelp graphics object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PictureSource {
    /// Index of an internal `|bmN` stream.
    Indexed(u16),
    /// Graphics object stored directly in the topic command.
    Embedded(Vec<u8>),
    /// Payload that is synchronized but is not a normal raster-picture reference.
    Unsupported(Vec<u8>),
}

/// Picture command plus its resolved raster image when decoding succeeded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureReference {
    pub command: u8,
    pub position: PicturePosition,
    pub picture_type: u8,
    pub encoded_size: usize,
    pub hotspot_count: Option<u16>,
    pub source: PictureSource,
    pub image: Option<DecodedPicture>,
    pub hotspots: Vec<PictureHotspot>,
    pub decode_warning: Option<String>,
}

/// One paragraph after synchronizing LinkData1 formatting and LinkData2 strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paragraph {
    pub format: ParagraphFormat,
    pub inlines: Vec<Inline>,
}

/// A non-fatal formatting problem. `layout_safe` distinguishes exact bounded omissions from
/// stream-ambiguous failures that require the caller to fall back to plain text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattingIssue {
    pub link_data1_offset: usize,
    pub message: String,
    /// True when the parser has consumed an exact, independently bounded structure and the
    /// retained layout before/after it remains trustworthy despite missing semantics.
    pub layout_safe: bool,
}

/// Semantic representation of one visible display/table/special compact TOPICLINK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedRecord {
    pub topic_size: i32,
    pub topic_length: Option<u16>,
    pub table: Option<TableInfo>,
    /// Recursive table-cell tree. Empty for ordinary display records.
    ///
    /// Paragraph text remains stored once in `paragraphs`; display cells point into that vector
    /// with ranges while nested tables retain their own geometry/cell hierarchy here.
    pub table_cells: Vec<TableCell>,
    pub paragraphs: Vec<Paragraph>,
    pub issues: Vec<FormattingIssue>,
}

impl FormattedRecord {
    /// Decodes a display or table record. Empty LinkData1 falls back to one plain paragraph.
    ///
    /// Starts from descriptor 0, which is only correct for the first record of a topic. Callers
    /// that walk every record of one topic should use [`Self::decode_with_font`] so WinHelp's
    /// running font selection survives record boundaries.
    pub fn decode(record: &TopicRecord) -> Result<Self, HlpError> {
        let mut font_index = 0_u16;
        Self::decode_with_font(record, &mut font_index)
    }

    /// Decodes one record while carrying WinHelp's running font selection in `font_index`.
    ///
    /// WinHlp32 keeps the selected font in a single global (`0x43C2C4` in the KB917607 build).
    /// It is initialised once per topic render at `0x41B05D` and afterwards changed only by
    /// character opcode `0x80` at `0x41AB8C`. Neither paragraph terminator touches it: `0xFF`
    /// at `0x41ABEB` and the `0x81`/`0x82`/`0x83` path at `0x41AB7A` only raise the
    /// `0x43C2D8` progress flag. A paragraph that reuses the previous paragraph's font
    /// therefore emits no `0x80` at all, so resetting the selection per paragraph silently
    /// substitutes descriptor 0 - the bold heading face in most authored files.
    pub fn decode_with_font(
        record: &TopicRecord,
        font_index: &mut u16,
    ) -> Result<Self, HlpError> {
        Self::decode_with_font_context(record, font_index, None)
    }

    /// Decodes one record using the document's verified per-face GDI charset table.
    pub(crate) fn decode_with_font_table(
        record: &TopicRecord,
        font_index: &mut u16,
        fonts: &FontTable,
    ) -> Result<Self, HlpError> {
        Self::decode_with_font_context(record, font_index, Some(fonts))
    }

    fn decode_with_font_context(
        record: &TopicRecord,
        font_index: &mut u16,
        fonts: Option<&FontTable>,
    ) -> Result<Self, HlpError> {
        if matches!(
            record.record_type,
            TopicRecordType::Graphic30
                | TopicRecordType::Graphic
                | TopicRecordType::EmbeddedWindow30
                | TopicRecordType::EmbeddedWindow
                | TopicRecordType::NoRender30
        ) {
            return Self::decode_compact_special(record);
        }
        if !matches!(
            record.record_type,
            TopicRecordType::Display30
                | TopicRecordType::Table30
                | TopicRecordType::Display
                | TopicRecordType::Table
        ) {
            return Err(HlpError::invalid(
                "LinkData1",
                format!("record {:?} is not displayable", record.record_type),
            ));
        }

        if record.link_data1.is_empty() {
            return Ok(Self::plain_fallback_with_font(record, *font_index, fonts));
        }

        let mut ld1 = Reader::new(&record.link_data1, "display LinkData1");
        let mut strings = StringStream::with_fonts(&record.link_data2, fonts);
        let topic_size = ld1.read_compressed_signed_long()?;
        let topic_length = if is_modern_record(record.record_type) {
            Some(ld1.read_compressed_unsigned_short()?)
        } else {
            None
        };

        let mut paragraphs = Vec::new();
        let mut issues = Vec::new();
        let (table, table_cells) = if is_table_record(record.record_type) {
            let table = parse_table_header(&mut ld1)?;
            let cells = decode_table_cells(
                &mut ld1,
                &mut strings,
                &table,
                font_index,
                &mut paragraphs,
                &mut issues,
                0,
                0,
            )?;
            (Some(table), cells)
        } else {
            decode_display_payload(
                &mut ld1,
                &mut strings,
                None,
                font_index,
                &mut paragraphs,
                &mut issues,
            )?;
            (None, Vec::new())
        };

        if paragraphs.is_empty() && !record.link_data2.is_empty() {
            let fallback = Self::plain_fallback_with_font(record, *font_index, fonts);
            paragraphs = fallback.paragraphs;
        }

        Ok(Self {
            topic_size,
            topic_length,
            table,
            table_cells,
            paragraphs,
            issues,
        })
    }

    /// Decodes a top-level compact graphics/hosted-window record using the same payload
    /// header that WinHlp32's generic dispatcher applies inside tables.
    fn decode_compact_special(record: &TopicRecord) -> Result<Self, HlpError> {
        if record.link_data1.is_empty() {
            return Ok(Self::plain_fallback(record));
        }
        let mut ld1 = Reader::new(&record.link_data1, "compact special LinkData1");
        let topic_size = ld1.read_compressed_signed_long()?;
        if topic_size < 0 {
            return Err(HlpError::invalid(
                "compact special LinkData1",
                format!("negative compact payload size {topic_size}"),
            ));
        }
        let topic_length = if matches!(
            record.record_type,
            TopicRecordType::Graphic | TopicRecordType::EmbeddedWindow
        ) {
            Some(ld1.read_compressed_unsigned_short()?)
        } else {
            None
        };
        let payload_size = usize::try_from(topic_size)
            .map_err(|_| HlpError::invalid("compact special LinkData1", "payload size does not fit usize"))?;
        let payload = ld1.read_bytes(payload_size)?;
        let mut issues = Vec::new();
        let inlines = match record.record_type {
            TopicRecordType::Graphic30 => vec![Inline::Picture(parse_compact_picture(0x03, payload.to_vec()))],
            TopicRecordType::Graphic => vec![Inline::Picture(parse_compact_picture(0x22, payload.to_vec()))],
            TopicRecordType::EmbeddedWindow30 | TopicRecordType::EmbeddedWindow => {
                match parse_embedded_window(record.record_type, payload) {
                    Ok(window) => {
                        issues.push(FormattingIssue {
                            link_data1_offset: 0,
                            layout_safe: true,
                            message: format!(
                                "top-level hosted-window record {:?} retained safely; native authored control execution is disabled",
                                record.record_type
                            ),
                        });
                        vec![Inline::EmbeddedWindow(window)]
                    }
                    Err(message) => {
                        issues.push(FormattingIssue {
                            link_data1_offset: 0,
                            layout_safe: false,
                            message,
                        });
                        Vec::new()
                    }
                }
            }
            TopicRecordType::NoRender30 => Vec::new(),
            _ => unreachable!("compact-special decoder called for ordinary display record"),
        };
        if ld1.remaining() != 0 {
            issues.push(FormattingIssue {
                link_data1_offset: ld1.position(),
                layout_safe: false,
                message: format!(
                    "compact special record left {} LinkData1 byte(s) outside its declared payload",
                    ld1.remaining()
                ),
            });
        }
        Ok(Self {
            topic_size,
            topic_length,
            table: None,
            table_cells: Vec::new(),
            paragraphs: if inlines.is_empty() {
                Vec::new()
            } else {
                vec![Paragraph {
                    format: ParagraphFormat::default(),
                    inlines,
                }]
            },
            issues,
        })
    }

    /// Creates a single default paragraph from already reconstructed plain text.
    pub fn from_plain_text(text: &str) -> Self {
        Self {
            topic_size: 0,
            topic_length: None,
            table: None,
            table_cells: Vec::new(),
            paragraphs: vec![Paragraph {
                format: ParagraphFormat::default(),
                inlines: if text.is_empty() {
                    Vec::new()
                } else {
                    vec![Inline::Text(TextRun {
                        text: text.to_owned(),
                        font_index: 0,
                        hotspot: None,
                    })]
                },
            }],
            issues: Vec::new(),
        }
    }

    fn plain_fallback(record: &TopicRecord) -> Self {
        Self::plain_fallback_with_font(record, 0, None)
    }

    fn plain_fallback_with_font(
        record: &TopicRecord,
        font_index: u16,
        fonts: Option<&FontTable>,
    ) -> Self {
        let mut stream = StringStream::with_fonts(&record.link_data2, fonts);
        let mut inlines = Vec::new();
        while let Ok(Some(text)) = stream.next_string_for_font(font_index) {
            if !text.is_empty() {
                inlines.push(Inline::Text(TextRun {
                    text,
                    font_index,
                    hotspot: None,
                }));
            }
        }
        Self {
            topic_size: 0,
            topic_length: None,
            table: None,
            table_cells: Vec::new(),
            paragraphs: vec![Paragraph {
                format: ParagraphFormat::default(),
                inlines,
            }],
            issues: Vec::new(),
        }
    }
}






fn is_modern_record(record_type: TopicRecordType) -> bool {
    matches!(record_type, TopicRecordType::Display | TopicRecordType::Table)
}

fn is_table_record(record_type: TopicRecordType) -> bool {
    matches!(record_type, TopicRecordType::Table30 | TopicRecordType::Table)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompactRecordHeader {
    record_type: TopicRecordType,
    payload_size: usize,
    topic_length: Option<u16>,
}

/// Reads one compact TOPICLINK header used by tables and inline object commands.
///
/// The verified KB917607 helper `0x412884` parses the same bounded record envelope for table
/// cells and for the nested objects reached from character commands `0x86`/`0x87`/`0x88`.
/// Ordinary compact records consume a record-type byte, a compressed signed payload size and,
/// for modern types above `0x10`, a compressed unsigned topic length. Topic-header generations
/// `0x02`/`0x21` use the helper's fixed-width DWORD payload-size form instead (`0x21` also has a
/// WORD TopicLength). Callers then advance by exactly the decoded payload size.
fn read_compact_record_header(reader: &mut Reader<'_>) -> Result<CompactRecordHeader, HlpError> {
    let raw_type = reader.read_u8()?;
    if !(matches!(raw_type, 0x01..=0x06 | 0x20..=0x24)) {
        return Err(HlpError::invalid(
            "compact record",
            format!("invalid compact record type 0x{raw_type:02X}"),
        ));
    }
    let record_type = TopicRecordType::from_byte(raw_type);

    // KB917607 helper 0x412884 has a separate fixed-width header for topic-header
    // generations 0x02/0x21. All display/table/graphics generations use the compressed
    // signed payload size handled by 0x4129E8.
    let (payload_size, topic_length) = if matches!(raw_type, 0x02 | 0x21) {
        let size = reader.read_u32()?;
        let topic_length = if raw_type == 0x21 {
            Some(reader.read_u16()?)
        } else {
            None
        };
        (
            usize::try_from(size)
                .map_err(|_| HlpError::invalid("compact record", "fixed payload size does not fit usize"))?,
            topic_length,
        )
    } else {
        let size = reader.read_compressed_signed_long()?;
        if size < 0 {
            return Err(HlpError::invalid(
                "compact record",
                format!("negative compact payload size {size}"),
            ));
        }
        let topic_length = if raw_type > 0x10 {
            Some(reader.read_compressed_unsigned_short()?)
        } else {
            None
        };
        (
            usize::try_from(size)
                .map_err(|_| HlpError::invalid("compact record", "payload size does not fit usize"))?,
            topic_length,
        )
    };

    Ok(CompactRecordHeader {
        record_type,
        payload_size,
        topic_length,
    })
}

/// Decodes the paragraph payload of one ordinary display record or one table cell.
fn decode_display_payload(
    ld1: &mut Reader<'_>,
    strings: &mut StringStream<'_>,
    column: Option<i16>,
    font_index: &mut u16,
    paragraphs: &mut Vec<Paragraph>,
    issues: &mut Vec<FormattingIssue>,
) -> Result<(), HlpError> {
    let mut same_format: Option<ParagraphFormat> = None;
    loop {
        if paragraphs.len() >= MAX_PARAGRAPHS_PER_RECORD {
            return Err(HlpError::invalid(
                "display LinkData1",
                format!("paragraph count exceeds {MAX_PARAGRAPHS_PER_RECORD}"),
            ));
        }
        if ld1.remaining() == 0 {
            break;
        }

        let mut format = if let Some(format) = same_format.take() {
            format
        } else {
            parse_paragraph_info(ld1)?
        };
        format.column = column;
        let (paragraph, ending) =
            parse_character_stream(ld1, strings, format, font_index, issues)?;
        let repeat = paragraph.format.clone();
        paragraphs.push(paragraph);
        match ending {
            CharacterEnding::SameParagraphFormat => same_format = Some(repeat),
            CharacterEnding::NextParagraphInfo | CharacterEnding::Stop => break,
        }
    }
    Ok(())
}

/// Decodes WinHlp32's table-cell sequence after the table geometry header.
///
/// `0x414F66` proves that a table is not followed directly by ParagraphInfo records. Instead
/// each cell is framed as `i16 column` + a complete compact TOPICLINK record. `0x417578`
/// dispatches that nested record, including recursively re-entering `0x414F66` for `0x04`/`0x23`
/// tables. The caller advances to the next cell by the compact header length plus its decoded
/// payload size. The cell list ends with column `-1`.
fn decode_table_cells(
    ld1: &mut Reader<'_>,
    strings: &mut StringStream<'_>,
    table: &TableInfo,
    font_index: &mut u16,
    paragraphs: &mut Vec<Paragraph>,
    issues: &mut Vec<FormattingIssue>,
    depth: usize,
    base_offset: usize,
) -> Result<Vec<TableCell>, HlpError> {
    let mut cells = Vec::new();
    loop {
        if ld1.remaining() < 2 {
            return Err(HlpError::invalid(
                "table LinkData1",
                "table cell list is missing its -1 column terminator",
            ));
        }
        let cell_offset = ld1.position();
        let cell_offset_absolute = base_offset.saturating_add(cell_offset);
        let column = ld1.read_i16()?;
        if column == -1 {
            break;
        }
        if column < 0 {
            return Err(HlpError::invalid(
                "table LinkData1",
                format!(
                    "negative table column {column} at LinkData1+0x{cell_offset_absolute:X}"
                ),
            ));
        }
        let column_index = usize::try_from(column)
            .map_err(|_| HlpError::invalid("table LinkData1", "column does not fit usize"))?;
        if column_index >= table.columns.len() {
            return Err(HlpError::invalid(
                "table LinkData1",
                format!(
                    "table column {column_index} exceeds declared column count {}",
                    table.columns.len()
                ),
            ));
        }

        let compact_offset = ld1.position();
        let compact_offset_absolute = base_offset.saturating_add(compact_offset);
        let header = read_compact_record_header(ld1)?;
        let payload_offset = ld1.position();
        let payload_offset_absolute = base_offset.saturating_add(payload_offset);
        let payload = ld1.read_bytes(header.payload_size)?;
        let mut cell = Reader::new(payload, "table cell LinkData1");

        let content = match header.record_type {
            TopicRecordType::Display30 | TopicRecordType::Display => {
                let paragraph_start = paragraphs.len();
                let issue_start = issues.len();
                decode_display_payload(
                    &mut cell,
                    strings,
                    Some(column),
                    font_index,
                    paragraphs,
                    issues,
                )?;
                for issue in &mut issues[issue_start..] {
                    issue.link_data1_offset = issue
                        .link_data1_offset
                        .saturating_add(payload_offset_absolute);
                }
                TableCellContent::Display {
                    paragraph_start,
                    paragraph_end: paragraphs.len(),
                }
            }
            TopicRecordType::Graphic30 | TopicRecordType::Graphic => {
                let picture_type = match header.record_type {
                    TopicRecordType::Graphic30 => 0x03,
                    TopicRecordType::Graphic => 0x22,
                    _ => unreachable!(),
                };
                TableCellContent::Picture(parse_compact_picture(
                    picture_type,
                    payload.to_vec(),
                ))
            }
            TopicRecordType::Table30 | TopicRecordType::Table => {
                if depth >= MAX_TABLE_NESTING_DEPTH {
                    issues.push(FormattingIssue {
                        link_data1_offset: compact_offset_absolute,
                        layout_safe: false,
                        message: format!(
                            "nested table depth exceeds safety limit {MAX_TABLE_NESTING_DEPTH}; bounded payload skipped"
                        ),
                    });
                    TableCellContent::Unsupported {
                        record_type: header.record_type,
                        payload_size: header.payload_size,
                    }
                } else {
                    let nested_info = parse_table_header(&mut cell)?;
                    let nested_cells = decode_table_cells(
                        &mut cell,
                        strings,
                        &nested_info,
                        font_index,
                        paragraphs,
                        issues,
                        depth + 1,
                        payload_offset_absolute,
                    )?;
                    TableCellContent::Table(Box::new(FormattedTable {
                        info: nested_info,
                        cells: nested_cells,
                    }))
                }
            }
            TopicRecordType::EmbeddedWindow30 | TopicRecordType::EmbeddedWindow => {
                match parse_embedded_window(header.record_type, payload) {
                    Ok(window) => {
                        issues.push(FormattingIssue {
                            link_data1_offset: compact_offset_absolute,
                            layout_safe: true,
                            message: format!(
                                "compact hosted-window record {:?} retained safely; native authored control execution is disabled",
                                header.record_type
                            ),
                        });
                        TableCellContent::EmbeddedWindow(window)
                    }
                    Err(message) => {
                        issues.push(FormattingIssue {
                            link_data1_offset: compact_offset_absolute,
                            layout_safe: false,
                            message,
                        });
                        TableCellContent::Unsupported {
                            record_type: header.record_type,
                            payload_size: header.payload_size,
                        }
                    }
                }
            }
            TopicRecordType::NoRender30 => TableCellContent::NoRender {
                record_type: header.record_type,
                payload_size: header.payload_size,
            },
            other => {
                issues.push(FormattingIssue {
                    link_data1_offset: compact_offset_absolute,
                    layout_safe: false,
                    message: format!(
                        "unsupported compact table-cell record {other:?}; skipped {} bounded payload bytes",
                        header.payload_size
                    ),
                });
                TableCellContent::Unsupported {
                    record_type: other,
                    payload_size: header.payload_size,
                }
            }
        };

        if cell.remaining() != 0
            && matches!(
                header.record_type,
                TopicRecordType::Display30
                    | TopicRecordType::Display
                    | TopicRecordType::Table30
                    | TopicRecordType::Table
            )
            && !matches!(&content, TableCellContent::Unsupported { .. })
        {
            issues.push(FormattingIssue {
                link_data1_offset: payload_offset_absolute.saturating_add(cell.position()),
                layout_safe: false,
                message: format!(
                    "table cell left {} bounded LinkData1 byte(s) undecoded",
                    cell.remaining()
                ),
            });
        }

        cells.push(TableCell { column, content });
    }
    Ok(cells)
}

fn parse_compact_picture(picture_type: u8, payload: Vec<u8>) -> PictureReference {
    let encoded_size = payload.len();
    PictureReference {
        // Compact graphics are full TOPICLINK records rather than 0x86/0x87/0x88 character
        // commands. Zero keeps that distinction explicit while sharing the resolved picture model.
        command: 0,
        position: PicturePosition::Inline,
        picture_type,
        encoded_size,
        hotspot_count: None,
        source: parse_picture_source(picture_type, payload),
        image: None,
        hotspots: Vec::new(),
        decode_warning: None,
    }
}

fn parse_embedded_window(
    record_type: TopicRecordType,
    payload: &[u8],
) -> Result<EmbeddedWindowReference, String> {
    let Some(prefix) = payload.get(..6) else {
        return Err(format!(
            "compact hosted-window record {record_type:?} is shorter than the six-byte prefix traced in WinHlp32"
        ));
    };
    let mut raw_prefix = [0_u8; 6];
    raw_prefix.copy_from_slice(prefix);
    let descriptor_bytes = &payload[6..];
    let descriptor_end = descriptor_bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(descriptor_bytes.len());
    let descriptor = decode_windows_1252(&descriptor_bytes[..descriptor_end]);
    Ok(EmbeddedWindowReference {
        record_type,
        raw_prefix,
        descriptor,
        payload_size: payload.len(),
    })
}

fn parse_table_header(reader: &mut Reader<'_>) -> Result<TableInfo, HlpError> {
    let count = usize::from(reader.read_u8()?);
    if count > MAX_COLUMNS {
        return Err(HlpError::invalid("table LinkData1", format!("column count {count} exceeds {MAX_COLUMNS}")));
    }
    let table_type = reader.read_u8()?;
    // KB917607 WinHlp32 0x414f66: only table type 0 carries this extra word.
    let minimum_width = if table_type == 0 {
        Some(reader.read_u16()?)
    } else {
        None
    };
    let mut columns = Vec::with_capacity(count);
    for _ in 0..count {
        // The reference reads the pair as width first, then gap-before-column, both unsigned.
        columns.push(TableColumn {
            width: reader.read_u16()?,
            gap_before: reader.read_u16()?,
        });
    }
    Ok(TableInfo {
        table_type,
        minimum_width,
        columns,
    })
}

fn parse_paragraph_info(reader: &mut Reader<'_>) -> Result<ParagraphFormat, HlpError> {
    // KB917607 WinHlp32 0x4125d9 begins every paragraph payload with the same
    // two/four-byte compressed signed-long decoder used elsewhere. Treating it as two
    // fixed bytes works only for the short encoding and desynchronizes long-form records.
    let _paragraph_leading_value = reader.read_compressed_signed_long()?;
    let id = reader.read_u16()?;
    let bits = reader.read_u16()?;

    let unknown_value = conditional(bits, 0, || reader.read_compressed_signed_long())?;
    let spacing_above = conditional(bits, 1, || reader.read_compressed_signed_short())?;
    let spacing_below = conditional(bits, 2, || reader.read_compressed_signed_short())?;
    let spacing_lines = conditional(bits, 3, || reader.read_compressed_signed_short())?;
    let left_indent = conditional(bits, 4, || reader.read_compressed_signed_short())?;
    let right_indent = conditional(bits, 5, || reader.read_compressed_signed_short())?;
    let first_line_indent = conditional(bits, 6, || reader.read_compressed_signed_short())?;
    // Verified against KB917607 WinHlp32 at 0x41278c: bit 7 carries a compressed
    // signed default-tab interval. Absence means 72 source units. It must be consumed
    // here or every later paragraph field becomes desynchronized.
    let default_tab_interval = conditional(bits, 7, || reader.read_compressed_signed_short())?;

    let border = if bits & (1 << 8) != 0 {
        let raw = reader.read_u8()?;
        let extra = reader.read_bytes(2)?;
        Some(BorderInfo {
            flags: BorderFlags {
                box_all: raw & 0x01 != 0,
                top: raw & 0x02 != 0,
                left: raw & 0x04 != 0,
                bottom: raw & 0x08 != 0,
                right: raw & 0x10 != 0,
            },
            style: BorderStyle::from_code((raw >> 5) & 0x07),
            raw_extra: [extra[0], extra[1]],
        })
    } else {
        None
    };

    let tabs = if bits & (1 << 9) != 0 {
        parse_tabs(reader)?
    } else {
        Vec::new()
    };
    // WinHlp32 stores this as a two-bit value and only value 2 is centered.
    // Values 1 and 3 both take the right-alignment path (0x416170..0x416187).
    let alignment = match (bits >> 10) & 0x03 {
        0 => ParagraphAlignment::Left,
        2 => ParagraphAlignment::Center,
        1 | 3 => ParagraphAlignment::Right,
        _ => unreachable!(),
    };
    let no_wrap = bits & (1 << 12) != 0;
    let right_to_left = bits & (1 << 13) != 0;

    Ok(ParagraphFormat {
        column: None,
        id,
        unknown_value,
        spacing_above,
        spacing_below,
        spacing_lines,
        left_indent,
        right_indent,
        first_line_indent,
        default_tab_interval,
        alignment,
        no_wrap,
        right_to_left,
        border,
        tabs,
    })
}

fn conditional<T>(
    bits: u16,
    bit: u8,
    read: impl FnOnce() -> Result<T, HlpError>,
) -> Result<Option<T>, HlpError> {
    if bits & (1_u16 << bit) != 0 {
        read().map(Some)
    } else {
        Ok(None)
    }
}

fn parse_tabs(reader: &mut Reader<'_>) -> Result<Vec<TabStop>, HlpError> {
    let count = reader.read_compressed_signed_short()?;
    if count < 0 {
        return Err(HlpError::invalid("paragraph tabs", format!("negative tab count {count}")));
    }
    let count = usize::try_from(count).map_err(|_| HlpError::invalid("paragraph tabs", "tab count does not fit usize"))?;
    if count > MAX_TABS {
        return Err(HlpError::invalid("paragraph tabs", format!("tab count {count} exceeds {MAX_TABS}")));
    }
    let mut tabs = Vec::with_capacity(count);
    for _ in 0..count {
        let raw = reader.read_compressed_unsigned_short()?;
        let alignment = if raw & 0x4000 != 0 {
            match reader.read_compressed_unsigned_short()? {
                0 => TabAlignment::Left,
                1 => TabAlignment::Right,
                2 => TabAlignment::Center,
                other => TabAlignment::Unknown(other),
            }
        } else {
            TabAlignment::Left
        };
        tabs.push(TabStop {
            position: raw & 0x3FFF,
            alignment,
        });
    }
    Ok(tabs)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharacterEnding {
    SameParagraphFormat,
    NextParagraphInfo,
    Stop,
}

/// Decodes one paragraph's character stream.
///
/// `font_index` is the caller's running WinHelp font selection. It is deliberately not reset
/// here: see [`FormattedRecord::decode_with_font`] for the reference-viewer behaviour this
/// mirrors. `hotspot` stays paragraph-local because opcode `0x89` is its explicit terminator
/// and WinHelp never carries an open hotspot across a paragraph break.
fn parse_character_stream(
    ld1: &mut Reader<'_>,
    strings: &mut StringStream<'_>,
    format: ParagraphFormat,
    font_index: &mut u16,
    issues: &mut Vec<FormattingIssue>,
) -> Result<(Paragraph, CharacterEnding), HlpError> {
    let mut inlines = Vec::new();
    let mut hotspot: Option<Hotspot> = None;

    loop {
        if let Some(text) = strings.next_string_for_font(*font_index)? {
            if !text.is_empty() {
                inlines.push(Inline::Text(TextRun {
                    text,
                    font_index: *font_index,
                    hotspot: hotspot.clone(),
                }));
            }
        }

        if ld1.remaining() == 0 {
            return Ok((Paragraph { format, inlines }, CharacterEnding::Stop));
        }
        let command_offset = ld1.position();
        let command = ld1.read_u8()?;
        match command {
            0xFF => return Ok((Paragraph { format, inlines }, CharacterEnding::NextParagraphInfo)),
            0x80 => *font_index = ld1.read_u16()?,
            0x81 => inlines.push(Inline::LineBreak),
            0x82 => return Ok((Paragraph { format, inlines }, CharacterEnding::SameParagraphFormat)),
            0x83 => inlines.push(Inline::Tab),
            // KB917607 WinHlp32 0x41abc3/0x417816 consumes this as a three-byte control and
            // stores the signed WORD in transient layout state. It is invisible by itself.
            0x85 => inlines.push(Inline::Control85(ld1.read_i16()?)),
            0x86..=0x88 => {
                if let Some(object) = parse_inline_compact_object(ld1, command, command_offset, issues)? {
                    inlines.push(object);
                }
            }
            0x89 => hotspot = None,
            0xC8 | 0xCC => {
                hotspot = Some(parse_macro_hotspot(ld1, command == 0xC8)?);
            }
            0xE0 | 0xE1 => {
                let offset = TopicOffset(ld1.read_i32()?);
                hotspot = Some(Hotspot {
                    target: HotspotTarget::Internal {
                        offset,
                        popup: command & 1 == 0,
                    },
                    emphasized: true,
                });
            }
            0xE2 | 0xE3 | 0xE6 | 0xE7 => {
                let hash = ld1.read_i32()?;
                hotspot = Some(Hotspot {
                    target: HotspotTarget::ContextHash {
                        hash,
                        popup: command & 1 == 0,
                    },
                    emphasized: matches!(command, 0xE2 | 0xE3),
                });
            }
            0xEA | 0xEB | 0xEE | 0xEF => {
                hotspot = Some(parse_external_hotspot(ld1, command)?);
            }
            // KB917607 0x41ac3b..0x41ac5c and 0x417996..0x4179d2 accept the complete
            // C0-CF/E0-EF hotspot envelope families. A second audit of the click dispatcher at
            // 0x429C13 proves that the remaining envelope values have no navigation/action
            // branch in this runtime: only C8/CC, E0-E3, E6/E7, EA/EB and EE/EF dispatch.
            // Preserve the exact envelope and following text, but deliberately leave these
            // verified-inert variants non-clickable rather than inventing a target.
            other if other & 0xD8 == 0xC0 => {
                let _payload = ld1.read_bytes(4)?;
                hotspot = None;
                issues.push(FormattingIssue {
                    link_data1_offset: command_offset,
                    layout_safe: true,
                    message: format!(
                        "KB917607-inert fixed hotspot opcode 0x{other:02X}; exact four-byte payload consumed and no click action dispatched"
                    ),
                });
            }
            other if other & 0xD8 == 0xC8 => {
                let payload_len = usize::from(ld1.read_u16()?);
                let _payload = ld1.read_bytes(payload_len)?;
                hotspot = None;
                issues.push(FormattingIssue {
                    link_data1_offset: command_offset,
                    layout_safe: true,
                    message: format!(
                        "KB917607-inert variable hotspot opcode 0x{other:02X}; exact {payload_len}-byte payload consumed and no click action dispatched"
                    ),
                });
            }
            other => {
                issues.push(FormattingIssue {
                    link_data1_offset: command_offset,
                    layout_safe: false,
                    message: format!("unsupported character-formatting opcode 0x{other:02X}; record decoding stopped safely"),
                });
                return Ok((Paragraph { format, inlines }, CharacterEnding::Stop));
            }
        }
    }
}

fn parse_macro_hotspot(reader: &mut Reader<'_>, emphasized: bool) -> Result<Hotspot, HlpError> {
    // KB917607 0x41ac4f..0x41ac5c and 0x4179c9..0x4179d2 treat the WORD after
    // every C8-family opcode as the number of payload bytes that follow it. It does not
    // include the command byte or the length WORD itself.
    let payload_len = usize::from(reader.read_u16()?);
    let bytes = reader.read_bytes(payload_len)?;
    let macro_text = decode_windows_1252(bytes).trim_end_matches('\0').to_owned();
    Ok(Hotspot {
        target: HotspotTarget::Macro(macro_text),
        emphasized,
    })
}

fn parse_external_hotspot(reader: &mut Reader<'_>, opcode: u8) -> Result<Hotspot, HlpError> {
    let total = reader.read_i16()?;
    if total < 5 {
        return Err(HlpError::invalid("external hotspot", format!("invalid following-structure size {total}")));
    }
    let payload_len = usize::try_from(total)
        .map_err(|_| HlpError::invalid("external hotspot", "length conversion failed"))?;
    let payload = reader.read_bytes(payload_len)?;
    let mut body = Reader::new(payload, "external hotspot payload");
    let type_code = body.read_u8()?;
    let offset = TopicOffset(body.read_i32()?);
    let window_number = if type_code == 1 && body.remaining() > 0 {
        Some(body.read_u8()?)
    } else {
        None
    };
    // Type 6 stores the window name first and the help-file path second. Type 4 stores only
    // the help-file path. Keeping this order matches the original WinHelp link reader and avoids
    // accidentally treating a secondary-window name as a filename.
    let window_name = if type_code == 6 && body.remaining() > 0 {
        Some(decode_windows_1252(body.read_c_string()?))
    } else {
        None
    };
    let help_file = if matches!(type_code, 4 | 6) && body.remaining() > 0 {
        Some(decode_windows_1252(body.read_c_string()?))
    } else {
        None
    };
    Ok(Hotspot {
        target: HotspotTarget::External {
            opcode,
            type_code,
            offset,
            window_number,
            help_file,
            window_name,
        },
        emphasized: matches!(opcode, 0xEA | 0xEE),
    })
}

fn inline_object_position(command: u8) -> Result<PicturePosition, HlpError> {
    match command {
        0x86 => Ok(PicturePosition::Inline),
        0x87 => Ok(PicturePosition::FloatLeft),
        0x88 => Ok(PicturePosition::FloatRight),
        _ => Err(HlpError::invalid(
            "inline compact object",
            format!("unexpected inline-object opcode 0x{command:02X}"),
        )),
    }
}

/// Decodes the compact TOPICLINK nested behind character commands 0x86/0x87/0x88.
///
/// These commands do not imply "picture". KB917607 scanner/dispatcher paths parse the nested
/// record header first and route by its own TOPICLINK type. In particular CALC.HLP carries a 0x05
/// hosted-window record here; treating it as a graphic produced the bogus `[embedded picture]`
/// placeholder seen by the viewer.
fn parse_inline_compact_object(
    reader: &mut Reader<'_>,
    command: u8,
    command_offset: usize,
    issues: &mut Vec<FormattingIssue>,
) -> Result<Option<Inline>, HlpError> {
    let position = inline_object_position(command)?;
    let header = read_compact_record_header(reader)?;
    if header.payload_size > MAX_COMPACT_OBJECT_BYTES {
        return Err(HlpError::invalid(
            "inline compact object",
            format!(
                "compact payload {} exceeds safety limit {MAX_COMPACT_OBJECT_BYTES}",
                header.payload_size
            ),
        ));
    }
    let payload = reader.read_bytes(header.payload_size)?;

    match header.record_type {
        TopicRecordType::Graphic30 | TopicRecordType::Graphic => {
            let picture_type = match header.record_type {
                TopicRecordType::Graphic30 => 0x03,
                TopicRecordType::Graphic => 0x22,
                _ => unreachable!(),
            };
            Ok(Some(Inline::Picture(PictureReference {
                command,
                position,
                picture_type,
                encoded_size: header.payload_size,
                hotspot_count: header.topic_length,
                source: parse_picture_source(picture_type, payload.to_vec()),
                image: None,
                hotspots: Vec::new(),
                decode_warning: None,
            })))
        }
        TopicRecordType::EmbeddedWindow30 | TopicRecordType::EmbeddedWindow => {
            match parse_embedded_window(header.record_type, payload) {
                Ok(window) => {
                    issues.push(FormattingIssue {
                        link_data1_offset: command_offset,
                        layout_safe: true,
                        message: format!(
                            "inline compact hosted-window record {:?} retained safely; native authored control execution is disabled",
                            header.record_type
                        ),
                    });
                    Ok(Some(Inline::EmbeddedWindow(window)))
                }
                Err(message) => {
                    issues.push(FormattingIssue {
                        link_data1_offset: command_offset,
                        layout_safe: true,
                        message,
                    });
                    Ok(None)
                }
            }
        }
        TopicRecordType::NoRender30 => {
            issues.push(FormattingIssue {
                link_data1_offset: command_offset,
                layout_safe: true,
                message: "inline compact no-render record 0x06 consumed on its exact bounded payload".to_owned(),
            });
            Ok(None)
        }
        other => {
            issues.push(FormattingIssue {
                link_data1_offset: command_offset,
                layout_safe: true,
                message: format!(
                    "inline compact record {other:?} is structurally bounded but has no inline renderer; skipped {} payload bytes",
                    header.payload_size
                ),
            });
            Ok(None)
        }
    }
}

fn parse_picture_source(picture_type: u8, mut payload: Vec<u8>) -> PictureSource {
    if !matches!(picture_type, 0x03 | 0x22) || payload.len() < 2 {
        return PictureSource::Unsupported(payload);
    }
    // KB917607 graphics loader 0x4062DF treats selector WORD 0 as an indexed resource and every
    // nonzero selector as an embedded logical graphics stream beginning immediately after it.
    // The indexed value is read signed and negative indices are rejected by the reference.
    let selector = u16::from_le_bytes([payload[0], payload[1]]);
    if selector == 0 {
        if payload.len() < 4 {
            return PictureSource::Unsupported(payload);
        }
        let index = i16::from_le_bytes([payload[2], payload[3]]);
        return u16::try_from(index)
            .map(PictureSource::Indexed)
            .unwrap_or(PictureSource::Unsupported(payload));
    }
    if payload.len() > 2 {
        PictureSource::Embedded(payload.split_off(2))
    } else {
        PictureSource::Unsupported(payload)
    }
}

struct StringStream<'a> {
    reader: Reader<'a>,
    charsets: Vec<Option<u8>>,
}

impl<'a> StringStream<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self::with_fonts(bytes, None)
    }

    fn with_fonts(bytes: &'a [u8], fonts: Option<&FontTable>) -> Self {
        let charsets = fonts.map_or_else(Vec::new, |table| {
            table.descriptors().iter().map(|font| font.charset).collect()
        });
        Self {
            reader: Reader::new(bytes, "display LinkData2"),
            charsets,
        }
    }

    fn next_string_for_font(&mut self, font_index: u16) -> Result<Option<String>, HlpError> {
        if self.reader.remaining() == 0 {
            return Ok(None);
        }
        let raw = self.reader.read_c_string()?;
        let charset = self
            .charsets
            .get(usize::from(font_index))
            .copied()
            .flatten();
        Ok(Some(decode_windows_charset(raw, charset)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TopicPos, TopicRegion};

    #[test]
    fn compressed_numbers_follow_winhelp_bias_rules() {
        let bytes = [20_u8, 5, 2, 4, 0, 5, 0, 1, 0];
        let mut reader = Reader::new(&bytes, "test");
        assert_eq!(reader.read_compressed_unsigned_short().unwrap(), 10);
        assert_eq!(reader.read_compressed_unsigned_short().unwrap(), 258);
        assert_eq!(reader.read_compressed_unsigned_long().unwrap(), 2);
        assert_eq!(reader.read_compressed_unsigned_long().unwrap(), 32_770);

        // Microsoft 0x4129e8: short zero is 0x8000; extended zero is 0x80000001.
        let mut signed = Reader::new(
            &[0x00, 0x80, 0x01, 0x00, 0x00, 0x80],
            "signed compressed long test",
        );
        assert_eq!(signed.read_compressed_signed_long().unwrap(), 0);
        assert_eq!(signed.read_compressed_signed_long().unwrap(), 0);
        assert_eq!(signed.remaining(), 0);
    }

    #[test]
    fn decodes_font_change_text_and_internal_hotspot() {
        // topic size = 0 in signed-compressed form (-16384 + 16384 requires extended form)
        // Use a simple positive value 16: first word = (16 + 16384) * 2 = 0x8020.
        let mut ld1 = vec![0x20, 0x80];
        // modern TopicLength = 3
        ld1.push(6);
        // ParagraphInfo: compressed signed leading value 0, id, flags
        ld1.extend_from_slice(&[0, 0x80, 1, 0, 0, 0]);
        // First string then set font 2, second string then start jump, third then end hotspot, final empty + FF.
        ld1.push(0x80);
        ld1.extend_from_slice(&2_u16.to_le_bytes());
        ld1.push(0xE1);
        ld1.extend_from_slice(&123_i32.to_le_bytes());
        ld1.push(0x89);
        ld1.push(0xFF);

        let record = TopicRecord {
            position: TopicPos(12),
            record_type: TopicRecordType::Display,
            region: TopicRegion::Scrolling,
            link_data1: ld1,
            link_data2: b"A\0B\0C\0\0".to_vec(),
            plain_text: "ABC".to_owned(),
        };
        let decoded = FormattedRecord::decode(&record).unwrap();
        assert_eq!(decoded.paragraphs.len(), 1);
        let text: Vec<_> = decoded.paragraphs[0]
            .inlines
            .iter()
            .filter_map(|inline| match inline { Inline::Text(run) => Some(run), _ => None })
            .collect();
        assert_eq!(text.len(), 3);
        assert_eq!(text[0].font_index, 0);
        assert_eq!(text[1].font_index, 2);
        assert!(matches!(text[2].hotspot.as_ref().map(|h| &h.target), Some(HotspotTarget::Internal { offset: TopicOffset(123), popup: false })));
    }

    /// Builds a display record whose LinkData1 carries `stream` after the standard preamble.
    fn display_record(stream: &[u8], link_data2: &[u8]) -> TopicRecord {
        let mut ld1 = vec![0x20, 0x80];
        ld1.push(6);
        ld1.extend_from_slice(&[0, 0x80, 1, 0, 0, 0]);
        ld1.extend_from_slice(stream);
        TopicRecord {
            position: TopicPos(12),
            record_type: TopicRecordType::Display,
            region: TopicRegion::Scrolling,
            link_data1: ld1,
            link_data2: link_data2.to_vec(),
            plain_text: String::new(),
        }
    }

    fn text_runs(paragraph: &Paragraph) -> Vec<&TextRun> {
        paragraph
            .inlines
            .iter()
            .filter_map(|inline| match inline { Inline::Text(run) => Some(run), _ => None })
            .collect()
    }

    #[test]
    fn control_85_is_consumed_as_an_invisible_signed_marker() {
        let mut reader = Reader::new(&[0x85, 0xFE, 0xFF], "0x85 character control");
        let mut strings = StringStream::new(b"\0\0");
        let mut font_index = 0_u16;
        let mut issues = Vec::new();
        let (paragraph, ending) = parse_character_stream(
            &mut reader,
            &mut strings,
            ParagraphFormat::default(),
            &mut font_index,
            &mut issues,
        )
        .unwrap();

        assert_eq!(ending, CharacterEnding::Stop);
        assert_eq!(paragraph.inlines, vec![Inline::Control85(-2)]);
        assert_eq!(reader.remaining(), 0);
        assert!(issues.is_empty());
    }

    #[test]
    fn bytes_20_21_8b_and_8c_are_not_character_commands_in_kb917607() {
        for command in [0x20_u8, 0x21, 0x8B, 0x8C] {
            let payload = [command, 0x11, 0x22, 0x33, 0x44];
            let mut reader = Reader::new(&payload, "rejected character control");
            let mut strings = StringStream::new(b"\0");
            let mut font_index = 0_u16;
            let mut issues = Vec::new();
            let (_paragraph, ending) = parse_character_stream(
                &mut reader,
                &mut strings,
                ParagraphFormat::default(),
                &mut font_index,
                &mut issues,
            )
            .unwrap();

            assert_eq!(ending, CharacterEnding::Stop);
            assert_eq!(reader.position(), 1, "command 0x{command:02X}");
            assert_eq!(issues.len(), 1, "command 0x{command:02X}");
        }
    }

    #[test]
    fn macro_hotspot_word_is_following_payload_length_not_total_record_size() {
        // The C8/CC WORD is the exact number of bytes after the WORD. A historical `len - 3`
        // interpretation left three macro bytes in LinkData1, which could be misread as commands.
        let mut payload = vec![0xC8];
        payload.extend_from_slice(&5_u16.to_le_bytes());
        payload.extend_from_slice(b"Go()\0");
        payload.push(0x89);
        payload.push(0xFF);

        let mut reader = Reader::new(&payload, "macro hotspot payload length");
        let mut strings = StringStream::new(b"\0linked\0after\0");
        let mut font_index = 0_u16;
        let mut issues = Vec::new();
        let (paragraph, ending) = parse_character_stream(
            &mut reader,
            &mut strings,
            ParagraphFormat::default(),
            &mut font_index,
            &mut issues,
        )
        .unwrap();

        assert_eq!(ending, CharacterEnding::NextParagraphInfo);
        assert_eq!(reader.remaining(), 0);
        assert!(issues.is_empty());
        let runs = text_runs(&paragraph);
        assert_eq!(runs.iter().map(|run| run.text.as_str()).collect::<Vec<_>>(), vec!["linked", "after"]);
        let hotspot = runs[0].hotspot.as_ref().unwrap();
        assert!(hotspot.emphasized);
        assert!(matches!(&hotspot.target, HotspotTarget::Macro(text) if text == "Go()"));
        assert!(runs[1].hotspot.is_none());
    }

    #[test]
    fn unresolved_hotspot_families_keep_exact_stream_synchronization() {
        for (payload, opcode) in [
            (vec![0xC0, 1, 2, 3, 4, 0xFF], 0xC0_u8),
            (vec![0xC9, 2, 0, 0xAA, 0xBB, 0xFF], 0xC9_u8),
        ] {
            let mut reader = Reader::new(&payload, "unresolved hotspot family");
            let mut strings = StringStream::new(b"A\0B\0");
            let mut font_index = 0_u16;
            let mut issues = Vec::new();
            let (paragraph, ending) = parse_character_stream(
                &mut reader,
                &mut strings,
                ParagraphFormat::default(),
                &mut font_index,
                &mut issues,
            )
            .unwrap();

            assert_eq!(ending, CharacterEnding::NextParagraphInfo, "opcode 0x{opcode:02X}");
            assert_eq!(reader.remaining(), 0, "opcode 0x{opcode:02X}");
            assert_eq!(text_runs(&paragraph).iter().map(|run| run.text.as_str()).collect::<Vec<_>>(), vec!["A", "B"]);
            assert_eq!(issues.len(), 1, "opcode 0x{opcode:02X}");
            assert!(issues[0].layout_safe, "opcode 0x{opcode:02X}");
        }
    }

    #[test]
    fn top_level_compact_graphic_uses_the_same_picture_payload_as_table_cells() {
        let mut link_data1 = vec![0x08, 0x80]; // compressed signed payload size 4
        link_data1.extend_from_slice(&0_u16.to_le_bytes()); // indexed selector
        link_data1.extend_from_slice(&3_i16.to_le_bytes()); // |bm3
        let record = TopicRecord {
            position: TopicPos(12),
            record_type: TopicRecordType::Graphic30,
            region: TopicRegion::Scrolling,
            link_data1,
            link_data2: Vec::new(),
            plain_text: String::new(),
        };

        let decoded = FormattedRecord::decode(&record).unwrap();
        assert_eq!(decoded.topic_size, 4);
        assert_eq!(decoded.topic_length, None);
        assert!(decoded.issues.is_empty());
        assert!(matches!(
            decoded.paragraphs[0].inlines.as_slice(),
            [Inline::Picture(PictureReference { source: PictureSource::Indexed(3), .. })]
        ));
    }

    #[test]
    fn top_level_hosted_window_is_retained_as_a_layout_safe_placeholder() {
        let mut payload = vec![1, 2, 3, 4, 5, 6];
        payload.extend_from_slice(b"BUTTON\0");
        let encoded_size = u16::try_from((payload.len() + 16_384) * 2).unwrap();
        let mut link_data1 = encoded_size.to_le_bytes().to_vec();
        link_data1.push(0); // modern TopicLength 0
        link_data1.extend_from_slice(&payload);
        let record = TopicRecord {
            position: TopicPos(12),
            record_type: TopicRecordType::EmbeddedWindow,
            region: TopicRegion::Scrolling,
            link_data1,
            link_data2: Vec::new(),
            plain_text: String::new(),
        };

        let decoded = FormattedRecord::decode(&record).unwrap();
        assert_eq!(decoded.issues.len(), 1);
        assert!(decoded.issues[0].layout_safe);
        assert!(matches!(
            decoded.paragraphs[0].inlines.as_slice(),
            [Inline::EmbeddedWindow(EmbeddedWindowReference { descriptor, .. })] if descriptor == "BUTTON"
        ));
    }

    #[test]
    fn font_selection_survives_a_paragraph_boundary() {
        // Paragraph one selects font 2; paragraph two emits no 0x80 at all. WinHlp32 keeps the
        // selection in a global that no paragraph terminator clears, so the second paragraph must
        // stay on font 2 rather than falling back to descriptor 0.
        let mut stream = vec![0x80];
        stream.extend_from_slice(&2_u16.to_le_bytes());
        stream.push(0x82);
        stream.push(0xFF);

        let decoded = FormattedRecord::decode(&display_record(&stream, b"A\0B\0C\0\0")).unwrap();
        assert_eq!(decoded.paragraphs.len(), 2);

        let first = text_runs(&decoded.paragraphs[0]);
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].font_index, 0);
        assert_eq!(first[1].font_index, 2);

        let second = text_runs(&decoded.paragraphs[1]);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].font_index, 2);
    }

    #[test]
    fn font_selection_survives_a_record_boundary() {
        let mut first_stream = vec![0x80];
        first_stream.extend_from_slice(&3_u16.to_le_bytes());
        first_stream.push(0xFF);

        let mut font_index = 0_u16;
        let first =
            FormattedRecord::decode_with_font(&display_record(&first_stream, b"A\0\0"), &mut font_index)
                .unwrap();
        assert_eq!(text_runs(&first.paragraphs[0])[0].font_index, 0);
        assert_eq!(font_index, 3);

        let second =
            FormattedRecord::decode_with_font(&display_record(&[0xFF], b"B\0\0"), &mut font_index)
                .unwrap();
        assert_eq!(text_runs(&second.paragraphs[0])[0].font_index, 3);
    }

    #[test]
    fn document_font_charset_decodes_hebrew_linkdata2_before_layout() {
        let mut fonts = FontTable::fallback();
        fonts.apply_system_metadata(&[0xB1], Some(0x040D));
        let mut font_index = 0_u16;
        let record = display_record(&[0xFF], &[0xF9, 0xEC, 0xE5, 0xED, 0, 0]);

        let decoded = FormattedRecord::decode_with_font_table(&record, &mut font_index, &fonts)
            .unwrap();
        let runs = text_runs(&decoded.paragraphs[0]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "שלום");
        assert_eq!(runs[0].font_index, 0);
    }

    #[test]
    fn document_font_charset_decodes_shift_jis_linkdata2_before_layout() {
        let mut fonts = FontTable::fallback();
        fonts.apply_system_metadata(&[0x80], Some(0x0411));
        let mut font_index = 0_u16;
        let record = display_record(&[0xFF], &[0x82, 0xB1, 0x82, 0xF1, 0, 0]);

        let decoded = FormattedRecord::decode_with_font_table(&record, &mut font_index, &fonts)
            .unwrap();
        let runs = text_runs(&decoded.paragraphs[0]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "こん");
        assert_eq!(runs[0].font_index, 0);
    }

    #[test]
    fn decodes_context_hash_hotspot_separately_from_topic_offset() {
        let mut ld1 = vec![0x20, 0x80];
        ld1.push(2);
        ld1.extend_from_slice(&[0, 0x80, 1, 0, 0, 0]);
        ld1.push(0xE3);
        ld1.extend_from_slice(&0x438E51B4_i32.to_le_bytes());
        ld1.push(0x89);
        ld1.push(0xFF);

        let record = TopicRecord {
            position: TopicPos(12),
            record_type: TopicRecordType::Display,
            region: TopicRegion::Scrolling,
            link_data1: ld1,
            // LinkData1 commands are consumed after each LinkData2 string, so the leading
            // empty string lets E3 start the hotspot before the visible text run.
            link_data2: b"\0operators\0\0".to_vec(),
            plain_text: "operators".to_owned(),
        };
        let decoded = FormattedRecord::decode(&record).unwrap();
        let run = decoded.paragraphs[0]
            .inlines
            .iter()
            .find_map(|inline| match inline { Inline::Text(run) => Some(run), _ => None })
            .unwrap();
        assert!(matches!(
            run.hotspot.as_ref().map(|h| &h.target),
            Some(HotspotTarget::ContextHash { hash: 0x438E51B4, popup: false })
        ));
    }

    #[test]
    fn invisible_context_hash_hotspot_preserves_non_emphasized_style() {
        let mut ld1 = vec![0x20, 0x80];
        ld1.push(2);
        ld1.extend_from_slice(&[0, 0x80, 1, 0, 0, 0]);
        ld1.push(0xE7);
        ld1.extend_from_slice(&123_i32.to_le_bytes());
        ld1.push(0x89);
        ld1.push(0xFF);

        let record = TopicRecord {
            position: TopicPos(12),
            record_type: TopicRecordType::Display,
            region: TopicRegion::Scrolling,
            link_data1: ld1,
            // E7 likewise applies to the following string; preserve that stream ordering in
            // the fixture so this test checks non-emphasized context-hash decoding itself.
            link_data2: b"\0hidden\0\0".to_vec(),
            plain_text: "hidden".to_owned(),
        };
        let decoded = FormattedRecord::decode(&record).unwrap();
        let run = decoded.paragraphs[0]
            .inlines
            .iter()
            .find_map(|inline| match inline { Inline::Text(run) => Some(run), _ => None })
            .unwrap();
        let hotspot = run.hotspot.as_ref().unwrap();
        assert!(!hotspot.emphasized);
        assert!(matches!(
            hotspot.target,
            HotspotTarget::ContextHash { hash: 123, popup: false }
        ));
    }


    #[test]
    fn paragraph_bit7_consumes_authored_default_tab_before_border_data() {
        // Paragraph flags: bit 7 default-tab interval + bit 8 border. Signed-compressed 72 is
        // [0x91, 0x80]. This specifically guards the stream-desynchronization bug found by
        // tracing KB917607 WinHlp32's paragraph parser at 0x41278c.
        let bytes = [
            0, 0x80, 1, 0, 0x80, 0x01,
            0x91, 0x80,
            0x01, 0x7b, 0x00,
        ];
        let mut reader = Reader::new(&bytes, "paragraph flags");
        let format = parse_paragraph_info(&mut reader).unwrap();
        assert_eq!(format.default_tab_interval, Some(72));
        let border = format.border.unwrap();
        assert!(border.flags.box_all);
        assert_eq!(border.style, BorderStyle::Normal);
        assert_eq!(border.raw_extra, [0x7b, 0x00]);
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn paragraph_border_high_bits_are_one_reference_style_code() {
        for (code, expected) in [
            (0, BorderStyle::Normal),
            (1, BorderStyle::Thick),
            (2, BorderStyle::Double),
            (3, BorderStyle::Shadow),
            (4, BorderStyle::ReferenceStyle4),
            (7, BorderStyle::Reserved(7)),
        ] {
            let bits = 1_u16 << 8;
            let mut bytes = vec![0, 0x80, 1, 0];
            bytes.extend_from_slice(&bits.to_le_bytes());
            bytes.extend_from_slice(&[(code << 5) | 0x01, 0xaa, 0x55]);
            let mut reader = Reader::new(&bytes, "paragraph border style");
            let format = parse_paragraph_info(&mut reader).unwrap();
            let border = format.border.unwrap();
            assert_eq!(border.style, expected);
            assert_eq!(border.raw_extra, [0xaa, 0x55]);
        }
    }

    #[test]
    fn paragraph_high_flags_match_reference_alignment_and_nowrap_bits() {
        // Both alignment bits set produce value 3, which Microsoft routes as right alignment.
        // Bit 12 is no-wrap; bit 13 selects the reference right-to-left paragraph path.
        let bits = (3_u16 << 10) | (1 << 12) | (1 << 13);
        let mut bytes = vec![0, 0x80, 1, 0];
        bytes.extend_from_slice(&bits.to_le_bytes());
        let mut reader = Reader::new(&bytes, "paragraph high flags");
        let format = parse_paragraph_info(&mut reader).unwrap();
        assert_eq!(format.alignment, ParagraphAlignment::Right);
        assert!(format.no_wrap);
        assert!(format.right_to_left);
    }
    #[test]
    fn table_header_preserves_reference_column_geometry() {
        // signed-compressed topic size 16, topic length 1, type-0 table with two columns.
        let mut ld1 = vec![0x20, 0x80, 2, 2, 0];
        ld1.extend_from_slice(&100_u16.to_le_bytes());
        // WinHlp32 stores each pair as width first, then gap-before-column.
        for value in [1000_u16, 10, 2000, 20] {
            ld1.extend_from_slice(&value.to_le_bytes());
        }
        // end-of-table ParagraphInfo marker
        ld1.extend_from_slice(&(-1_i16).to_le_bytes());
        let record = TopicRecord {
            position: TopicPos(12),
            record_type: TopicRecordType::Table,
            region: TopicRegion::Scrolling,
            link_data1: ld1,
            link_data2: Vec::new(),
            plain_text: String::new(),
        };
        let decoded = FormattedRecord::decode(&record).unwrap();
        let table = decoded.table.unwrap();
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.minimum_width, Some(100));
        assert_eq!(table.columns[0].width, 1000);
        assert_eq!(table.columns[0].gap_before, 10);
        assert_eq!(table.columns[1].width, 2000);
        assert_eq!(table.columns[1].gap_before, 20);
    }

    #[test]
    fn modern_table_cells_are_nested_compact_display_records() {
        // KB917607 0x414F66: after geometry each cell is `i16 column` followed by the
        // complete compact record consumed by 0x412884/0x417578.  A modern display cell
        // therefore carries type 0x20, compressed payload size, compressed TopicLength,
        // and only then ParagraphInfo + character commands.
        let mut ld1 = vec![0x20, 0x80, 2]; // top-level TopicSize 16, TopicLength 1
        ld1.extend_from_slice(&[2, 1]); // two columns, absolute table type
        for (width, gap) in [(144_u16, 0_u16), (144, 12)] {
            ld1.extend_from_slice(&width.to_le_bytes());
            ld1.extend_from_slice(&gap.to_le_bytes());
        }

        let paragraph_payload = [
            0x00, 0x80, // compressed signed paragraph-leading value 0
            0x01, 0x00, // paragraph id
            0x00, 0x00, // paragraph flags
            0xFF,       // next ParagraphInfo / end this bounded cell payload
        ];
        for column in [0_i16, 1_i16] {
            ld1.extend_from_slice(&column.to_le_bytes());
            ld1.push(0x20); // nested modern display record
            ld1.extend_from_slice(&0x800E_u16.to_le_bytes()); // compressed signed payload size 7
            ld1.push(0); // compressed unsigned TopicLength 0
            ld1.extend_from_slice(&paragraph_payload);
        }
        ld1.extend_from_slice(&(-1_i16).to_le_bytes());

        let record = TopicRecord {
            position: TopicPos(12),
            record_type: TopicRecordType::Table,
            region: TopicRegion::Scrolling,
            link_data1: ld1,
            link_data2: b"left\0right\0".to_vec(),
            plain_text: "left right".to_owned(),
        };
        let decoded = FormattedRecord::decode(&record).unwrap();
        assert_eq!(decoded.paragraphs.len(), 2);
        assert_eq!(decoded.paragraphs[0].format.column, Some(0));
        assert_eq!(decoded.paragraphs[1].format.column, Some(1));
        assert_eq!(text_runs(&decoded.paragraphs[0])[0].text, "left");
        assert_eq!(text_runs(&decoded.paragraphs[1])[0].text, "right");
        assert_eq!(decoded.table_cells.len(), 2);
        assert!(matches!(
            &decoded.table_cells[0].content,
            TableCellContent::Display {
                paragraph_start: 0,
                paragraph_end: 1
            }
        ));
        assert!(matches!(
            &decoded.table_cells[1].content,
            TableCellContent::Display {
                paragraph_start: 1,
                paragraph_end: 2
            }
        ));
        assert!(decoded.issues.is_empty());
    }

    #[test]
    fn windows_30_table_type_04_uses_old_compact_display_cells() {
        // 0x417578 dispatches type 0x04 to the same table renderer as type 0x23.  Its
        // nested type-0x01 display record has no modern TopicLength field.
        let mut ld1 = vec![0x20, 0x80]; // top-level TopicSize 16, no TopicLength for 0x04
        ld1.extend_from_slice(&[1, 1]); // one column, absolute table type
        ld1.extend_from_slice(&144_u16.to_le_bytes());
        ld1.extend_from_slice(&0_u16.to_le_bytes());
        ld1.extend_from_slice(&0_i16.to_le_bytes());
        ld1.push(0x01); // nested Windows 3.0 display record
        ld1.extend_from_slice(&0x800E_u16.to_le_bytes()); // payload size 7
        ld1.extend_from_slice(&[
            0x00, 0x80,
            0x01, 0x00,
            0x00, 0x00,
            0xFF,
        ]);
        ld1.extend_from_slice(&(-1_i16).to_le_bytes());

        let record = TopicRecord {
            position: TopicPos(12),
            record_type: TopicRecordType::Table30,
            region: TopicRegion::Scrolling,
            link_data1: ld1,
            link_data2: b"old table\0".to_vec(),
            plain_text: "old table".to_owned(),
        };
        let decoded = FormattedRecord::decode(&record).unwrap();
        assert_eq!(decoded.topic_length, None);
        assert_eq!(decoded.paragraphs.len(), 1);
        assert_eq!(decoded.paragraphs[0].format.column, Some(0));
        assert_eq!(text_runs(&decoded.paragraphs[0])[0].text, "old table");
        assert_eq!(decoded.table_cells.len(), 1);
        assert!(matches!(
            &decoded.table_cells[0].content,
            TableCellContent::Display {
                paragraph_start: 0,
                paragraph_end: 1
            }
        ));
        assert!(decoded.issues.is_empty());
    }

    #[test]
    fn windows_30_nested_table_recurses_without_modern_topic_length() {
        // The old 0x04 table branch in dispatcher 0x417578 re-enters the same table walker, but
        // unlike 0x23 its compact header has no modern TopicLength field. This fixture protects
        // that generation distinction while exercising real recursive retention.
        let mut ld1 = vec![0x20, 0x80]; // top-level TopicSize 16, no modern TopicLength
        ld1.extend_from_slice(&[1, 1]); // one outer absolute column
        ld1.extend_from_slice(&288_u16.to_le_bytes());
        ld1.extend_from_slice(&0_u16.to_le_bytes());

        let paragraph_payload = [
            0x00, 0x80,
            0x01, 0x00,
            0x00, 0x00,
            0xFF,
        ];

        // Nested old table payload: one column, one old 0x01 display cell, then -1.
        let mut nested_payload = vec![1, 1];
        nested_payload.extend_from_slice(&144_u16.to_le_bytes());
        nested_payload.extend_from_slice(&0_u16.to_le_bytes());
        nested_payload.extend_from_slice(&0_i16.to_le_bytes());
        nested_payload.push(0x01);
        nested_payload.extend_from_slice(&0x800E_u16.to_le_bytes()); // payload size 7
        nested_payload.extend_from_slice(&paragraph_payload);
        nested_payload.extend_from_slice(&(-1_i16).to_le_bytes());
        assert_eq!(nested_payload.len(), 20);

        ld1.extend_from_slice(&0_i16.to_le_bytes());
        ld1.push(0x04);
        ld1.extend_from_slice(&0x8028_u16.to_le_bytes()); // compressed signed payload size 20
        ld1.extend_from_slice(&nested_payload);
        ld1.extend_from_slice(&(-1_i16).to_le_bytes());

        let record = TopicRecord {
            position: TopicPos(12),
            record_type: TopicRecordType::Table30,
            region: TopicRegion::Scrolling,
            link_data1: ld1,
            link_data2: b"old nested\0".to_vec(),
            plain_text: "old nested".to_owned(),
        };

        let decoded = FormattedRecord::decode(&record).unwrap();
        assert_eq!(decoded.paragraphs.len(), 1);
        assert_eq!(text_runs(&decoded.paragraphs[0])[0].text, "old nested");
        let TableCellContent::Table(nested) = &decoded.table_cells[0].content else {
            panic!("outer old table cell should retain its nested 0x04 table");
        };
        assert_eq!(nested.info.columns.len(), 1);
        assert_eq!(nested.cells.len(), 1);
        assert!(matches!(
            &nested.cells[0].content,
            TableCellContent::Display {
                paragraph_start: 0,
                paragraph_end: 1
            }
        ));
        assert!(decoded.issues.is_empty());
    }

    #[test]
    fn nested_table_cells_are_retained_recursively_and_share_string_order() {
        // Outer modern table: one absolute-width column containing a nested modern table,
        // followed by a normal display cell in the same outer column.  WinHlp32 dispatches the
        // nested 0x23 record back through 0x414F66 before advancing to the following outer cell.
        let mut ld1 = vec![0x20, 0x80, 2]; // top-level TopicSize 16, TopicLength 1
        ld1.extend_from_slice(&[1, 1]); // one outer column, absolute table type
        ld1.extend_from_slice(&432_u16.to_le_bytes());
        ld1.extend_from_slice(&0_u16.to_le_bytes());

        let paragraph_payload = [
            0x00, 0x80, // compressed signed paragraph-leading value 0
            0x01, 0x00, // paragraph id
            0x00, 0x00, // paragraph flags
            0xFF,
        ];

        // Build the nested table payload itself: two columns and one display cell in column 1.
        let mut nested_payload = vec![2, 1];
        for (width, gap) in [(216_u16, 0_u16), (216_u16, 0_u16)] {
            nested_payload.extend_from_slice(&width.to_le_bytes());
            nested_payload.extend_from_slice(&gap.to_le_bytes());
        }
        nested_payload.extend_from_slice(&1_i16.to_le_bytes());
        nested_payload.push(0x20);
        nested_payload.extend_from_slice(&0x800E_u16.to_le_bytes()); // display payload size 7
        nested_payload.push(0); // nested display TopicLength 0
        nested_payload.extend_from_slice(&paragraph_payload);
        nested_payload.extend_from_slice(&(-1_i16).to_le_bytes());
        assert_eq!(nested_payload.len(), 25);

        // Outer cell 0 contains the complete compact nested-table record.  Signed-compressed
        // payload size 25 is (25 + 0x4000) * 2 = 0x8032.
        ld1.extend_from_slice(&0_i16.to_le_bytes());
        ld1.push(0x23);
        ld1.extend_from_slice(&0x8032_u16.to_le_bytes());
        ld1.push(0); // nested table TopicLength 0
        ld1.extend_from_slice(&nested_payload);

        // A normal outer display cell follows. Its text must consume the next LinkData2 string,
        // proving recursive decoding returns at the exact compact payload boundary.
        ld1.extend_from_slice(&0_i16.to_le_bytes());
        ld1.push(0x20);
        ld1.extend_from_slice(&0x800E_u16.to_le_bytes());
        ld1.push(0);
        ld1.extend_from_slice(&paragraph_payload);
        ld1.extend_from_slice(&(-1_i16).to_le_bytes());

        let record = TopicRecord {
            position: TopicPos(12),
            record_type: TopicRecordType::Table,
            region: TopicRegion::Scrolling,
            link_data1: ld1,
            link_data2: b"nested\0outer\0".to_vec(),
            plain_text: "nested outer".to_owned(),
        };

        let decoded = FormattedRecord::decode(&record).unwrap();
        assert_eq!(decoded.paragraphs.len(), 2);
        assert_eq!(text_runs(&decoded.paragraphs[0])[0].text, "nested");
        assert_eq!(text_runs(&decoded.paragraphs[1])[0].text, "outer");
        assert_eq!(decoded.table_cells.len(), 2);

        let TableCellContent::Table(nested) = &decoded.table_cells[0].content else {
            panic!("first outer cell should retain its nested table");
        };
        assert_eq!(nested.info.columns.len(), 2);
        assert_eq!(nested.cells.len(), 1);
        assert_eq!(nested.cells[0].column, 1);
        assert!(matches!(
            &nested.cells[0].content,
            TableCellContent::Display {
                paragraph_start: 0,
                paragraph_end: 1
            }
        ));
        assert!(matches!(
            &decoded.table_cells[1].content,
            TableCellContent::Display {
                paragraph_start: 1,
                paragraph_end: 2
            }
        ));
        assert!(decoded.issues.is_empty());
    }

    #[test]
    fn compact_table_headers_match_compressed_and_fixed_reference_forms() {
        let compressed = [
            0x20_u8,             // modern display
            0x0E, 0x80,         // compressed signed payload size 7
            0x00,                // compressed unsigned TopicLength 0
        ];
        let mut reader = Reader::new(&compressed, "compressed compact header");
        let header = read_compact_record_header(&mut reader).unwrap();
        assert_eq!(header.record_type, TopicRecordType::Display);
        assert_eq!(header.payload_size, 7);
        assert_eq!(reader.remaining(), 0);

        let fixed_old = [
            0x02_u8,             // old topic-header generation
            0x07, 0x00, 0x00, 0x00,
        ];
        let mut reader = Reader::new(&fixed_old, "fixed old compact header");
        let header = read_compact_record_header(&mut reader).unwrap();
        assert_eq!(header.record_type, TopicRecordType::TopicHeader);
        assert_eq!(header.payload_size, 7);
        assert_eq!(reader.remaining(), 0);

        let fixed_modern = [
            0x21_u8,             // modern fixed-size topic-header generation
            0x07, 0x00, 0x00, 0x00,
            0x34, 0x12,         // fixed WORD TopicLength
        ];
        let mut reader = Reader::new(&fixed_modern, "fixed modern compact header");
        let header = read_compact_record_header(&mut reader).unwrap();
        assert_eq!(header.record_type, TopicRecordType::TopicHeader);
        assert_eq!(header.payload_size, 7);
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn compact_graphics_reuse_the_reference_picture_source_rules() {
        let indexed = parse_compact_picture(0x22, vec![0, 0, 7, 0]);
        assert!(matches!(indexed.source, PictureSource::Indexed(7)));

        let negative = parse_compact_picture(0x03, vec![0, 0, 0xFF, 0xFF]);
        assert!(matches!(negative.source, PictureSource::Unsupported(_)));

        let embedded = parse_compact_picture(0x22, vec![1, 0, 0xAA, 0xBB, 0xCC]);
        assert_eq!(
            embedded.source,
            PictureSource::Embedded(vec![0xAA, 0xBB, 0xCC])
        );
    }

    #[test]
    fn hosted_window_record_retains_prefix_and_descriptor_without_execution() {
        let payload = [1_u8, 2, 3, 4, 5, 6, b'!', b'd', b'e', b'm', b'o', 0, 0x99];
        let window = parse_embedded_window(TopicRecordType::EmbeddedWindow, &payload).unwrap();
        assert_eq!(window.raw_prefix, [1, 2, 3, 4, 5, 6]);
        assert_eq!(window.descriptor, "!demo");
        assert_eq!(window.payload_size, payload.len());
    }

    #[test]
    fn nonzero_table_type_has_no_minimum_width_word() {
        let bytes = [
            1_u8, 2, // count, nonzero type
            0x34, 0x12, // width
            0x78, 0x56, // gap before
        ];
        let mut reader = Reader::new(&bytes, "nonzero table header");
        let table = parse_table_header(&mut reader).unwrap();
        assert_eq!(table.table_type, 2);
        assert_eq!(table.minimum_width, None);
        assert_eq!(table.columns[0].width, 0x1234);
        assert_eq!(table.columns[0].gap_before, 0x5678);
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn paragraph_leading_value_accepts_four_byte_compressed_form() {
        // The four-byte compressed signed-long spelling of zero is [1,0,0,0x80].
        // A fixed two-byte skip would read the id/bits from the middle of this field.
        let bytes = [
            0x01, 0x00, 0x00, 0x80,
            0x34, 0x12, // id
            0x00, 0x00, // no optional flags
        ];
        let mut reader = Reader::new(&bytes, "long paragraph prelude");
        let format = parse_paragraph_info(&mut reader).unwrap();
        assert_eq!(format.id, 0x1234);
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn inline_graphic_type_22_uses_compressed_topic_length() {
        // type 0x22, signed-compressed zero-byte payload [0x00, 0x80], TopicLength 258 [5, 2].
        let bytes = [0x22_u8, 0x00, 0x80, 5, 2];
        let mut reader = Reader::new(&bytes, "inline picture test");
        let mut issues = Vec::new();
        let object = parse_inline_compact_object(&mut reader, 0x86, 7, &mut issues)
            .unwrap()
            .expect("graphic object");
        let Inline::Picture(picture) = object else {
            panic!("0x22 compact object should remain a picture");
        };
        assert_eq!(picture.picture_type, 0x22);
        assert_eq!(picture.encoded_size, 0);
        assert_eq!(picture.hotspot_count, Some(258));
        assert!(matches!(picture.source, PictureSource::Unsupported(ref payload) if payload.is_empty()));
        assert!(issues.is_empty());
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn inline_graphic_type_03_preserves_indexed_bitmap_reference() {
        // type 3, signed-compressed payload size 4 [0x08, 0x80], selector 0, bitmap index 7.
        let bytes = [0x03_u8, 0x08, 0x80, 0, 0, 7, 0];
        let mut reader = Reader::new(&bytes, "indexed inline picture test");
        let mut issues = Vec::new();
        let object = parse_inline_compact_object(&mut reader, 0x86, 3, &mut issues)
            .unwrap()
            .expect("graphic object");
        let Inline::Picture(picture) = object else {
            panic!("0x03 compact object should remain a picture");
        };
        assert!(matches!(picture.source, PictureSource::Indexed(7)));
        assert!(picture.image.is_none());
        assert!(issues.is_empty());
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn calc_inline_type_05_is_a_standard_button_not_an_embedded_picture() {
        let descriptor = b"!,AL(\"A_CALC_LIST_EQUIV;A_CALC_KEYB_SEQ\")\0";
        assert_eq!(descriptor.len() + 6, 48);
        let mut bytes = vec![0x05_u8, 0x60, 0x80]; // old hosted-window type, payload size 48
        bytes.extend_from_slice(&[1, 0, 0, 0, 0x38, 0x5C]);
        bytes.extend_from_slice(descriptor);

        let mut reader = Reader::new(&bytes, "CALC inline hosted button");
        let mut issues = Vec::new();
        let object = parse_inline_compact_object(&mut reader, 0x86, 0x123, &mut issues)
            .unwrap()
            .expect("hosted object");
        let Inline::EmbeddedWindow(window) = object else {
            panic!("CALC 0x05 compact object must not be decoded as a picture");
        };
        assert_eq!(window.record_type, TopicRecordType::EmbeddedWindow30);
        assert_eq!(window.raw_prefix, [1, 0, 0, 0, 0x38, 0x5C]);
        assert_eq!(window.descriptor, r#"!,AL("A_CALC_LIST_EQUIV;A_CALC_KEYB_SEQ")"#);
        assert_eq!(
            window.standard_button_parts(),
            Some(("", r#"AL("A_CALC_LIST_EQUIV;A_CALC_KEYB_SEQ")"#))
        );
        assert_eq!(reader.remaining(), 0);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].layout_safe);
    }

    #[test]
    fn modern_inline_type_24_consumes_topic_length_before_hosted_payload() {
        let payload = [0_u8, 0, 0, 0, 0, 0, b'!', b',', b'A', b'L', b'(', b')', 0];
        let size = i32::try_from(payload.len()).unwrap();
        assert!(size < 16_384);
        let encoded = u16::try_from((size + 16_384) * 2).unwrap().to_le_bytes();
        let mut bytes = vec![0x24_u8, encoded[0], encoded[1], 0x00]; // TopicLength = 0
        bytes.extend_from_slice(&payload);

        let mut reader = Reader::new(&bytes, "modern inline hosted button");
        let mut issues = Vec::new();
        let object = parse_inline_compact_object(&mut reader, 0x86, 0, &mut issues)
            .unwrap()
            .expect("hosted object");
        assert!(matches!(object, Inline::EmbeddedWindow(_)));
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn type_six_external_hotspot_reads_window_before_help_file() {
        let window = b"secondary\0";
        let file = b"other.hlp\0";
        let payload_len = 1 + 4 + window.len() + file.len();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&i16::try_from(payload_len).unwrap().to_le_bytes());
        bytes.push(6);
        bytes.extend_from_slice(&321_i32.to_le_bytes());
        bytes.extend_from_slice(window);
        bytes.extend_from_slice(file);
        let mut reader = Reader::new(&bytes, "type-six external hotspot test");
        let hotspot = parse_external_hotspot(&mut reader, 0xEF).unwrap();
        assert!(matches!(
            hotspot.target,
            HotspotTarget::External {
                offset: TopicOffset(321),
                help_file: Some(ref help_file),
                window_name: Some(ref window_name),
                ..
            } if help_file == "other.hlp" && window_name == "secondary"
        ));
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn external_hotspot_length_covers_following_structure_itself() {
        let mut bytes = Vec::new();
        // Type 4 + TOPICOFFSET + NUL-terminated help file = 15 bytes total.
        bytes.extend_from_slice(&15_i16.to_le_bytes());
        bytes.push(4);
        bytes.extend_from_slice(&123_i32.to_le_bytes());
        bytes.extend_from_slice(b"other.hlp\0");
        bytes.push(0xA5); // Sentinel proving the parser consumes exactly SizeOfFollowingStruct.
        let mut reader = Reader::new(&bytes, "external hotspot test");
        let hotspot = parse_external_hotspot(&mut reader, 0xEE).unwrap();
        assert!(matches!(
            hotspot.target,
            HotspotTarget::External {
                offset: TopicOffset(123),
                help_file: Some(ref name),
                ..
            } if name == "other.hlp"
        ));
        assert_eq!(reader.read_u8().unwrap(), 0xA5);
    }

}
