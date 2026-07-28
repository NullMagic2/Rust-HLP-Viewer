//! `|TOPIC` block transformation and high-level topic extraction.

use std::collections::HashSet;

use crate::compression::lz77_decompress;
use crate::encoding::decode_windows_1252;
use crate::phrases::{PhraseCompression, PhraseTable};
use crate::reader::Reader;
use crate::{Compression, HlpError, HlpFile, SystemInfo};

const TOPIC_BLOCK_HEADER_SIZE: usize = 12;
const TOPIC_LINK_HEADER_SIZE: usize = 21;
const MAX_TOPIC_LINKS: usize = 1_000_000;
const MAX_TOPIC_RECORD_SIZE: usize = 64 * 1024 * 1024;

/// A logical byte position in transformed `|TOPIC` data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TopicPos(pub i32);

/// A character/hotspot-oriented WinHelp topic offset used by context and browse maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TopicOffset(pub i32);

/// Stable identifier for a topic within one loaded HLP file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TopicId(pub TopicPos);

/// TOPICLINK record kinds currently understood by the parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopicRecordType {
    /// Windows 3.0 ordinary display information.
    Display30,
    /// Topic metadata/header record (0x02 or the modern 0x21 generation).
    TopicHeader,
    /// Windows 3.0 standalone graphics/special display record (0x03).
    Graphic30,
    /// Windows 3.0 table information.
    Table30,
    /// Windows 3.0 hosted/custom-window record (0x05).
    EmbeddedWindow30,
    /// Windows 3.0 compact record 0x06. The verified WinHlp32 dispatcher does not paint it.
    NoRender30,
    /// Windows 3.1+ ordinary display information.
    Display,
    /// Windows 3.1+ standalone graphics/special display record (0x22).
    Graphic,
    /// Windows 3.1+ table information.
    Table,
    /// Windows 3.1+ hosted/custom-window record (0x24).
    EmbeddedWindow,
    /// A record generation not yet decoded structurally.
    Unknown(u8),
}

impl TopicRecordType {
    pub(crate) const fn from_byte(value: u8) -> Self {
        match value {
            0x01 => Self::Display30,
            0x02 | 0x21 => Self::TopicHeader,
            0x03 => Self::Graphic30,
            0x04 => Self::Table30,
            0x05 => Self::EmbeddedWindow30,
            0x06 => Self::NoRender30,
            0x20 => Self::Display,
            0x22 => Self::Graphic,
            0x23 => Self::Table,
            0x24 => Self::EmbeddedWindow,
            other => Self::Unknown(other),
        }
    }
}

/// Which visual region a topic record belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopicRegion {
    /// The type-2 metadata record that starts a topic.
    Header,
    /// Fixed region above the topic's scrolling viewport.
    NonScrolling,
    /// Main scrolling topic body.
    Scrolling,
    /// Record could not yet be associated with a known visual region.
    Unclassified,
}

/// The 12-byte header stored at the beginning of every physical topic block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopicBlockHeader {
    pub last_topic_link: TopicPos,
    pub first_topic_link: TopicPos,
    pub last_topic_header: TopicPos,
}

/// Diagnostics for one physical/transformed `|TOPIC` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicBlockInfo {
    /// Zero-based physical block number.
    pub index: usize,
    /// Original bytes in this block including its 12-byte header.
    pub physical_size: usize,
    /// Bytes available in its transformed data buffer.
    pub decoded_data_size: usize,
    /// Original block header.
    pub header: TopicBlockHeader,
}

/// One decoded TOPICLINK after phrase expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicRecord {
    /// Logical position at which this record's TOPICLINK begins.
    pub position: TopicPos,
    /// Parsed record kind.
    pub record_type: TopicRecordType,
    /// Visual region inferred from the enclosing topic header.
    pub region: TopicRegion,
    /// Raw LinkData1 bytes retained for later formatting/layout decoding.
    pub link_data1: Vec<u8>,
    /// Fully phrase-expanded LinkData2 bytes.
    pub link_data2: Vec<u8>,
    /// Best-effort visible text for display/table records.
    pub plain_text: String,
}

