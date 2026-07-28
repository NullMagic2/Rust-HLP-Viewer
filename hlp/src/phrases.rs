//! Phrase dictionaries and `LinkData2` expansion for classic and WinHelp 4 help files.

use crate::compression::lz77_decompress;
use crate::reader::Reader;
use crate::{HlpError, HlpFile, SystemInfo, WinHelpVersion};

const MAX_PHRASES: usize = 1_000_000;
const MAX_PHRASE_IMAGE_SIZE: usize = 64 * 1024 * 1024;
const MAX_EXPANDED_LINK_DATA: usize = 64 * 1024 * 1024;

/// Phrase compression generation selected by streams present in a help file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhraseCompression {
    /// No phrase table is present; `LinkData2` must already be expanded.
    None,
    /// Classic `|Phrases` table used by Windows 3.x help compilers.
    Classic,
    /// Hall compression using `|PhrIndex` and `|PhrImage`.
    Hall,
}

/// Decoded phrase dictionary shared by all topic records in one HLP file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhraseTable {
    kind: PhraseCompression,
    phrases: Vec<Vec<u8>>,
}

impl PhraseTable {
    /// Creates an empty table for uncompressed LinkData2 streams and parser tests.
    pub(crate) fn none() -> Self {
        Self {
            kind: PhraseCompression::None,
            phrases: Vec::new(),
        }
    }

    /// Loads whichever phrase dictionary generation the HLP contains.
    pub(crate) fn load(file: &HlpFile, system: &SystemInfo) -> Result<Self, HlpError> {
        match file.internal_file("|Phrases") {
            Ok(classic) => return parse_classic(classic.content, system),
            Err(HlpError::MissingInternalFile(_)) => {}
            Err(error) => return Err(error),
        }

        let index = file.internal_file("|PhrIndex");
        let image = file.internal_file("|PhrImage");
        match (index, image) {
            (Ok(index), Ok(image)) => parse_hall(index.content, image.content),
            (Err(HlpError::MissingInternalFile(_)), Err(HlpError::MissingInternalFile(_))) => {
                Ok(Self::none())
            }
            (Err(HlpError::MissingInternalFile(_)), Ok(_)) => Err(HlpError::invalid(
                "Hall phrase table",
                "|PhrImage exists without |PhrIndex",
            )),
            (Ok(_), Err(HlpError::MissingInternalFile(_))) => Err(HlpError::invalid(
                "Hall phrase table",
                "|PhrIndex exists without |PhrImage",
            )),
            (Err(error), _) | (_, Err(error)) => Err(error),
        }
    }

    /// Returns which phrase compression generation this table represents.
    pub const fn kind(&self) -> PhraseCompression {
        self.kind
    }

    /// Returns how many dictionary entries were reconstructed.
    pub fn len(&self) -> usize {
        self.phrases.len()
    }

    /// Returns whether there are no dictionary entries.
    pub fn is_empty(&self) -> bool {
        self.phrases.is_empty()
    }

    /// Expands one stored topic `LinkData2` buffer to its advertised decompressed length.
    pub(crate) fn decode_link_data(
        &self,
        stored: &[u8],
        expected_len: usize,
    ) -> Result<Vec<u8>, HlpError> {
        if expected_len > MAX_EXPANDED_LINK_DATA {
            return Err(HlpError::invalid(
                "topic LinkData2",
                format!(
                    "advertised decoded length {expected_len} exceeds safety limit {MAX_EXPANDED_LINK_DATA}"
                ),
            ));
        }
        if expected_len == stored.len() {
            return Ok(stored.to_vec());
        }
        if expected_len < stored.len() {
            if self.kind == PhraseCompression::Hall {
                // Hall files may store an uncompressed LinkData2 prefix followed by unused bytes.
                return Ok(stored[..expected_len].to_vec());
            }
            return Err(HlpError::invalid(
                "topic LinkData2",
                format!(
                    "record stores {} bytes for a {expected_len}-byte payload; trailing unused bytes are only defined for Hall compression",
                    stored.len()
                ),
            ));
        }

        let decoded = match self.kind {
            PhraseCompression::None => {
                return Err(HlpError::invalid(
                    "topic LinkData2",
                    format!(
                        "record advertises {expected_len} decoded bytes but stores only {} and has no phrase table",
                        stored.len()
                    ),
                ));
            }
            PhraseCompression::Classic => decode_classic_link_data(stored, self, expected_len)?,
            PhraseCompression::Hall => decode_hall_link_data(stored, self, expected_len)?,
        };

        if decoded.len() != expected_len {
            return Err(HlpError::invalid(
                "topic LinkData2",
                format!(
                    "phrase expansion produced {} bytes, expected {expected_len}",
                    decoded.len()
                ),
            ));
        }
        Ok(decoded)
    }

