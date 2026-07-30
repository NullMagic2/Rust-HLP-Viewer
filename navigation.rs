//! Character decoding helpers shared by legacy WinHelp structures.

use encoding_rs::{
    BIG5, EUC_KR, GBK, MACINTOSH, SHIFT_JIS, WINDOWS_874, WINDOWS_1250, WINDOWS_1251,
    WINDOWS_1253, WINDOWS_1254, WINDOWS_1255, WINDOWS_1256, WINDOWS_1257, WINDOWS_1258,
    Encoding,
};

/// Decodes one Windows-1252 byte slice through the parser's compact legacy table.
pub(crate) fn decode_windows_1252(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| cp1252_char(*byte)).collect()
}

/// Decodes bytes only up to their first NUL terminator.
pub(crate) fn decode_c_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
    decode_windows_1252(&bytes[..end])
}

/// Decodes a LinkData2 byte slice using the GDI charset selected for the active WinHelp face.
///
/// KB917607's Unicode output path obtains the selected GDI charset, resolves it to a Windows code
/// page with `TranslateCharsetInfo`, then calls `MultiByteToWideChar` (`0x416EB6..0x416FF8`). The
/// mappings below reproduce the Win32 charset constants that have stable historical code pages.
/// `JOHAB_CHARSET` is CP1361 and is decoded by the compact lookup table generated from the
/// standard Johab mapping. `OEM_CHARSET` remains a host-defined boundary: KB917607 asks GDI for
/// the selected font charset and translates that through the machine's charset/code-page tables,
/// so the HLP bytes alone do not identify one portable OEM code page.
pub(crate) fn decode_windows_charset(bytes: &[u8], charset: Option<u8>) -> String {
    match charset {
        Some(0x4D) => decode_with(MACINTOSH, bytes),
        Some(0x80) => decode_with(SHIFT_JIS, bytes),
        Some(0x81) => decode_with(EUC_KR, bytes),
        Some(0x82) => decode_johab(bytes),
        Some(0x86) => decode_with(GBK, bytes),
        Some(0x88) => decode_with(BIG5, bytes),
        Some(0xA1) => decode_with(WINDOWS_1253, bytes),
        Some(0xA2) => decode_with(WINDOWS_1254, bytes),
        Some(0xA3) => decode_with(WINDOWS_1258, bytes),
        Some(0xB1) => decode_with(WINDOWS_1255, bytes),
        Some(0xB2) => decode_with(WINDOWS_1256, bytes),
        Some(0xBA) => decode_with(WINDOWS_1257, bytes),
        Some(0xCC) => decode_with(WINDOWS_1251, bytes),
        Some(0xDE) => decode_with(WINDOWS_874, bytes),
        Some(0xEE) => decode_with(WINDOWS_1250, bytes),
        Some(0xFF) => decode_oem_charset(bytes),
        _ => decode_windows_1252(bytes),
    }
}

fn decode_with(encoding: &'static Encoding, bytes: &[u8]) -> String {
    let (decoded, _had_errors) = encoding.decode_without_bom_handling(bytes);
    decoded.into_owned()
}

/// Decodes `OEM_CHARSET` through the active Windows OEM code page when the parser is running on
/// Windows, matching KB917607's deliberate delegation to the host charset database.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn decode_oem_charset(bytes: &[u8]) -> String {
    use windows_sys::Win32::Globalization::MultiByteToWideChar;

    const CP_OEMCP: u32 = 1;

    if bytes.is_empty() {
        return String::new();
    }
    let Ok(input_len) = i32::try_from(bytes.len()) else {
        return decode_windows_1252(bytes);
    };

    // SAFETY: the input pointer/length refer to the live `bytes` slice. The first call supplies a
    // null output buffer exactly as documented to obtain the UTF-16 length. The second call writes
    // only into a Vec allocated to that returned length. No pointer is retained after either call.
    unsafe {
        let required = MultiByteToWideChar(
            CP_OEMCP,
            0,
            bytes.as_ptr(),
            input_len,
            std::ptr::null_mut(),
            0,
        );
        let Ok(required_len) = usize::try_from(required) else {
            return decode_windows_1252(bytes);
        };
        if required_len == 0 {
            return decode_windows_1252(bytes);
        }
        let mut wide = vec![0_u16; required_len];
        let written = MultiByteToWideChar(
            CP_OEMCP,
            0,
            bytes.as_ptr(),
            input_len,
            wide.as_mut_ptr(),
            required,
        );
        let Ok(written_len) = usize::try_from(written) else {
            return decode_windows_1252(bytes);
        };
        if written_len == 0 {
            return decode_windows_1252(bytes);
        }
        String::from_utf16_lossy(&wide[..written_len])
    }
}

/// Non-Windows builds have no authoritative Windows OEM code-page database. Keep the parser
/// deterministic rather than pretending that one particular OEM page is universal.
#[cfg(not(target_os = "windows"))]
fn decode_oem_charset(bytes: &[u8]) -> String {
    decode_windows_1252(bytes)
}