/// Topic-level metadata carried by a type-2 record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TopicMetadata {
    /// Compiler-advertised byte size of the topic and its internal records.
    pub block_size: i32,
    /// Previous topic in a browse sequence, for Windows 3.1+ files.
    pub browse_back: Option<TopicOffset>,
    /// Next topic in a browse sequence, for Windows 3.1+ files.
    pub browse_forward: Option<TopicOffset>,
    /// Compiler topic number when present.
    pub topic_number: Option<i32>,
    /// Start of the fixed/non-scrolling region.
    pub non_scroll_start: Option<TopicPos>,
    /// Start of the scrolling region.
    pub scroll_start: Option<TopicPos>,
    /// Position of the next type-2 topic header.
    pub next_topic: Option<TopicPos>,
    /// Windows 3.0 previous topic number when present.
    pub previous_topic_number: Option<i32>,
    /// Windows 3.0 next topic number when present.
    pub next_topic_number: Option<i32>,
}

/// One reconstructed WinHelp topic with title, inert macros, regions, and searchable text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Topic {
    pub id: TopicId,
    pub title: String,
    /// Topic macros are retained as strings; execution policy belongs to the viewer shell.
    pub macros: Vec<String>,
    pub metadata: TopicMetadata,
    pub non_scrolling: Vec<TopicRecord>,
    pub scrolling: Vec<TopicRecord>,
    pub unclassified: Vec<TopicRecord>,
    /// Plain searchable text reconstructed from display/table string lists.
    pub plain_text: String,
}

/// Fully transformed `|TOPIC` stream for one help file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicStore {
    blocks: Vec<TopicBlockInfo>,
    topics: Vec<Topic>,
    phrase_compression: PhraseCompression,
    phrase_count: usize,
}

impl TopicStore {
    /// Decodes `|TOPIC`, its block compression, phrase tables, and topic boundaries.
    pub(crate) fn parse(file: &HlpFile, system: &SystemInfo) -> Result<Self, HlpError> {
        let stream = file.internal_file("|TOPIC")?;
        let phrases = PhraseTable::load(file, system)?;
        let decoded_blocks = decode_blocks(stream.content, system)?;
        let blocks = decoded_blocks.iter().map(DecodedBlock::info).collect();
        let links = parse_links(&decoded_blocks, &phrases, system)?;
        let topics = group_topics(links, system)?;
        Ok(Self {
            blocks,
            topics,
            phrase_compression: phrases.kind(),
            phrase_count: phrases.len(),
        })
    }

    /// Returns transformed block diagnostics.
    pub fn blocks(&self) -> &[TopicBlockInfo] {
        &self.blocks
    }

    /// Returns all reconstructed topics in TOPICLINK traversal order.
    pub fn topics(&self) -> &[Topic] {
        &self.topics
    }

    /// Returns the phrase compression generation used by this file.
    pub const fn phrase_compression(&self) -> PhraseCompression {
        self.phrase_compression
    }

    /// Returns the number of decoded phrases available to topic records.
    pub const fn phrase_count(&self) -> usize {
        self.phrase_count
    }
}

/// Internal transformed topic block retaining the actual usable data bytes.
#[derive(Debug, Clone)]
struct DecodedBlock {
    index: usize,
    physical_size: usize,
    header: TopicBlockHeader,
    data: Vec<u8>,
}

impl DecodedBlock {
    fn info(&self) -> TopicBlockInfo {
        TopicBlockInfo {
            index: self.index,
            physical_size: self.physical_size,
            decoded_data_size: self.data.len(),
            header: self.header,
        }
    }
}

/// Internal TOPICLINK representation before grouping records into topics.
#[derive(Debug, Clone)]
struct DecodedLink {
    position: TopicPos,
    record_type: TopicRecordType,
    link_data1: Vec<u8>,
    link_data2: Vec<u8>,
}