    /// Returns one phrase by index with a contextual bounds error.
    fn phrase(&self, index: usize) -> Result<&[u8], HlpError> {
        self.phrases.get(index).map(Vec::as_slice).ok_or_else(|| {
            HlpError::invalid(
                "phrase expansion",
                format!("phrase index {index} exceeds {} entries", self.phrases.len()),
            )
        })
    }
}

/// Parses the older `|Phrases` stream, including its optionally LZ77-compressed image.
fn parse_classic(content: &[u8], system: &SystemInfo) -> Result<PhraseTable, HlpError> {
    let mut reader = Reader::new(content, "|Phrases header");
    let first_word = reader.read_u16()?;

    // Some MediaView (MVB) files add a 0x0800 prefix and 30 reserved bytes.
    // Ordinary HC31/HCW |Phrases streams do not contain that padding.
    let mvb_layout = matches!(system.version, WinHelpVersion::Multimedia) && first_word == 0x0800;
    let count = if mvb_layout {
        usize::from(reader.read_u16()?)
    } else {
        usize::from(first_word)
    };
    if count > MAX_PHRASES {
        return Err(HlpError::invalid(
            "|Phrases",
            format!("phrase count {count} exceeds safety limit {MAX_PHRASES}"),
        ));
    }

    let marker = reader.read_u16()?;
    if marker != 0x0100 {
        return Err(HlpError::InvalidMagic {
            context: "|Phrases",
            expected: 0x0100,
            actual: u32::from(marker),
        });
    }

    let decompressed_size = if system.minor > 16 {
        let size = usize::try_from(reader.read_u32()?).map_err(|_| {
            HlpError::invalid("|Phrases", "decompressed image size does not fit usize")
        })?;
        if size > MAX_PHRASE_IMAGE_SIZE {
            return Err(HlpError::invalid(
                "|Phrases",
                format!(
                    "decompressed image size {size} exceeds safety limit {MAX_PHRASE_IMAGE_SIZE}"
                ),
            ));
        }
        if mvb_layout {
            let _reserved = reader.read_bytes(30)?;
        }
        Some(size)
    } else {
        None
    };

    let mut raw_offsets = Vec::with_capacity(count.saturating_add(1));
    for _ in 0..=count {
        raw_offsets.push(usize::from(reader.read_u16()?));
    }
    let base = *raw_offsets
        .first()
        .ok_or_else(|| HlpError::invalid("|Phrases", "missing offset table"))?;
    let offsets = normalize_offsets(&raw_offsets, base, "|Phrases")?;
    let stored_image = reader.read_bytes(reader.remaining())?;

    let image = match decompressed_size {
        Some(size) => {
            let decoded = lz77_decompress(stored_image, size)?;
            if decoded.len() != size {
                return Err(HlpError::invalid(
                    "|Phrases",
                    format!("LZ77 image decoded to {} bytes, expected {size}", decoded.len()),
                ));
            }
            decoded
        }
        None => {
            if stored_image.len() > MAX_PHRASE_IMAGE_SIZE {
                return Err(HlpError::invalid(
                    "|Phrases",
                    format!(
                        "phrase image size {} exceeds safety limit {MAX_PHRASE_IMAGE_SIZE}",
                        stored_image.len()
                    ),
                ));
            }
            stored_image.to_vec()
        }
    };

    let phrases = slice_phrases(&image, &offsets, "|Phrases")?;
    Ok(PhraseTable {
        kind: PhraseCompression::Classic,
        phrases,
    })
}

