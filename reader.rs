//! Parser for the internal `|FONT` stream used by WinHelp display records.
//!
//! The KB917607 WinHlp32 reference indexes every font descriptor with an 11-byte stride.
//! Compiler generation changes the fixed face-name slot width (20 bytes before HCW 4.0,
//! 32 bytes for minor version 33), not the descriptor layout. Modern per-face GDI charset
//! bytes live in `|SYSTEM` record 11 and are applied after this stream is decoded.

use crate::encoding::decode_windows_1252;
use crate::reader::Reader;
use crate::{HlpError, HlpFile};

const FONT_DESCRIPTOR_SIZE: usize = 11;
const LEGACY_FACE_NAME_SIZE: usize = 20;
const HCW40_FACE_NAME_SIZE: usize = 32;
const MAX_FACES: usize = 4096;
const MAX_DESCRIPTORS: usize = 16384;

/// RGB colour stored by WinHelp font descriptors.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Rgb {
    const fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            red: bytes[0],
            green: bytes[1],
            blue: bytes[2],
        }
    }
}

/// Unit used by retained font/format metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontMetric {
    /// `|FONT` size byte: one unit is half a typographic point.
    HalfPoints,
    /// Retained for callers that need to convert an independently sourced twip metric.
    Twips,
}

/// Coarse WinHelp font-family classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HlpFontFamily {
    Modern,
    Roman,
    Swiss,
    Script,
    Decorative,
    Unknown(u8),
}

impl HlpFontFamily {
    const fn from_old(value: u8) -> Self {
        match value {
            1 => Self::Modern,
            2 => Self::Roman,
            3 => Self::Swiss,
            4 => Self::Script,
            5 => Self::Decorative,
            other => Self::Unknown(other),
        }
    }
}

/// One decoded 11-byte font descriptor referenced by LinkData1 command `0x80`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontDescriptor {
    pub face_index: usize,
    pub face_name: String,
    /// Nominal point size multiplied by 20, avoiding floating-point in the format layer.
    pub point_size_twips: i32,
    pub weight: i16,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike_out: bool,
    /// Attribute bit 0x10 is retained losslessly although the verified font builder does not
    /// consume it as a second underline style.
    pub double_underline: bool,
    pub small_caps: bool,
    pub foreground: Rgb,
    pub background: Rgb,
    pub family: HlpFontFamily,
    /// GDI charset byte selected by `|SYSTEM` record 11 for this descriptor's face.
    /// `None` means WinHlp32 would ask GDI to infer the charset from the face name.
    pub charset: Option<u8>,
}

impl FontDescriptor {
    /// Returns a practical integer point size for native GUI font creation, capped to bound
    /// layout/native-font arithmetic for malformed descriptors.
    pub fn point_size(&self) -> i32 {
        let twips = self.point_size_twips.unsigned_abs();
        let rounded = (twips + 10) / 20;
        i32::try_from(rounded.clamp(1, 4_096)).unwrap_or(4_096)
    }

    /// Reports whether the descriptor requests fixed-pitch/monospaced text.
    pub const fn is_fixed_pitch(&self) -> bool {
        matches!(self.family, HlpFontFamily::Modern)
    }

}

/// Complete font table for one HLP file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontTable {
    metric: FontMetric,
    face_name_size: usize,
    locale_id: Option<u16>,
    face_names: Vec<String>,
    descriptors: Vec<FontDescriptor>,
}

impl FontTable {
    /// Parses `|FONT`; returns an empty fallback table when a legacy HLP has no font stream.
    pub(crate) fn load(file: &HlpFile) -> Result<Self, HlpError> {
        let stream = match file.internal_file("|FONT") {
            Ok(stream) => stream,
            Err(HlpError::MissingInternalFile(_)) => return Ok(Self::fallback()),
            Err(error) => return Err(error),
        };
        let mut table = Self::parse(stream.content)?;
        let system = file.system_info()?;
        table.apply_system_metadata(&system.font_charsets, system.locale_id);
        Ok(table)
    }