/// Splits the physical stream and independently transforms each topic block.
fn decode_blocks(content: &[u8], system: &SystemInfo) -> Result<Vec<DecodedBlock>, HlpError> {
    if system.topic_block_size <= TOPIC_BLOCK_HEADER_SIZE {
        return Err(HlpError::invalid(
            "|TOPIC",
            format!("physical block size {} is too small", system.topic_block_size),
        ));
    }
    if system.topic_decompressed_block_size == 0 {
        return Err(HlpError::invalid("|TOPIC", "decoded block size is zero"));
    }

    let mut result = Vec::new();
    for (index, chunk) in content.chunks(system.topic_block_size).enumerate() {
        if chunk.len() < TOPIC_BLOCK_HEADER_SIZE {
            return Err(HlpError::invalid(
                "|TOPIC",
                format!("final physical block {index} is shorter than its 12-byte header"),
            ));
        }
        let mut header_reader = Reader::new(chunk, "TOPICBLOCKHEADER");
        let header = TopicBlockHeader {
            last_topic_link: TopicPos(header_reader.read_i32()?),
            first_topic_link: TopicPos(header_reader.read_i32()?),
            last_topic_header: TopicPos(header_reader.read_i32()?),
        };
        let stored = &chunk[TOPIC_BLOCK_HEADER_SIZE..];
        let data = match system.compression {
            Compression::None => stored
                .get(..stored.len().min(system.topic_decompressed_block_size))
                .unwrap_or(stored)
                .to_vec(),
            Compression::Lz77 => lz77_decompress(stored, system.topic_decompressed_block_size)?,
            Compression::Unknown(flags) => {
                return Err(HlpError::Unsupported {
                    context: "|TOPIC compression",
                    detail: format!("unrecognized |SYSTEM flags 0x{flags:04X}"),
                });
            }
        };
        result.push(DecodedBlock {
            index,
            physical_size: chunk.len(),
            header,
            data,
        });
    }

    if result.is_empty() {
        return Err(HlpError::invalid("|TOPIC", "stream contains no topic blocks"));
    }
    Ok(result)
}

/// Traverses the linked TOPICLINK chain and reconstructs both LinkData buffers.
fn parse_links(
    blocks: &[DecodedBlock],
    phrases: &PhraseTable,
    system: &SystemInfo,
) -> Result<Vec<DecodedLink>, HlpError> {
    let first = find_first_link(blocks, system)?;
    let transformed_capacity = blocks.iter().try_fold(0_usize, |total, block| {
        total.checked_add(block.data.len()).ok_or_else(|| {
            HlpError::invalid("|TOPIC", "transformed topic size overflows usize")
        })
    })?;
    let mut position = first;
    let mut visited = HashSet::new();
    let mut links = Vec::new();

    while position.0 >= i32::try_from(TOPIC_BLOCK_HEADER_SIZE).unwrap_or(12) {
        if links.len() >= MAX_TOPIC_LINKS {
            return Err(HlpError::invalid(
                "|TOPIC",
                format!("TOPICLINK count exceeds safety limit {MAX_TOPIC_LINKS}"),
            ));
        }
        if !visited.insert(position) {
            return Err(HlpError::invalid(
                "|TOPIC",
                format!("TOPICLINK cycle detected at position {}", position.0),
            ));
        }

        // The 21-byte TOPICLINK header is guaranteed by the format not to cross a
        // transformed block. Treat a crossing header as corruption instead of silently
        // stitching bytes across compiler padding.
        let (header_block_index, header_offset) = locate(blocks, system, position)?;
        let header_block = &blocks[header_block_index];
        let header_end = header_offset
            .checked_add(TOPIC_LINK_HEADER_SIZE)
            .ok_or_else(|| HlpError::invalid("TOPICLINK", "header offset overflow"))?;
        let header_bytes = header_block.data.get(header_offset..header_end).ok_or_else(|| {
            HlpError::invalid(
                "TOPICLINK",
                format!(
                    "21-byte header at position {} crosses transformed block {}",
                    position.0, header_block_index
                ),
            )
        })?;
        let mut reader = Reader::new(header_bytes, "TOPICLINK header");
        let block_size = nonnegative_usize(reader.read_i32()?, "TOPICLINK BlockSize")?;
        let data_len2 = nonnegative_usize(reader.read_i32()?, "TOPICLINK DataLen2")?;
        let _prev_raw = reader.read_i32()?;
        let next_raw = reader.read_i32()?;
        let data_len1 = nonnegative_usize(reader.read_i32()?, "TOPICLINK DataLen1")?;
        let record_type = TopicRecordType::from_byte(reader.read_u8()?);

        if block_size > MAX_TOPIC_RECORD_SIZE {
            return Err(HlpError::invalid(
                "TOPICLINK",
                format!("BlockSize {block_size} exceeds safety limit {MAX_TOPIC_RECORD_SIZE}"),
            ));
        }
        if data_len2 > MAX_TOPIC_RECORD_SIZE {
            return Err(HlpError::invalid(
                "TOPICLINK",
                format!("DataLen2 {data_len2} exceeds safety limit {MAX_TOPIC_RECORD_SIZE}"),
            ));
        }
        if block_size > transformed_capacity {
            return Err(HlpError::invalid(
                "TOPICLINK",
                format!(
                    "BlockSize {block_size} exceeds total transformed topic data {transformed_capacity}"
                ),
            ));
        }

        if block_size < TOPIC_LINK_HEADER_SIZE {
            return Err(HlpError::invalid(
                "TOPICLINK",
                format!("BlockSize {block_size} is below {TOPIC_LINK_HEADER_SIZE}"),
            ));
        }
        if data_len1 < TOPIC_LINK_HEADER_SIZE || data_len1 > block_size {
            return Err(HlpError::invalid(
                "TOPICLINK",
                format!("DataLen1 {data_len1} is outside BlockSize {block_size}"),
            ));
        }

        let record = read_logical(blocks, system, position, block_size)?;
        let link_data1 = record[TOPIC_LINK_HEADER_SIZE..data_len1].to_vec();
        let stored_data2 = &record[data_len1..block_size];
        let link_data2 = phrases.decode_link_data(stored_data2, data_len2)?;
        links.push(DecodedLink {
            position,
            record_type,
            link_data1,
            link_data2,
        });

        if next_raw <= 0 {
            break;
        }
        position = if system.minor <= 16 {
            advance_hc30_relative(blocks, system, position, next_raw)?
        } else {
            TopicPos(next_raw)
        };
    }

    if links.is_empty() {
        return Err(HlpError::invalid("|TOPIC", "no TOPICLINK records were decoded"));
    }
    Ok(links)
}