/// Parses WinHelp's newer Hall phrase dictionary from `|PhrIndex` and `|PhrImage`.
fn parse_hall(index: &[u8], image: &[u8]) -> Result<PhraseTable, HlpError> {
    let mut reader = Reader::new(index, "|PhrIndex header");
    let magic = reader.read_i32()?;
    if magic != 1 {
        return Err(HlpError::InvalidMagic {
            context: "|PhrIndex",
            expected: 1,
            actual: u32::from_le_bytes(magic.to_le_bytes()),
        });
    }

    let count = nonnegative_usize(reader.read_i32()?, "|PhrIndex phrase count")?;
    if count > MAX_PHRASES {
        return Err(HlpError::invalid(
            "|PhrIndex",
            format!("phrase count {count} exceeds safety limit {MAX_PHRASES}"),
        ));
    }
    let _compressed_size = nonnegative_usize(reader.read_i32()?, "|PhrIndex compressed size")?;
    let image_size = nonnegative_usize(reader.read_i32()?, "|PhrIndex image size")?;
    if image_size > MAX_PHRASE_IMAGE_SIZE {
        return Err(HlpError::invalid(
            "|PhrIndex",
            format!("phrase image size {image_size} exceeds safety limit {MAX_PHRASE_IMAGE_SIZE}"),
        ));
    }
    let image_compressed_size =
        nonnegative_usize(reader.read_i32()?, "|PhrIndex compressed image size")?;
    let _reserved = reader.read_i32()?;
    let bit_count_raw = reader.read_u16()?;
    let _signature = reader.read_u16()?; // Historically 0x4A00, but real files vary.
    // BitCount occupies four bits. The documented decoder is valid for the entire 0..=15
    // range, including zero (where phrase lengths are represented by the unary component only).
    let bit_count = usize::from(bit_count_raw & 0x000F);

    let mut bits = BitReader::new(reader.read_bytes(reader.remaining())?);
    let mut offsets = Vec::with_capacity(count.saturating_add(1));
    offsets.push(0);
    let mut total = 0_usize;
    for _ in 0..count {
        let mut unary = 0_usize;
        while bits.read_bit()? {
            unary = unary
                .checked_add(1)
                .ok_or_else(|| HlpError::invalid("|PhrIndex", "unary length overflow"))?;
        }
        let low = bits.read_bits(bit_count)?;
        let high = unary
            .checked_mul(1_usize << bit_count)
            .ok_or_else(|| HlpError::invalid("|PhrIndex", "phrase length overflow"))?;
        let length = low
            .checked_add(high)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| HlpError::invalid("|PhrIndex", "phrase length overflow"))?;
        total = total
            .checked_add(length)
            .ok_or_else(|| HlpError::invalid("|PhrIndex", "phrase image size overflow"))?;
        if total > image_size {
            return Err(HlpError::invalid(
                "|PhrIndex",
                format!("phrase lengths exceed advertised image size {image_size}"),
            ));
        }
        offsets.push(total);
    }
    if total != image_size {
        return Err(HlpError::invalid(
            "|PhrIndex",
            format!("phrase lengths total {total} bytes, expected image size {image_size}"),
        ));
    }

    if image_compressed_size > MAX_PHRASE_IMAGE_SIZE {
        return Err(HlpError::invalid(
            "|PhrIndex",
            format!(
                "stored phrase image size {image_compressed_size} exceeds safety limit {MAX_PHRASE_IMAGE_SIZE}"
            ),
        ));
    }
    if image_compressed_size == 0 && image_size != 0 {
        return Err(HlpError::invalid(
            "|PhrIndex",
            "non-empty phrase image advertises a zero stored size",
        ));
    }
    let stored_len = image_compressed_size;
    let stored = image.get(..stored_len).ok_or_else(|| {
        HlpError::invalid(
            "|PhrImage",
            format!("advertised stored image size {stored_len} exceeds stream size {}", image.len()),
        )
    })?;
    let decoded = if stored_len == image_size {
        stored.to_vec()
    } else {
        let decoded = lz77_decompress(stored, image_size)?;
        if decoded.len() != image_size {
            return Err(HlpError::invalid(
                "|PhrImage",
                format!("LZ77 image decoded to {} bytes, expected {image_size}", decoded.len()),
            ));
        }
        decoded
    };

    let phrases = slice_phrases(&decoded, &offsets, "|PhrImage")?;
    Ok(PhraseTable {
        kind: PhraseCompression::Hall,
        phrases,
    })
}

