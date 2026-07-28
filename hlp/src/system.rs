//! Parser for the mandatory `|SYSTEM` metadata stream.

use crate::encoding::{decode_c_string, decode_windows_1252};
use crate::font::Rgb;
use crate::reader::Reader;
use crate::HlpError;

const SYSTEM_MAGIC: u16 = 0x036C;

/// Known compiler generations encoded by the `|SYSTEM` minor version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinHelpVersion {
    /// HC30 / Windows 3.0 generation.
    Windows30,
    /// HC31 / Windows 3.1 generation.
    Windows31,
    /// WMVC/MMVC multimedia viewer generation.
    Multimedia,
    /// MVC or HCW 4.0 / Windows 95 generation.
    Windows95,
    /// Unrecognized minor version retained for diagnostics.
    Unknown(u16),
}

/// Topic compression mode inferred from the `|SYSTEM` version and flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Topic blocks are stored without LZ77 compression.
    None,
    /// Topic blocks use the classic WinHelp LZ77 coding.
    Lz77,
    /// Flags were not recognized; parsing can still expose metadata.
    Unknown(u16),
}


/// One `[WINDOWS]` definition embedded in a modern `|SYSTEM` stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowDefinition {
    pub raw_flags: u16,
    pub window_type: Option<String>,
    pub name: Option<String>,
    pub caption: Option<String>,
    pub x: Option<i16>,
    pub y: Option<i16>,
    pub width: Option<i16>,
    pub height: Option<i16>,
    pub maximize: bool,
    pub raw_maximize_style: i16,
    pub scrolling_color: Option<Rgb>,
    pub non_scrolling_color: Option<Rgb>,
    pub always_on_top: bool,
    pub auto_size_height: bool,
}

/// One unhandled record retained from a newer `|SYSTEM` stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSystemRecord {
    /// Numeric WinHelp record type.
    pub record_type: u16,
    /// Original payload bytes.
    pub data: Vec<u8>,
}

/// High-level metadata decoded from the HLP `|SYSTEM` internal file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemInfo {
    /// Raw minor format version.
    pub minor: u16,
    /// Raw major format version, conventionally 1.
    pub major: u16,
    /// Recognized WinHelp compiler generation.
    pub version: WinHelpVersion,
    /// Original generation timestamp field, measured from 1980-01-01 by WinHelp.
    pub generation_timestamp: u32,
    /// Raw compression/options flags.
    pub flags: u16,
    /// Topic compression inferred from the raw version and flags.
    pub compression: Compression,
    /// Physical `|TOPIC` block size stored in the HLP stream.
    pub topic_block_size: usize,
    /// Size of the per-block topic data buffer after optional LZ77 decompression.
    pub topic_decompressed_block_size: usize,
    /// Display title when present.
    pub title: Option<String>,
    /// Copyright text when present.
    pub copyright: Option<String>,
    /// Default contents topic offset when present.
    pub contents_topic_offset: Option<i32>,
    /// Startup macros retained as text; execution policy belongs to the viewer shell.
    pub config_macros: Vec<String>,
    /// Associated contents filename from a CNT system record when present.
    pub contents_file: Option<String>,
    /// Locale identifier carried in the 10-byte `|SYSTEM` record type 9.
    /// WinHlp32 masks this with 0x03FF before selecting its Arabic/Hebrew reorder path.
    pub locale_id: Option<u16>,
    /// Per-face GDI charset bytes from modern `|SYSTEM` record type 11.
    ///
    /// KB917607 WinHlp32 allocates exactly the record payload length and later indexes this
    /// table with the `|FONT` descriptor's face index. It is therefore not one 16-bit charset.
    pub font_charsets: Vec<u8>,
    /// Secondary/main window definitions from `[WINDOWS]` records.
    pub windows: Vec<WindowDefinition>,
    /// Records not yet interpreted by this milestone.
    pub unknown_records: Vec<RawSystemRecord>,
}

impl SystemInfo {
    /// Parses one complete `|SYSTEM` stream.
    pub fn parse(content: &[u8]) -> Result<Self, HlpError> {
        let mut reader = Reader::new(content, "|SYSTEM header");
        let magic = reader.read_u16()?;
        if magic != SYSTEM_MAGIC {
            return Err(HlpError::InvalidMagic {
                context: "|SYSTEM",
                expected: u32::from(SYSTEM_MAGIC),
                actual: u32::from(magic),
            });
        }

        let minor = reader.read_u16()?;
        let major = reader.read_u16()?;
        let generation_timestamp = reader.read_u32()?;
        let flags = reader.read_u16()?;
        let version = version_from_minor(minor);
        let (compression, topic_block_size, topic_decompressed_block_size) = compression_from(minor, flags);

        let mut result = Self {
            minor,
            major,
            version,
            generation_timestamp,
            flags,
            compression,
            topic_block_size,
            topic_decompressed_block_size,
            title: None,
            copyright: None,
            contents_topic_offset: None,
            config_macros: Vec::new(),
            contents_file: None,
            locale_id: None,
            font_charsets: Vec::new(),
            windows: Vec::new(),
            unknown_records: Vec::new(),
        };

        if minor <= 16 {
            if reader.remaining() > 0 {
                result.title = Some(decode_windows_1252(reader.read_c_string()?));
            }
            return Ok(result);
        }

        while reader.remaining() > 0 {
            if reader.remaining() < 4 {
                return Err(HlpError::invalid(
                    "|SYSTEM records",
                    "trailing bytes are too short for a record header",
                ));
            }
            let record_type = reader.read_u16()?;
            let data_size = usize::from(reader.read_u16()?);
            let data = reader.read_bytes(data_size)?;
            apply_record(&mut result, record_type, data)?;
        }

        Ok(result)
    }
}