    /// Returns a fallback table for files whose display records never reference `|FONT`.
    pub fn fallback() -> Self {
        Self {
            metric: FontMetric::HalfPoints,
            face_name_size: LEGACY_FACE_NAME_SIZE,
            locale_id: None,
            face_names: vec!["MS Sans Serif".to_owned()],
            descriptors: vec![FontDescriptor {
                face_index: 0,
                face_name: "MS Sans Serif".to_owned(),
                point_size_twips: 200,
                weight: 400,
                bold: false,
                italic: false,
                underline: false,
                strike_out: false,
                double_underline: false,
                small_caps: false,
                foreground: Rgb { red: 0, green: 0, blue: 0 },
                background: Rgb { red: 255, green: 255, blue: 255 },
                family: HlpFontFamily::Swiss,
                charset: None,
            }],
        }
    }

    /// Parses raw `|FONT` contents.
    ///
    /// KB917607 `0x411E8C..0x411EBC` proves that descriptor indexing is always `index * 11`.
    /// The same routine chooses a 20- or 32-byte face-name stride. We infer that generation
    /// directly from the bounded face-name region, which also lets standalone parser tests work
    /// without a separate `|SYSTEM` object.
    pub fn parse(bytes: &[u8]) -> Result<Self, HlpError> {
        let mut header = Reader::new(bytes, "|FONT header");
        let num_faces = usize::from(header.read_u16()?);
        let num_descriptors = usize::from(header.read_u16()?);
        let face_offset = usize::from(header.read_u16()?);
        let descriptor_offset = usize::from(header.read_u16()?);

        if num_faces > MAX_FACES || num_descriptors > MAX_DESCRIPTORS {
            return Err(HlpError::invalid(
                "|FONT",
                format!("font counts exceed safety limits ({num_faces} faces, {num_descriptors} descriptors)"),
            ));
        }
        if face_offset < 8 || descriptor_offset < face_offset || descriptor_offset > bytes.len() {
            return Err(HlpError::invalid(
                "|FONT",
                format!("invalid face/descriptor offsets {face_offset}/{descriptor_offset}"),
            ));
        }

        let face_name_size = face_name_stride(num_faces, face_offset, descriptor_offset)?;
        let face_names = parse_face_names(
            bytes,
            num_faces,
            face_offset,
            descriptor_offset,
            face_name_size,
        )?;
        let descriptors = parse_descriptors(bytes, &face_names, num_descriptors, descriptor_offset)?;

        Ok(Self {
            metric: FontMetric::HalfPoints,
            face_name_size,
            locale_id: None,
            face_names,
            descriptors,
        })
    }

    /// Applies the `|SYSTEM` metadata that WinHlp32 consults while selecting and ordering fonts.
    pub(crate) fn apply_system_metadata(&mut self, charsets: &[u8], locale_id: Option<u16>) {
        self.locale_id = locale_id;
        for descriptor in &mut self.descriptors {
            let explicit = charsets.get(descriptor.face_index).copied();
            descriptor.charset = match explicit {
                // DEFAULT_CHARSET asks GDI/font mapping to choose. Resolve it deterministically
                // from the authored face/locale when possible so LinkData2 can be decoded before
                // the viewer creates its native font.
                None | Some(0x01) => infer_legacy_charset(&descriptor.face_name, locale_id),
                Some(charset) => Some(charset),
            };
        }
    }

    pub const fn metric(&self) -> FontMetric {
        self.metric
    }

    /// Fixed slot width used by the face-name directory: 20 for HC31/MVB, 32 for HCW 4.0.
    pub const fn face_name_size(&self) -> usize {
        self.face_name_size
    }

    /// True when the HLP locale enters WinHlp32's Arabic/Hebrew line-reordering path.
    pub const fn is_rtl_locale(&self) -> bool {
        match self.locale_id {
            Some(locale) => matches!(locale & 0x03FF, 0x0001 | 0x000D),
            None => false,
        }
    }