/// Converts classic phrase offsets into offsets relative to the phrase image itself.
fn normalize_offsets(
    raw: &[usize],
    base: usize,
    context: &'static str,
) -> Result<Vec<usize>, HlpError> {
    let mut normalized = Vec::with_capacity(raw.len());
    let mut previous = 0_usize;
    for (index, value) in raw.iter().copied().enumerate() {
        let offset = value.checked_sub(base).ok_or_else(|| {
            HlpError::invalid(context, format!("phrase offset {index} precedes image base"))
        })?;
        if index > 0 && offset < previous {
            return Err(HlpError::invalid(
                context,
                format!("phrase offsets decrease at entry {index}"),
            ));
        }
        normalized.push(offset);
        previous = offset;
    }
    Ok(normalized)
}

/// Slices one reconstructed phrase image using a validated N+1 offset table.
fn slice_phrases(
    image: &[u8],
    offsets: &[usize],
    context: &'static str,
) -> Result<Vec<Vec<u8>>, HlpError> {
    if offsets.is_empty() {
        return Ok(Vec::new());
    }
    if offsets.last().copied().unwrap_or(0) > image.len() {
        return Err(HlpError::invalid(
            context,
            "final phrase offset exceeds reconstructed image",
        ));
    }
    let mut phrases = Vec::with_capacity(offsets.len().saturating_sub(1));
    for pair in offsets.windows(2) {
        let phrase = image.get(pair[0]..pair[1]).ok_or_else(|| {
            HlpError::invalid(context, "phrase offset pair exceeds reconstructed image")
        })?;
        phrases.push(phrase.to_vec());
    }
    Ok(phrases)
}

/// Expands the Windows 3.x phrase token scheme used inside topic LinkData2.
fn decode_classic_link_data(
    input: &[u8],
    table: &PhraseTable,
    expected_len: usize,
) -> Result<Vec<u8>, HlpError> {
    let mut output = Vec::with_capacity(expected_len);
    let mut position = 0_usize;
    while position < input.len() && output.len() < expected_len {
        let first = input[position];
        position += 1;
        if first == 0 || first >= 0x10 {
            output.push(first);
            continue;
        }
        let second = *input.get(position).ok_or(HlpError::UnexpectedEof {
            context: "classic phrase token",
        })?;
        position += 1;
        let index = ((usize::from(first) - 1) << 7) + (usize::from(second) >> 1);
        append_capped(&mut output, table.phrase(index)?, expected_len);
        if second & 1 != 0 && output.len() < expected_len {
            output.push(b' ');
        }
    }
    Ok(output)
}