/// Maps the compiler's numeric minor format version to a stable project enum.
const fn version_from_minor(minor: u16) -> WinHelpVersion {
    match minor {
        15 => WinHelpVersion::Windows30,
        21 => WinHelpVersion::Windows31,
        27 => WinHelpVersion::Multimedia,
        33 => WinHelpVersion::Windows95,
        other => WinHelpVersion::Unknown(other),
    }
}

/// Infers compression and topic block size using the classic WinHelp rules.
const fn compression_from(minor: u16, flags: u16) -> (Compression, usize, usize) {
    if minor <= 16 {
        return (Compression::None, 2048, 2036);
    }
    match flags {
        0 => (Compression::None, 4096, 4084),
        4 => (Compression::Lz77, 4096, 16384),
        8 => (Compression::Lz77, 2048, 16384),
        other => (Compression::Unknown(other), 4096, 4084),
    }
}

/// Applies a recognized modern `|SYSTEM` record and preserves everything else verbatim.
fn apply_record(
    system: &mut SystemInfo,
    record_type: u16,
    data: &[u8],
) -> Result<(), HlpError> {
    match record_type {
        1 => system.title = Some(decode_record_string(data)),
        2 => system.copyright = Some(decode_record_string(data)),
        3 => {
            if data.len() < 4 {
                return Err(HlpError::invalid(
                    "|SYSTEM CONTENTS record",
                    "record is shorter than a topic offset",
                ));
            }
            system.contents_topic_offset = Some(i32::from_le_bytes([
                data[0], data[1], data[2], data[3],
            ]));
        }
        4 => system.config_macros.push(decode_record_string(data)),
        6 if system.version != WinHelpVersion::Multimedia => match parse_window_definition(data) {
            Ok(window) => system.windows.push(window),
            Err(_) => system.unknown_records.push(RawSystemRecord {
                record_type,
                data: data.to_vec(),
            }),
        },
        // KB917607 record-dispatch case 9 accepts exactly ten bytes at 0x42CE5B and copies
        // them to +0x1DC. The final WORD at +0x1E4 is later resolved as the locale ID.
        9 if data.len() == 10 => {
            system.locale_id = Some(u16::from_le_bytes([data[8], data[9]]));
        }
        9 => system.unknown_records.push(RawSystemRecord {
            record_type,
            data: data.to_vec(),
        }),
        10 => system.contents_file = Some(decode_record_string(data)),
        // KB917607 0x42CE6C..0x42CE7E allocates data_size + 1 bytes for record 11 and
        // copies the complete payload. Font creation/RTL detection later reads one byte at
        // [table + face_index] (0x411EA7..0x411ED4 / 0x415F6C..0x415F94).
        11 => system.font_charsets = data.to_vec(),
        _ => system.unknown_records.push(RawSystemRecord {
            record_type,
            data: data.to_vec(),
        }),
    }
    Ok(())
}

/// Parses the 90-byte HC31/HCW window record used by ordinary WinHelp files.
fn parse_window_definition(data: &[u8]) -> Result<WindowDefinition, HlpError> {
    if data.len() < 90 {
        return Err(HlpError::invalid(
            "|SYSTEM WINDOW record",
            format!("record is {} bytes; HC31/HCW window needs at least 90", data.len()),
        ));
    }
    let mut reader = Reader::new(data, "|SYSTEM WINDOW record");
    let flags = reader.read_u16()?;
    let window_type_raw = reader.read_bytes(10)?;
    let name_raw = reader.read_bytes(9)?;
    let caption_raw = reader.read_bytes(51)?;
    let x = reader.read_i16()?;
    let y = reader.read_i16()?;
    let width = reader.read_i16()?;
    let height = reader.read_i16()?;
    let maximize_style = reader.read_i16()?;
    let scrolling_raw = reader.read_u32()?;
    let non_scrolling_raw = reader.read_u32()?;

    Ok(WindowDefinition {
        raw_flags: flags,
        window_type: flag_string(flags, 0, window_type_raw),
        name: flag_string(flags, 1, name_raw),
        caption: flag_string(flags, 2, caption_raw),
        x: bit(flags, 3).then_some(x),
        y: bit(flags, 4).then_some(y),
        width: bit(flags, 5).then_some(width),
        height: bit(flags, 6).then_some(height),
        maximize: bit(flags, 7),
        raw_maximize_style: maximize_style,
        scrolling_color: bit(flags, 8).then_some(colorref(scrolling_raw)),
        non_scrolling_color: bit(flags, 9).then_some(colorref(non_scrolling_raw)),
        always_on_top: bit(flags, 10),
        auto_size_height: bit(flags, 11),
    })
}