/// Applies an HC30 relative link distance in the original physical stream.
///
/// HC30 `NextBlock` counts skipped bytes and 12-byte physical TOPICBLOCKHEADERs, while
/// TOPICPOS itself addresses only transformed block data after one initial 12-byte bias.
fn advance_hc30_relative(
    blocks: &[DecodedBlock],
    system: &SystemInfo,
    position: TopicPos,
    distance: i32,
) -> Result<TopicPos, HlpError> {
    if distance <= 0 {
        return Err(HlpError::invalid(
            "HC30 TOPICLINK",
            format!("relative distance must be positive, got {distance}"),
        ));
    }
    let (block_index, offset) = locate(blocks, system, position)?;
    let physical = block_index
        .checked_mul(system.topic_block_size)
        .and_then(|value| value.checked_add(TOPIC_BLOCK_HEADER_SIZE))
        .and_then(|value| value.checked_add(offset))
        .ok_or_else(|| HlpError::invalid("HC30 TOPICLINK", "physical position overflow"))?;
    let next_physical = physical
        .checked_add(usize::try_from(distance).map_err(|_| {
            HlpError::invalid("HC30 TOPICLINK", "relative distance does not fit usize")
        })?)
        .ok_or_else(|| HlpError::invalid("HC30 TOPICLINK", "physical next-position overflow"))?;

    let next_block = next_physical / system.topic_block_size;
    let within_block = next_physical % system.topic_block_size;
    if within_block < TOPIC_BLOCK_HEADER_SIZE {
        return Err(HlpError::invalid(
            "HC30 TOPICLINK",
            format!(
                "relative link lands inside TOPICBLOCKHEADER at physical offset {next_physical}"
            ),
        ));
    }
    let next_offset = within_block - TOPIC_BLOCK_HEADER_SIZE;
    let block = blocks.get(next_block).ok_or_else(|| {
        HlpError::invalid(
            "HC30 TOPICLINK",
            format!("relative link selects missing block {next_block}"),
        )
    })?;
    if next_offset >= block.data.len() {
        return Err(HlpError::invalid(
            "HC30 TOPICLINK",
            format!(
                "relative link selects offset {next_offset} beyond block {next_block} data size {}",
                block.data.len()
            ),
        ));
    }

    let logical = TOPIC_BLOCK_HEADER_SIZE
        .checked_add(
            next_block
                .checked_mul(system.topic_decompressed_block_size)
                .ok_or_else(|| HlpError::invalid("HC30 TOPICLINK", "TOPICPOS overflow"))?,
        )
        .and_then(|value| value.checked_add(next_offset))
        .ok_or_else(|| HlpError::invalid("HC30 TOPICLINK", "TOPICPOS overflow"))?;
    let logical = i32::try_from(logical)
        .map_err(|_| HlpError::invalid("HC30 TOPICLINK", "TOPICPOS exceeds i32 range"))?;
    Ok(TopicPos(logical))
}