/// Expands Hall-compressed topic text using its compact token families.
fn decode_hall_link_data(
    input: &[u8],
    table: &PhraseTable,
    expected_len: usize,
) -> Result<Vec<u8>, HlpError> {
    let mut output = Vec::with_capacity(expected_len);
    let mut position = 0_usize;
    while position < input.len() && output.len() < expected_len {
        let first = input[position];
        position += 1;
        match first & 0x0F {
            _ if first & 0x01 == 0 => {
                append_capped(&mut output, table.phrase(usize::from(first >> 1))?, expected_len);
            }
            _ if first & 0x03 == 0x01 => {
                let second = *input.get(position).ok_or(HlpError::UnexpectedEof {
                    context: "Hall phrase token",
                })?;
                position += 1;
                let index = (usize::from(first) + 1)
                    .checked_mul(64)
                    .and_then(|base| base.checked_add(usize::from(second)))
                    .ok_or_else(|| HlpError::invalid("Hall phrase token", "phrase index overflow"))?;
                append_capped(&mut output, table.phrase(index)?, expected_len);
            }
            _ if first & 0x07 == 0x03 => {
                let count = usize::from(first >> 3) + 1;
                let end = position.checked_add(count).ok_or_else(|| {
                    HlpError::invalid("Hall literal token", "literal length overflow")
                })?;
                let literal = input.get(position..end).ok_or(HlpError::UnexpectedEof {
                    context: "Hall literal token",
                })?;
                position = end;
                append_capped(&mut output, literal, expected_len);
            }
            0x07 => {
                let count = usize::from(first >> 4) + 1;
                append_repeat(&mut output, b' ', count, expected_len);
            }
            0x0F => {
                let count = usize::from(first >> 4) + 1;
                append_repeat(&mut output, 0, count, expected_len);
            }
            _ => unreachable!("Hall token families cover all byte values"),
        }
    }
    Ok(output)
}

/// Appends bytes without allowing corrupted data to exceed the advertised decoded length.
fn append_capped(output: &mut Vec<u8>, bytes: &[u8], cap: usize) {
    let remaining = cap.saturating_sub(output.len());
    output.extend_from_slice(&bytes[..bytes.len().min(remaining)]);
}

/// Appends a repeated byte while respecting the same output cap.
fn append_repeat(output: &mut Vec<u8>, byte: u8, count: usize, cap: usize) {
    let count = count.min(cap.saturating_sub(output.len()));
    output.extend(std::iter::repeat(byte).take(count));
}

/// Converts a non-negative signed WinHelp size/count to usize.
fn nonnegative_usize(value: i32, context: &'static str) -> Result<usize, HlpError> {
    if value < 0 {
        return Err(HlpError::invalid(context, format!("negative value {value}")));
    }
    usize::try_from(value).map_err(|_| HlpError::invalid(context, "value does not fit usize"))
}

/// Least-significant-bit-first reader used by Hall's packed phrase-length table.
struct BitReader<'a> {
    bytes: &'a [u8],
    bit_position: usize,
}