const fn bit(flags: u16, index: u32) -> bool {
    flags & (1_u16 << index) != 0
}

fn flag_string(flags: u16, index: u32, bytes: &[u8]) -> Option<String> {
    bit(flags, index).then(|| decode_c_string(bytes))
}

/// Converts Win32 COLORREF (0x00BBGGRR) into the renderer's RGB tuple.
const fn colorref(value: u32) -> Rgb {
    Rgb {
        red: (value & 0xFF) as u8,
        green: ((value >> 8) & 0xFF) as u8,
        blue: ((value >> 16) & 0xFF) as u8,
    }
}

/// Decodes a record string, ignoring bytes after its first NUL terminator.
fn decode_record_string(data: &[u8]) -> String {
    decode_c_string(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ensures Windows 3.1 records are decoded and unknown records are retained.
    #[test]
    fn parses_modern_system_records() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&SYSTEM_MAGIC.to_le_bytes());
        bytes.extend_from_slice(&21_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&123_u32.to_le_bytes());
        bytes.extend_from_slice(&4_u16.to_le_bytes());
        push_record(&mut bytes, 1, b"Example Help\0");
        push_record(&mut bytes, 3, &0x1234_i32.to_le_bytes());
        push_record(&mut bytes, 4, b"BrowseButtons()\0");
        let mut locale = [0_u8; 10];
        locale[8..10].copy_from_slice(&0x040D_u16.to_le_bytes());
        push_record(&mut bytes, 9, &locale);
        push_record(&mut bytes, 11, &[0x00, 0xB1, 0xB2]);
        push_record(&mut bytes, 99, &[1, 2, 3]);

        let info = SystemInfo::parse(&bytes).expect("synthetic |SYSTEM should parse");
        assert_eq!(info.version, WinHelpVersion::Windows31);
        assert_eq!(info.compression, Compression::Lz77);
        assert_eq!(info.topic_block_size, 4096);
        assert_eq!(info.title.as_deref(), Some("Example Help"));
        assert_eq!(info.contents_topic_offset, Some(0x1234));
        assert_eq!(info.config_macros, ["BrowseButtons()"]);
        assert_eq!(info.locale_id, Some(0x040D));
        assert_eq!(info.font_charsets, [0x00, 0xB1, 0xB2]);
        assert_eq!(info.unknown_records.len(), 1);
    }

    #[test]
    fn parses_standard_window_definition() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&SYSTEM_MAGIC.to_le_bytes());
        bytes.extend_from_slice(&21_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());

        let mut window = vec![0_u8; 90];
        let flags = 0x0002_u16 | 0x0004 | 0x0008 | 0x0010 | 0x0020 | 0x0040 | 0x0100 | 0x0200;
        window[0..2].copy_from_slice(&flags.to_le_bytes());
        window[12..21].copy_from_slice(b"secondary");
        window[21..35].copy_from_slice(b"Reference Help");
        window[72..74].copy_from_slice(&100_i16.to_le_bytes());
        window[74..76].copy_from_slice(&120_i16.to_le_bytes());
        window[76..78].copy_from_slice(&700_i16.to_le_bytes());
        window[78..80].copy_from_slice(&500_i16.to_le_bytes());
        window[82..86].copy_from_slice(&0x00332211_u32.to_le_bytes());
        window[86..90].copy_from_slice(&0x00665544_u32.to_le_bytes());
        push_record(&mut bytes, 6, &window);

        let info = SystemInfo::parse(&bytes).expect("WINDOW record should parse");
        let parsed = &info.windows[0];
        assert_eq!(parsed.name.as_deref(), Some("secondary"));
        assert_eq!(parsed.caption.as_deref(), Some("Reference Help"));
        assert_eq!(parsed.width, Some(700));
        assert_eq!(parsed.scrolling_color, Some(Rgb { red: 0x11, green: 0x22, blue: 0x33 }));
        assert_eq!(parsed.non_scrolling_color, Some(Rgb { red: 0x44, green: 0x55, blue: 0x66 }));
    }

    /// Appends one length-prefixed `|SYSTEM` record to a synthetic stream.
    fn push_record(target: &mut Vec<u8>, record_type: u16, data: &[u8]) {
        target.extend_from_slice(&record_type.to_le_bytes());
        target.extend_from_slice(
            &u16::try_from(data.len())
                .expect("test record length fits u16")
                .to_le_bytes(),
        );
        target.extend_from_slice(data);
    }
}