/// Finds the earliest valid `FirstTopicLink` advertised by transformed block headers.
fn find_first_link(blocks: &[DecodedBlock], system: &SystemInfo) -> Result<TopicPos, HlpError> {
    let mut candidates: Vec<_> = blocks
        .iter()
        .map(|block| block.header.first_topic_link)
        .filter(|pos| pos.0 >= i32::try_from(TOPIC_BLOCK_HEADER_SIZE).unwrap_or(12))
        .collect();
    candidates.sort_unstable();
    candidates.dedup();
    for candidate in candidates {
        if locate(blocks, system, candidate).is_ok() {
            return Ok(candidate);
        }
    }
    Err(HlpError::invalid(
        "|TOPIC",
        "no block advertises a valid FirstTopicLink",
    ))
}

/// Reads a logical TOPICLINK byte range while skipping physical block headers automatically.
fn read_logical(
    blocks: &[DecodedBlock],
    system: &SystemInfo,
    start: TopicPos,
    length: usize,
) -> Result<Vec<u8>, HlpError> {
    let (mut block_index, mut offset) = locate(blocks, system, start)?;
    let mut remaining = length;
    let mut output = Vec::with_capacity(length);

    while remaining > 0 {
        let block = blocks.get(block_index).ok_or_else(|| {
            HlpError::invalid("TOPICPOS", "record continues beyond final transformed block")
        })?;
        if offset > block.data.len() {
            return Err(HlpError::invalid(
                "TOPICPOS",
                format!("offset {offset} exceeds decoded block {} size {}", block_index, block.data.len()),
            ));
        }
        let available = block.data.len().saturating_sub(offset);
        if available == 0 {
            block_index += 1;
            offset = 0;
            continue;
        }
        let take = remaining.min(available);
        output.extend_from_slice(&block.data[offset..offset + take]);
        remaining -= take;
        block_index += 1;
        offset = 0;
    }
    Ok(output)
}

/// Maps a TOPICPOS to a transformed block number and byte offset within its data buffer.
fn locate(
    blocks: &[DecodedBlock],
    system: &SystemInfo,
    position: TopicPos,
) -> Result<(usize, usize), HlpError> {
    let raw = position.0;
    if raw < i32::try_from(TOPIC_BLOCK_HEADER_SIZE).unwrap_or(12) {
        return Err(HlpError::invalid(
            "TOPICPOS",
            format!("position {raw} is below the 12-byte topic-block header"),
        ));
    }
    let adjusted = usize::try_from(raw - i32::try_from(TOPIC_BLOCK_HEADER_SIZE).unwrap_or(12))
        .map_err(|_| HlpError::invalid("TOPICPOS", "position does not fit usize"))?;
    let span = system.topic_decompressed_block_size;
    let block_index = adjusted / span;
    let offset = adjusted % span;
    let block = blocks.get(block_index).ok_or_else(|| {
        HlpError::invalid(
            "TOPICPOS",
            format!("position {raw} selects missing transformed block {block_index}"),
        )
    })?;
    if offset >= block.data.len() {
        return Err(HlpError::invalid(
            "TOPICPOS",
            format!(
                "position {raw} selects offset {offset} beyond decoded block {block_index} size {}",
                block.data.len()
            ),
        ));
    }
    Ok((block_index, offset))
}

/// Converts the linear TOPICLINK stream into topic records and visual regions.
fn group_topics(links: Vec<DecodedLink>, system: &SystemInfo) -> Result<Vec<Topic>, HlpError> {
    let mut topics = Vec::new();
    let mut current: Option<TopicBuilder> = None;

    for link in links {
        if link.record_type == TopicRecordType::TopicHeader {
            if let Some(builder) = current.take() {
                topics.push(builder.finish());
            }
            current = Some(TopicBuilder::from_header(link, system)?);
            continue;
        }

        let Some(builder) = current.as_mut() else {
            // Some compilers leave housekeeping records before their first topic header. Preserve
            // traversal safety but do not manufacture a synthetic user-visible topic for them.
            continue;
        };
        builder.push_record(link);
    }

    if let Some(builder) = current {
        topics.push(builder.finish());
    }
    if topics.is_empty() {
        return Err(HlpError::invalid("|TOPIC", "no type-2 topic headers were decoded"));
    }
    Ok(topics)
}

/// Mutable accumulator for records following one topic header.
struct TopicBuilder {
    id: TopicId,
    title: String,
    macros: Vec<String>,
    metadata: TopicMetadata,
    non_scrolling: Vec<TopicRecord>,
    scrolling: Vec<TopicRecord>,
    unclassified: Vec<TopicRecord>,
}