    pub fn face_names(&self) -> &[String] {
        &self.face_names
    }

    pub fn descriptors(&self) -> &[FontDescriptor] {
        &self.descriptors
    }

    pub fn descriptor(&self, index: u16) -> Option<&FontDescriptor> {
        self.descriptors.get(usize::from(index))
    }
}

/// Deterministic counterpart to WinHlp32's host-dependent charset fallback.
///
/// The Microsoft executable ultimately asks the selected GDI font for its charset and translates
/// that charset to a code page before `MultiByteToWideChar`. When record 11 is absent there is no
/// portable font database to query during parsing, so use well-known legacy Windows face names
/// first and the HLP LANGID second. Explicit record-11 charsets always win.
fn infer_legacy_charset(face_name: &str, locale_id: Option<u16>) -> Option<u8> {
    let face = face_name.trim().to_ascii_lowercase();
    let named = match face.as_str() {
        "symbol" | "wingdings" | "wingdings 2" | "wingdings 3" => Some(0x02),
        "ms gothic" | "ms pgothic" | "ms mincho" | "ms pmincho" | "meiryo" => Some(0x80),
        "gulim" | "gulimche" | "batang" | "batangche" | "dotum" | "dotumche"
        | "gungsuh" | "gungsuhche" => Some(0x81),
        "simsun" | "nsimsun" | "simhei" | "fangsong" | "kaiti" => Some(0x86),
        "mingliu" | "pmingliu" | "dfkai-sb" => Some(0x88),
        _ => None,
    };
    named.or_else(|| locale_id.and_then(charset_from_lang_id))
}

fn charset_from_lang_id(locale: u16) -> Option<u8> {
    // Some sublanguages choose different Windows code pages despite sharing a primary LANGID.
    match locale {
        0x0404 | 0x0C04 | 0x1404 => return Some(0x88), // Traditional Chinese / Big5
        0x0804 | 0x1004 => return Some(0x86),          // Simplified Chinese / GBK
        // Serbian shares primary LANGID 0x1A with Croatian/Bosnian. Preserve the script-specific
        // Windows code page where the full LANGID is available instead of collapsing everything
        // onto Central European CP1250.
        0x0C1A | 0x1C1A | 0x281A | 0x301A => return Some(0xCC), // Serbian Cyrillic
        0x081A | 0x181A | 0x241A | 0x2C1A => return Some(0xEE), // Serbian/Bosnian Latin
        _ => {}
    }

    match locale & 0x03FF {
        0x0001 => Some(0xB2), // Arabic
        0x0002 | 0x0019 | 0x0022 | 0x0023 | 0x002F => Some(0xCC), // Cyrillic families
        0x0005 | 0x000E | 0x0015 | 0x0018 | 0x001A | 0x001B | 0x001C | 0x0024 => Some(0xEE),
        0x0008 => Some(0xA1), // Greek
        0x000D => Some(0xB1), // Hebrew
        0x0011 => Some(0x80), // Japanese
        0x0012 => Some(0x81), // Korean
        0x001E => Some(0xDE), // Thai
        0x001F => Some(0xA2), // Turkish
        0x0025 | 0x0026 | 0x0027 => Some(0xBA), // Baltic
        0x002A => Some(0xA3), // Vietnamese
        _ => None,
    }
}

fn face_name_stride(count: usize, start: usize, end: usize) -> Result<usize, HlpError> {
    if count == 0 {
        return Ok(LEGACY_FACE_NAME_SIZE);
    }
    let region = end
        .checked_sub(start)
        .ok_or_else(|| HlpError::invalid("|FONT", "face-name range underflow"))?;
    if region % count != 0 {
        return Err(HlpError::invalid(
            "|FONT",
            format!("face-name region {region} is not divisible by {count}"),
        ));
    }
    let stride = region / count;
    if !matches!(stride, LEGACY_FACE_NAME_SIZE | HCW40_FACE_NAME_SIZE) {
        return Err(HlpError::invalid(
            "|FONT",
            format!("unsupported face-name stride {stride}; WinHlp32 uses 20 or 32 bytes"),
        ));
    }
    Ok(stride)
}