const JOHAB_CP1361_TABLE: &[u8] = include_bytes!("cp1361.bin");

/// Decodes Windows CP1361 / JOHAB_CHARSET without depending on host locale state.
///
/// The mapping asset is sorted by the encoded big-endian two-byte value and stores each entry as
/// `u16 encoded` + `u16 Unicode BMP scalar`, four bytes per record. CP1361's single-byte range is
/// ASCII; all non-ASCII characters use a two-byte sequence. Invalid/truncated sequences become
/// U+FFFD, matching the viewer's existing lossy legacy-decoding policy.
fn decode_johab(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len());
    let mut index = 0_usize;
    while index < bytes.len() {
        let first = bytes[index];
        if first < 0x80 {
            output.push(char::from(first));
            index += 1;
            continue;
        }
        let Some(&second) = bytes.get(index + 1) else {
            output.push('\u{FFFD}');
            break;
        };
        let encoded = u16::from_be_bytes([first, second]);
        output.push(johab_scalar(encoded).unwrap_or('\u{FFFD}'));
        index += 2;
    }
    output
}

fn johab_scalar(encoded: u16) -> Option<char> {
    let entries = JOHAB_CP1361_TABLE.len() / 4;
    let mut low = 0_usize;
    let mut high = entries;
    while low < high {
        let middle = low + (high - low) / 2;
        let base = middle * 4;
        let key = u16::from_be_bytes([
            JOHAB_CP1361_TABLE[base],
            JOHAB_CP1361_TABLE[base + 1],
        ]);
        if key < encoded {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    if low >= entries {
        return None;
    }
    let base = low * 4;
    let key = u16::from_be_bytes([
        JOHAB_CP1361_TABLE[base],
        JOHAB_CP1361_TABLE[base + 1],
    ]);
    if key != encoded {
        return None;
    }
    let scalar = u16::from_le_bytes([
        JOHAB_CP1361_TABLE[base + 2],
        JOHAB_CP1361_TABLE[base + 3],
    ]);
    char::from_u32(u32::from(scalar))
}

/// Maps one Windows-1252 byte to its Unicode scalar value.
const fn cp1252_char(byte: u8) -> char {
    match byte {
        0x80 => '\u{20AC}',
        0x82 => '\u{201A}',
        0x83 => '\u{0192}',
        0x84 => '\u{201E}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02C6}',
        0x89 => '\u{2030}',
        0x8A => '\u{0160}',
        0x8B => '\u{2039}',
        0x8C => '\u{0152}',
        0x8E => '\u{017D}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201C}',
        0x94 => '\u{201D}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02DC}',
        0x99 => '\u{2122}',
        0x9A => '\u{0161}',
        0x9B => '\u{203A}',
        0x9C => '\u{0153}',
        0x9E => '\u{017E}',
        0x9F => '\u{0178}',
        _ => byte as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_cp1252_punctuation() {
        assert_eq!(decode_windows_1252(&[0x93, b'H', b'i', 0x94]), "“Hi”");
    }

    #[test]
    fn decodes_hebrew_and_arabic_reference_charsets() {
        assert_eq!(decode_windows_charset(&[0xF9, 0xEC, 0xE5, 0xED], Some(0xB1)), "שלום");
        assert_eq!(decode_windows_charset(&[0xE3, 0xD1, 0xCD, 0xC8, 0xC7], Some(0xB2)), "مرحبا");
    }

    #[test]
    fn decodes_single_byte_windows_charsets() {
        assert_eq!(decode_windows_charset(&[0xC3, 0xE5, 0xE9, 0xDC], Some(0xA1)), "Γειά");
        assert_eq!(decode_windows_charset(&[0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2], Some(0xCC)), "Привет");
        assert_eq!(decode_windows_charset(&[0xA1, 0xE8, 0xE9], Some(0xEE)), "ˇčé");
    }

    #[test]
    fn decodes_windows_dbcs_charsets() {
        assert_eq!(decode_windows_charset(&[0x82, 0xB1, 0x82, 0xF1], Some(0x80)), "こん");
        assert_eq!(decode_windows_charset(&[0xC4, 0xE3, 0xBA, 0xC3], Some(0x86)), "你好");
        assert_eq!(decode_windows_charset(&[0xA7, 0x41, 0xA6, 0x6E], Some(0x88)), "你好");
    }

    #[test]
    fn decodes_johab_cp1361() {
        assert_eq!(decode_windows_charset(&[0xD0, 0x65, 0x8A, 0x82], Some(0x82)), "한국");
        assert_eq!(decode_windows_charset(&[0x88, 0x61, 0x90, 0x61, 0x94, 0x61], Some(0x82)), "가나다");
    }

    #[test]
    fn invalid_johab_sequence_is_lossy_but_bounded() {
        assert_eq!(decode_windows_charset(&[0x80], Some(0x82)), "�");
    }

    #[test]
    fn oem_charset_keeps_ascii_stable_on_every_host() {
        assert_eq!(decode_windows_charset(b"OEM text", Some(0xFF)), "OEM text");
    }
}