impl TopicBuilder {
    fn from_header(link: DecodedLink, system: &SystemInfo) -> Result<Self, HlpError> {
        let metadata = parse_topic_metadata(&link.link_data1, system)?;
        let mut strings = link.link_data2.split(|byte| *byte == 0);
        let title = decode_windows_1252(strings.next().unwrap_or_default());
        let macros = strings
            .filter(|part| !part.is_empty())
            .map(decode_windows_1252)
            .collect();
        Ok(Self {
            id: TopicId(link.position),
            title,
            macros,
            metadata,
            non_scrolling: Vec::new(),
            scrolling: Vec::new(),
            unclassified: Vec::new(),
        })
    }

    fn push_record(&mut self, link: DecodedLink) {
        let region = classify_region(link.position, &self.metadata);
        let plain_text = match link.record_type {
            TopicRecordType::Display30
            | TopicRecordType::Table30
            | TopicRecordType::Display
            | TopicRecordType::Table => {
                visible_string_list(&link.link_data2)
            }
            TopicRecordType::TopicHeader
            | TopicRecordType::Graphic30
            | TopicRecordType::EmbeddedWindow30
            | TopicRecordType::NoRender30
            | TopicRecordType::Graphic
            | TopicRecordType::EmbeddedWindow
            | TopicRecordType::Unknown(_) => String::new(),
        };
        let record = TopicRecord {
            position: link.position,
            record_type: link.record_type,
            region,
            link_data1: link.link_data1,
            link_data2: link.link_data2,
            plain_text,
        };
        match region {
            TopicRegion::NonScrolling => self.non_scrolling.push(record),
            TopicRegion::Scrolling => self.scrolling.push(record),
            TopicRegion::Header | TopicRegion::Unclassified => self.unclassified.push(record),
        }
    }