fn parse_face_names(
    bytes: &[u8],
    count: usize,
    start: usize,
    end: usize,
    stride: usize,
) -> Result<Vec<String>, HlpError> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let required = count
        .checked_mul(stride)
        .and_then(|size| start.checked_add(size))
        .ok_or_else(|| HlpError::invalid("|FONT", "face-name range overflow"))?;
    if required != end {
        return Err(HlpError::invalid("|FONT", "face-name directory size mismatch"));
    }
    let mut names = Vec::with_capacity(count);
    for index in 0..count {
        let begin = start
            .checked_add(index.checked_mul(stride).ok_or_else(|| HlpError::invalid("|FONT", "face offset overflow"))?)
            .ok_or_else(|| HlpError::invalid("|FONT", "face offset overflow"))?;
        let slot = bytes
            .get(begin..begin + stride)
            .ok_or(HlpError::UnexpectedEof { context: "|FONT face name" })?;
        let text = slot.split(|byte| *byte == 0).next().unwrap_or(slot);
        names.push(decode_windows_1252(text));
    }
    Ok(names)
}

fn parse_descriptors(
    bytes: &[u8],
    faces: &[String],
    count: usize,
    start: usize,
) -> Result<Vec<FontDescriptor>, HlpError> {
    let total = count
        .checked_mul(FONT_DESCRIPTOR_SIZE)
        .ok_or_else(|| HlpError::invalid("|FONT", "descriptor byte count overflow"))?;
    let data = bytes
        .get(start..start.checked_add(total).ok_or_else(|| HlpError::invalid("|FONT", "descriptor range overflow"))?)
        .ok_or(HlpError::UnexpectedEof { context: "|FONT descriptors" })?;
    data.chunks_exact(FONT_DESCRIPTOR_SIZE)
        .map(|chunk| parse_descriptor(chunk, faces))
        .collect()
}

fn face(faces: &[String], index: u16) -> Result<(usize, String), HlpError> {
    let index = usize::from(index);
    let name = faces.get(index).ok_or_else(|| {
        HlpError::invalid("|FONT descriptor", format!("face index {index} exceeds {} names", faces.len()))
    })?;
    Ok((index, name.clone()))
}