impl<'a> BitReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            bit_position: 0,
        }
    }

    fn read_bit(&mut self) -> Result<bool, HlpError> {
        let byte_index = self.bit_position / 8;
        let bit_index = self.bit_position % 8;
        let byte = *self.bytes.get(byte_index).ok_or(HlpError::UnexpectedEof {
            context: "|PhrIndex bitstream",
        })?;
        self.bit_position += 1;
        Ok(byte & (1 << bit_index) != 0)
    }

    fn read_bits(&mut self, count: usize) -> Result<usize, HlpError> {
        let mut value = 0_usize;
        for bit in 0..count {
            if self.read_bit()? {
                value |= 1_usize << bit;
            }
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(kind: PhraseCompression, phrases: &[&[u8]]) -> PhraseTable {
        PhraseTable {
            kind,
            phrases: phrases.iter().map(|item| item.to_vec()).collect(),
        }
    }

    #[test]
    fn classic_phrase_token_expands_optional_space() {
        let phrases = table(PhraseCompression::Classic, &[b"Hello"]);
        assert_eq!(
            phrases.decode_link_data(&[0x01, 0x01, b'X'], 7).unwrap(),
            b"Hello X"
        );
    }

    #[test]
    fn hall_small_phrase_and_literal_tokens_expand() {
        let phrases = table(PhraseCompression::Hall, &[b"Help"]);
        // 0x00 => phrase 0. 0x0B => two literal bytes follow.
        assert_eq!(
            phrases.decode_link_data(&[0x00, 0x0B, b'!', b'!'], 6).unwrap(),
            b"Help!!"
        );
    }

    #[test]
    fn hall_space_and_nul_runs_expand() {
        let phrases = table(PhraseCompression::Hall, &[]);
        assert_eq!(
            phrases.decode_link_data(&[0x17, 0x0F], 3).unwrap(),
            &[b' ', b' ', 0]
        );
    }


    #[test]
    fn hall_uncompressed_link_data_can_have_unused_tail_bytes() {
        let phrases = table(PhraseCompression::Hall, &[]);
        assert_eq!(
            phrases.decode_link_data(b"TextJunk", 4).unwrap(),
            b"Text"
        );
    }

    #[test]
    fn classic_link_data_rejects_undefined_unused_tail_bytes() {
        let phrases = table(PhraseCompression::Classic, &[]);
        assert!(phrases.decode_link_data(b"TextJunk", 4).is_err());
    }

    #[test]
    fn parses_normal_hc31_classic_phrase_layout_without_reserved_padding() {
        let system = SystemInfo {
            minor: 21,
            major: 1,
            version: WinHelpVersion::Windows31,
            generation_timestamp: 0,
            flags: 0,
            compression: crate::Compression::None,
            topic_block_size: 4096,
            topic_decompressed_block_size: 4084,
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
        // One phrase. Offsets are relative to the beginning of the offset table image.
        // 0x00 marks eight literal LZ77 operations; only two are needed.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&0x0100_u16.to_le_bytes());
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&4_u16.to_le_bytes());
        bytes.extend_from_slice(&6_u16.to_le_bytes());
        bytes.extend_from_slice(&[0x00, b'H', b'i']);
        let table = parse_classic(&bytes, &system).unwrap();
        assert_eq!(table.phrase(0).unwrap(), b"Hi");
    }

    #[test]
    fn parses_mvb_classic_phrase_layout_with_reserved_padding() {
        let system = SystemInfo {
            minor: 27,
            major: 1,
            version: WinHelpVersion::Multimedia,
            generation_timestamp: 0,
            flags: 0,
            compression: crate::Compression::None,
            topic_block_size: 4096,
            topic_decompressed_block_size: 4084,
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
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x0800_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&0x0100_u16.to_le_bytes());
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&[0_u8; 30]);
        bytes.extend_from_slice(&4_u16.to_le_bytes());
        bytes.extend_from_slice(&6_u16.to_le_bytes());
        bytes.extend_from_slice(&[0x00, b'H', b'i']);
        let table = parse_classic(&bytes, &system).unwrap();
        assert_eq!(table.phrase(0).unwrap(), b"Hi");
    }

    #[test]
    fn hall_two_byte_phrase_token_uses_documented_64_multiplier() {
        let phrases_vec: Vec<Vec<u8>> = (0..400)
            .map(|index| format!("P{index}").into_bytes())
            .collect();
        let phrases = PhraseTable {
            kind: PhraseCompression::Hall,
            phrases: phrases_vec,
        };
        // ch=0x01 => (ch + 1) * 64 + next = 128 + next.
        assert_eq!(
            phrases.decode_link_data(&[0x01, 0x01], 4).unwrap(),
            b"P129"
        );
    }


    #[test]
    fn hall_phrase_index_accepts_zero_bit_count() {
        let mut index = Vec::new();
        index.extend_from_slice(&1_i32.to_le_bytes()); // magic
        index.extend_from_slice(&1_i32.to_le_bytes()); // one phrase
        index.extend_from_slice(&1_i32.to_le_bytes()); // compressed-size metadata
        index.extend_from_slice(&1_i32.to_le_bytes()); // decoded image size
        index.extend_from_slice(&1_i32.to_le_bytes()); // stored image size
        index.extend_from_slice(&0_i32.to_le_bytes()); // reserved
        index.extend_from_slice(&0_u16.to_le_bytes()); // BitCount = 0
        index.extend_from_slice(&0x4A00_u16.to_le_bytes());
        index.push(0); // unary terminator; phrase length is the initial value 1

        let table = parse_hall(&index, b"A").unwrap();
        assert_eq!(table.phrase(0).unwrap(), b"A");
    }

    #[test]
    fn bit_reader_is_lsb_first() {
        let mut bits = BitReader::new(&[0b1010_0101]);
        assert!(bits.read_bit().unwrap());
        assert!(!bits.read_bit().unwrap());
        assert_eq!(bits.read_bits(3).unwrap(), 1);
    }
}