    fn finish(self) -> Topic {
        let plain_text = self
            .non_scrolling
            .iter()
            .chain(self.scrolling.iter())
            .chain(self.unclassified.iter())
            .filter_map(|record| (!record.plain_text.is_empty()).then_some(record.plain_text.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        Topic {
            id: self.id,
            title: self.title,
            macros: self.macros,
            metadata: self.metadata,
            non_scrolling: self.non_scrolling,
            scrolling: self.scrolling,
            unclassified: self.unclassified,
            plain_text,
        }
    }
}

/// Parses either the 12-byte HC30 or 28-byte HC31+ topic-header payload.
fn parse_topic_metadata(data: &[u8], system: &SystemInfo) -> Result<TopicMetadata, HlpError> {
    let mut reader = Reader::new(data, "topic header LinkData1");
    let block_size = reader.read_i32()?;
    if block_size < 0 {
        return Err(HlpError::invalid(
            "topic header LinkData1",
            format!("negative topic BlockSize {block_size}"),
        ));
    }
    if system.minor <= 16 {
        let previous = reader.read_i32()?;
        let next = reader.read_i32()?;
        return Ok(TopicMetadata {
            block_size,
            browse_back: None,
            browse_forward: None,
            topic_number: None,
            non_scroll_start: None,
            scroll_start: None,
            next_topic: None,
            previous_topic_number: optional_hc30_topic_number(previous),
            next_topic_number: optional_hc30_topic_number(next),
        });
    }

    let browse_back = reader.read_i32()?;
    let browse_forward = reader.read_i32()?;
    let topic_number = reader.read_i32()?;
    let non_scroll = reader.read_i32()?;
    let scroll = reader.read_i32()?;
    let next_topic = reader.read_i32()?;
    Ok(TopicMetadata {
        block_size,
        browse_back: optional_topic_offset(browse_back),
        browse_forward: optional_topic_offset(browse_forward),
        topic_number: Some(topic_number),
        non_scroll_start: optional_topic_pos(non_scroll),
        scroll_start: optional_topic_pos(scroll),
        next_topic: optional_topic_pos(next_topic),
        previous_topic_number: None,
        next_topic_number: None,
    })
}

/// Classifies a record using the non-scroll/scroll start positions from its topic header.
fn classify_region(position: TopicPos, metadata: &TopicMetadata) -> TopicRegion {
    if let Some(non_scroll) = metadata.non_scroll_start {
        if position >= non_scroll && metadata.scroll_start.map_or(true, |scroll| position < scroll) {
            return TopicRegion::NonScrolling;
        }
    }
    if let Some(scroll) = metadata.scroll_start {
        if position >= scroll && metadata.next_topic.map_or(true, |next| position < next) {
            return TopicRegion::Scrolling;
        }
    }
    // HC30 has no separate fixed/scroll positions; its display records belong to the body.
    if metadata.non_scroll_start.is_none() && metadata.scroll_start.is_none() {
        return TopicRegion::Scrolling;
    }
    TopicRegion::Unclassified
}

/// Concatenates every NUL-separated display string without inventing paragraph semantics yet.
fn visible_string_list(data: &[u8]) -> String {
    data.split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(decode_windows_1252)
        .collect::<Vec<_>>()
        .join("")
}

const fn optional_topic_pos(value: i32) -> Option<TopicPos> {
    if value < 12 { None } else { Some(TopicPos(value)) }
}

const fn optional_topic_offset(value: i32) -> Option<TopicOffset> {
    if value < 0 {
        None
    } else {
        Some(TopicOffset(value))
    }
}

const fn optional_hc30_topic_number(value: i32) -> Option<i32> {
    if value < 0 || value == 0xFFFF {
        None
    } else {
        Some(value)
    }
}

fn nonnegative_usize(value: i32, context: &'static str) -> Result<usize, HlpError> {
    if value < 0 {
        return Err(HlpError::invalid(context, format!("negative value {value}")));
    }
    usize::try_from(value).map_err(|_| HlpError::invalid(context, "value does not fit usize"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system() -> SystemInfo {
        SystemInfo {
            minor: 21,
            major: 1,
            version: crate::WinHelpVersion::Windows31,
            generation_timestamp: 0,
            flags: 0,
            compression: Compression::None,
            topic_block_size: 64,
            topic_decompressed_block_size: 52,
            title: None,
            copyright: None,
            contents_topic_offset: None,
            config_macros: Vec::new(),
            contents_file: None,
            locale_id: None,
            font_charsets: Vec::new(),
            windows: Vec::new(),
            unknown_records: Vec::new(),
        }
    }

    #[test]
    fn topicpos_maps_across_transformed_blocks() {
        let blocks = vec![
            DecodedBlock {
                index: 0,
                physical_size: 64,
                header: TopicBlockHeader {
                    last_topic_link: TopicPos(-1),
                    first_topic_link: TopicPos(12),
                    last_topic_header: TopicPos(12),
                },
                data: vec![1; 52],
            },
            DecodedBlock {
                index: 1,
                physical_size: 64,
                header: TopicBlockHeader {
                    last_topic_link: TopicPos(12),
                    first_topic_link: TopicPos(64),
                    last_topic_header: TopicPos(12),
                },
                data: vec![2; 52],
            },
        ];
        assert_eq!(locate(&blocks, &system(), TopicPos(12)).unwrap(), (0, 0));
        assert_eq!(locate(&blocks, &system(), TopicPos(64)).unwrap(), (1, 0));
        assert_eq!(locate(&blocks, &system(), TopicPos(69)).unwrap(), (1, 5));
    }

    #[test]
    fn logical_read_skips_physical_headers() {
        let blocks = vec![
            DecodedBlock {
                index: 0,
                physical_size: 64,
                header: TopicBlockHeader {
                    last_topic_link: TopicPos(-1),
                    first_topic_link: TopicPos(12),
                    last_topic_header: TopicPos(12),
                },
                data: (0_u8..52).collect(),
            },
            DecodedBlock {
                index: 1,
                physical_size: 64,
                header: TopicBlockHeader {
                    last_topic_link: TopicPos(12),
                    first_topic_link: TopicPos(64),
                    last_topic_header: TopicPos(12),
                },
                data: (100_u8..152).collect(),
            },
        ];
        assert_eq!(
            read_logical(&blocks, &system(), TopicPos(62), 4).unwrap(),
            vec![50, 51, 100, 101]
        );
    }

    #[test]
    fn parses_modern_topic_metadata() {
        let mut bytes = Vec::new();
        for value in [100_i32, -1, 7, 3, 40, 50, 90] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let metadata = parse_topic_metadata(&bytes, &system()).unwrap();
        assert_eq!(metadata.topic_number, Some(3));
        assert_eq!(metadata.non_scroll_start, Some(TopicPos(40)));
        assert_eq!(metadata.scroll_start, Some(TopicPos(50)));
        assert_eq!(metadata.browse_back, None);
        assert_eq!(metadata.browse_forward, Some(TopicOffset(7)));
    }

    #[test]
    fn visible_text_joins_linkdata2_strings() {
        assert_eq!(visible_string_list(b"Hello \0world\0"), "Hello world");
    }
    #[test]
    fn hc30_relative_next_position_counts_skipped_block_header() {
        let system = SystemInfo {
            minor: 15,
            version: crate::WinHelpVersion::Windows30,
            topic_block_size: 64,
            topic_decompressed_block_size: 52,
            ..system()
        };
        let blocks = vec![
            DecodedBlock {
                index: 0,
                physical_size: 64,
                header: TopicBlockHeader {
                    last_topic_link: TopicPos(-1),
                    first_topic_link: TopicPos(12),
                    last_topic_header: TopicPos(12),
                },
                data: vec![0; 52],
            },
            DecodedBlock {
                index: 1,
                physical_size: 64,
                header: TopicBlockHeader {
                    last_topic_link: TopicPos(62),
                    first_topic_link: TopicPos(64),
                    last_topic_header: TopicPos(12),
                },
                data: vec![0; 52],
            },
        ];
        // TOPICPOS 62 is physical byte 62. The next block's data starts at physical
        // byte 76, so HC30 stores a relative distance of 14 (2 skipped + 12 header).
        assert_eq!(
            advance_hc30_relative(&blocks, &system, TopicPos(62), 14).unwrap(),
            TopicPos(64)
        );
    }

    #[test]
    fn parses_hc30_topic_numbers_as_32_bit_values() {
        let system = SystemInfo {
            minor: 15,
            version: crate::WinHelpVersion::Windows30,
            ..system()
        };
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&100_i32.to_le_bytes());
        bytes.extend_from_slice(&12345_i32.to_le_bytes());
        bytes.extend_from_slice(&0xFFFF_i32.to_le_bytes());
        let metadata = parse_topic_metadata(&bytes, &system).unwrap();
        assert_eq!(metadata.previous_topic_number, Some(12345));
        assert_eq!(metadata.next_topic_number, None);
    }

    #[test]
    fn decodes_complete_uncompressed_topic_with_two_regions() {
        let system = SystemInfo {
            topic_block_size: 128,
            topic_decompressed_block_size: 116,
            ..system()
        };
        let mut body = Vec::new();

        let header_pos = 12_i32;
        let fixed_pos = 67_i32;
        let scroll_pos = 94_i32;

        let mut metadata = Vec::new();
        for value in [108_i32, -1, -1, 0, fixed_pos, scroll_pos, -1] {
            metadata.extend_from_slice(&value.to_le_bytes());
        }
        push_link(&mut body, 6, -1, fixed_pos, 49, 0x02, &metadata, b"Intro\0");
        push_link(&mut body, 6, header_pos, scroll_pos, 21, 0x20, &[], b"Fixed\0");
        push_link(&mut body, 5, fixed_pos, -1, 21, 0x20, &[], b"Body\0");
        body.resize(116, 0);

        let mut stream = Vec::new();
        stream.extend_from_slice(&(-1_i32).to_le_bytes());
        stream.extend_from_slice(&header_pos.to_le_bytes());
        stream.extend_from_slice(&header_pos.to_le_bytes());
        stream.extend_from_slice(&body);

        let blocks = decode_blocks(&stream, &system).unwrap();
        let links = parse_links(&blocks, &PhraseTable::none(), &system).unwrap();
        let topics = group_topics(links, &system).unwrap();
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].title, "Intro");
        assert_eq!(topics[0].non_scrolling.len(), 1);
        assert_eq!(topics[0].scrolling.len(), 1);
        assert_eq!(topics[0].plain_text, "Fixed\nBody");
    }

    fn push_link(
        target: &mut Vec<u8>,
        data_len2: i32,
        previous: i32,
        next: i32,
        data_len1: i32,
        record_type: u8,
        link_data1: &[u8],
        link_data2: &[u8],
    ) {
        let block_size = usize::try_from(data_len1).unwrap() + link_data2.len();
        target.extend_from_slice(&i32::try_from(block_size).unwrap().to_le_bytes());
        target.extend_from_slice(&data_len2.to_le_bytes());
        target.extend_from_slice(&previous.to_le_bytes());
        target.extend_from_slice(&next.to_le_bytes());
        target.extend_from_slice(&data_len1.to_le_bytes());
        target.push(record_type);
        target.extend_from_slice(link_data1);
        target.extend_from_slice(link_data2);
    }

}