fn parse_descriptor(bytes: &[u8], faces: &[String]) -> Result<FontDescriptor, HlpError> {
    let mut reader = Reader::new(bytes, "|FONT descriptor");
    let attributes = reader.read_u8()?;
    let half_points = reader.read_u8()?;
    let family_raw = reader.read_u8()?;
    let face_index = reader.read_u16()?;
    let fg = reader.read_bytes(3)?;
    let bg = reader.read_bytes(3)?;
    let (face_index, face_name) = face(faces, face_index)?;
    Ok(FontDescriptor {
        face_index,
        face_name,
        point_size_twips: i32::from(half_points) * 10,
        weight: if attributes & 0x01 != 0 { 700 } else { 400 },
        bold: attributes & 0x01 != 0,
        italic: attributes & 0x02 != 0,
        underline: attributes & 0x04 != 0,
        strike_out: attributes & 0x08 != 0,
        double_underline: attributes & 0x10 != 0,
        small_caps: attributes & 0x20 != 0,
        foreground: Rgb::from_bytes(fg),
        background: Rgb::from_bytes(bg),
        family: HlpFontFamily::from_old(family_raw),
        charset: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font_stream(face_stride: usize, face_index: u16) -> Vec<u8> {
        let face_offset = 8_u16;
        let descriptor_offset = u16::try_from(8 + face_stride).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&face_offset.to_le_bytes());
        bytes.extend_from_slice(&descriptor_offset.to_le_bytes());
        let mut face = vec![0_u8; face_stride];
        face[..5].copy_from_slice(b"Arial");
        bytes.extend_from_slice(&face);
        bytes.push(0x27); // bold, italic, underline, small caps
        bytes.push(20); // 10 pt
        bytes.push(3); // swiss
        bytes.extend_from_slice(&face_index.to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3, 255, 255, 255]);
        bytes
    }

    #[test]
    fn parses_legacy_20_byte_faces_and_11_byte_descriptor() {
        let table = FontTable::parse(&font_stream(20, 0)).unwrap();
        assert_eq!(table.metric(), FontMetric::HalfPoints);
        assert_eq!(table.face_name_size(), 20);
        assert_eq!(table.face_names(), &["Arial"]);
        let font = &table.descriptors()[0];
        assert_eq!(font.point_size(), 10);
        assert!(font.bold && font.italic && font.underline && font.small_caps);
        assert_eq!(font.foreground, Rgb { red: 1, green: 2, blue: 3 });
        assert!(!font.is_fixed_pitch());
    }

    #[test]
    fn parses_hcw40_32_byte_faces_without_inventing_a_42_byte_descriptor() {
        let table = FontTable::parse(&font_stream(32, 0)).unwrap();
        assert_eq!(table.face_name_size(), 32);
        assert_eq!(table.descriptors().len(), 1);
        assert_eq!(table.descriptors()[0].face_name, "Arial");
    }

    #[test]
    fn face_charset_table_is_applied_by_face_index() {
        let mut table = FontTable::parse(&font_stream(32, 0)).unwrap();
        table.apply_system_metadata(&[0xB1], None);
        assert_eq!(table.descriptors()[0].charset, Some(0xB1));
        assert!(matches!(table.descriptors()[0].charset, Some(0xB1) | Some(0xB2)));
    }

    #[test]
    fn absent_charset_table_uses_hlp_locale_for_international_text() {
        let mut table = FontTable::parse(&font_stream(32, 0)).unwrap();
        table.apply_system_metadata(&[], Some(0x0408));
        assert_eq!(table.descriptors()[0].charset, Some(0xA1));

        table.apply_system_metadata(&[], Some(0x0411));
        assert_eq!(table.descriptors()[0].charset, Some(0x80));

        table.apply_system_metadata(&[], Some(0x0804));
        assert_eq!(table.descriptors()[0].charset, Some(0x86));

        table.apply_system_metadata(&[], Some(0x0C1A));
        assert_eq!(table.descriptors()[0].charset, Some(0xCC));

        table.apply_system_metadata(&[], Some(0x081A));
        assert_eq!(table.descriptors()[0].charset, Some(0xEE));
    }

    #[test]
    fn explicit_charset_overrides_locale_inference() {
        let mut table = FontTable::parse(&font_stream(32, 0)).unwrap();
        table.apply_system_metadata(&[0xCC], Some(0x0408));
        assert_eq!(table.descriptors()[0].charset, Some(0xCC));
    }

    #[test]
    fn default_charset_uses_locale_but_ansi_charset_remains_explicit() {
        let mut table = FontTable::parse(&font_stream(32, 0)).unwrap();
        table.apply_system_metadata(&[0x01], Some(0x0411));
        assert_eq!(table.descriptors()[0].charset, Some(0x80));

        table.apply_system_metadata(&[0x00], Some(0x0411));
        assert_eq!(table.descriptors()[0].charset, Some(0x00));
    }

    #[test]
    fn rejects_descriptor_face_index_outside_table() {
        assert!(FontTable::parse(&font_stream(20, 4)).is_err());
    }

    #[test]
    fn rejects_face_slots_not_used_by_reference_renderer() {
        assert!(FontTable::parse(&font_stream(12, 0)).is_err());
    }
}
