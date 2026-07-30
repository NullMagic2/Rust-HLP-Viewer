//! Loaded WinHelp document, navigation resolution, and browser-style history.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::{
    ContentsFile, DirectoryEntry, FontTable, FormattedRecord, HlpError, HlpFile,
    Hotspot, HotspotTarget, Inline, KeywordIndex, NavigationMetadata, PictureHotspot, PictureReference,
    PictureSource, ResolvedKeyword, SearchHit, SystemInfo, TableCellContent, Topic, TopicId, TopicOffset,
    TopicPos, TopicRecord, TopicRecordType, TopicStore, WindowDefinition,
};
use crate::graphics::{
    DecodedPicture, DecodedPictureHotspotTarget, decode_embedded_picture, decode_indexed_picture,
};
use crate::search::SearchIndex;

/// Formatting-decoded representation of one topic, split into the two WinHelp visual regions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicPresentation {
    pub id: TopicId,
    pub title: String,
    pub non_scrolling: Vec<FormattedRecord>,
    pub scrolling: Vec<FormattedRecord>,
    /// Non-fatal record-level decode errors. A plain-text fallback is retained for each failure.
    pub warnings: Vec<String>,
}

impl TopicPresentation {
    fn from_topic(topic: &Topic, fonts: &FontTable) -> Self {
        let mut warnings = Vec::new();
        let non_scrolling = decode_records(&topic.non_scrolling, fonts, &mut warnings);

        // Real-world HLPs occasionally carry display records whose TOPICPOS lies just outside
        // the fixed/scrolling bounds advertised by their type-2 topic header. The topic parser
        // intentionally retains those records as `unclassified` so that malformed metadata never
        // destroys content. Presentation must retain the same guarantee: otherwise a context hash
        // can resolve to the correct popup topic while its entire visible body is silently dropped.
        // Unknown housekeeping records remain non-displayable and are not promoted.
        let mut body_records: Vec<&TopicRecord> = topic
            .scrolling
            .iter()
            .chain(topic.unclassified.iter().filter(|record| {
                matches!(
                    record.record_type,
                    TopicRecordType::Display30
                        | TopicRecordType::Graphic30
                        | TopicRecordType::Table30
                        | TopicRecordType::EmbeddedWindow30
                        | TopicRecordType::Display
                        | TopicRecordType::Graphic
                        | TopicRecordType::Table
                        | TopicRecordType::EmbeddedWindow
                )
            }))
            .collect();
        body_records.sort_by_key(|record| record.position);

        let recovered = body_records
            .iter()
            .filter(|record| record.region == crate::TopicRegion::Unclassified)
            .count();
        if recovered != 0 {
            warnings.push(format!(
                "recovered {recovered} display record(s) outside the topic's declared visual ranges into the scrolling region"
            ));
        }
        let scrolling = decode_record_refs(&body_records, fonts, &mut warnings);

        Self {
            id: topic.id,
            title: topic.title.clone(),
            non_scrolling,
            scrolling,
            warnings,
        }
    }
}

/// One exact TOPICOFFSET anchor at the start of a display/table TOPICLINK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopicOffsetAnchor {
    pub offset: TopicOffset,
    pub topic_index: usize,
    pub record_position: TopicPos,
}

/// One loaded help file plus decoded metadata/topics needed by navigation and rendering layers.
#[derive(Debug, Clone)]
pub struct HelpDocument {
    source_path: PathBuf,
    file: HlpFile,
    system: SystemInfo,
    fonts: FontTable,
    topics: TopicStore,
    navigation: NavigationMetadata,
    keywords: KeywordIndex,
    contents: Option<ContentsFile>,
    contents_warning: Option<String>,
    search_index: SearchIndex,
    presentations: Vec<TopicPresentation>,
    offset_anchors: Vec<TopicOffsetAnchor>,
    topic_start_offsets: Vec<Option<TopicOffset>>,
}

impl HelpDocument {
    /// Opens an HLP document and decodes its container, fonts, phrases, topics, and display records.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, HlpError> {
        let source_path = path.as_ref().to_path_buf();
        let file = HlpFile::open(&source_path)?;
        let system = file.system_info()?;
        let fonts = file.fonts()?;
        let topics = file.topics()?;
        let navigation = file.navigation_metadata()?;
        let keywords = KeywordIndex::load(&file)?;
        let (contents, contents_warning) =
            ContentsFile::discover(&source_path, system.contents_file.as_deref());
        let mut presentations: Vec<_> = topics
            .topics()
            .iter()
            .map(|topic| TopicPresentation::from_topic(topic, &fonts))
            .collect();
        resolve_presentation_pictures(&file, &navigation, &mut presentations);
        let (offset_anchors, topic_start_offsets) = build_offset_anchors(&topics, &system);
        let search_index = SearchIndex::build(
            topics.topics(),
            keywords.default_table(),
            |offset| resolve_topic_offset_in_anchors(&offset_anchors, offset),
        );
        Ok(Self {
            source_path,
            file,
            system,
            fonts,
            topics,
            navigation,
            keywords,
            contents,
            contents_warning,
            search_index,
            presentations,
            offset_anchors,
            topic_start_offsets,
        })
    }

    /// Returns the source pathname used to open this document.
    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    /// Returns decoded `|SYSTEM` metadata.
    pub const fn system(&self) -> &SystemInfo {
        &self.system
    }

    /// Returns decoded `|FONT` metadata and descriptors.
    pub const fn fonts(&self) -> &FontTable {
        &self.fonts
    }

    /// Returns all reconstructed topics in stream order.
    pub fn topics(&self) -> &[Topic] {
        self.topics.topics()
    }

    /// Returns the formatting-decoded presentation topics in stream order.
    pub fn presentations(&self) -> &[TopicPresentation] {
        &self.presentations
    }

    /// Returns the presentation topic with a matching topic id.
    pub fn presentation(&self, id: TopicId) -> Option<&TopicPresentation> {
        self.presentations.iter().find(|topic| topic.id == id)
    }

    /// Returns decoded optional context/map/window navigation metadata.
    pub const fn navigation(&self) -> &NavigationMetadata {
        &self.navigation
    }

    /// Returns discovered authored Contents metadata from `.CNT`, or from a same-basename
    /// compiled `.GID` cache when the CNT is absent and the GID carries a valid hierarchy.
    pub const fn contents_file(&self) -> Option<&ContentsFile> {
        self.contents.as_ref()
    }

    /// Returns a non-fatal `.CNT`/`.GID` Contents discovery or decode warning.
    pub fn contents_warning(&self) -> Option<&str> {
        self.contents_warning.as_deref()
    }

    /// Returns all authored keyword tables decoded from `|?WBTREE` / `|?WDATA`.
    pub const fn keywords(&self) -> &KeywordIndex {
        &self.keywords
    }

    /// Returns the standard K-table keywords with their topic offsets resolved to topic indices.
    pub fn resolved_keywords(&self) -> &[ResolvedKeyword] {
        self.search_index.keywords()
    }

    /// Searches titles, authored K keywords, and decoded body text in descending relevance order.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        self.search_index.search(query, limit)
    }

    /// Resolves a textual context/map reference used by `.CNT` topic entries.
    pub fn topic_index_for_reference(&self, reference: &str) -> Option<usize> {
        let reference = reference.trim();
        if reference.is_empty() {
            return None;
        }
        self.topic_index_for_context_name(reference).or_else(|| {
            parse_map_reference(reference).and_then(|map_id| self.topic_index_for_map_id(map_id))
        })
    }

    /// Returns the TOPICOFFSET anchors built from display/table TopicLength fields.
    pub fn offset_anchors(&self) -> &[TopicOffsetAnchor] {
        &self.offset_anchors
    }

    /// Resolves a WinHelp TOPICOFFSET to the topic containing that cursor position.
    pub fn resolve_topic_offset(&self, offset: TopicOffset) -> Option<usize> {
        resolve_topic_offset_in_anchors(&self.offset_anchors, offset)
    }

    pub fn topic_index_for_context_name(&self, name: &str) -> Option<usize> {
        self.navigation
            .offset_for_context_name(name)
            .and_then(|offset| self.resolve_topic_offset(offset))
    }

    pub fn topic_index_for_map_id(&self, map_id: i32) -> Option<usize> {
        self.navigation
            .offset_for_map_id(map_id)
            .and_then(|offset| self.resolve_topic_offset(offset))
    }

    /// Resolves the signed 32-bit context hash used by JumpHash/PopupHash macros.
    pub fn topic_index_for_context_hash(&self, hash: i32) -> Option<usize> {
        self.navigation
            .offset_for_hash(hash)
            .and_then(|offset| self.resolve_topic_offset(offset))
    }

    pub fn contents_topic_index(&self) -> Option<usize> {
        self.system
            .contents_topic_offset
            .map(TopicOffset)
            .and_then(|offset| self.resolve_topic_offset(offset))
            .or_else(|| {
                self.navigation
                    .hc30_index_topic_pos()
                    .and_then(|position| self.topic_index_for_topic_pos(position))
            })
            .or_else(|| (!self.presentations.is_empty()).then_some(0))
    }

    /// Returns the topic shown when an HLP is opened directly, without an explicit WinHelp
    /// command such as HELP_CONTENTS or HELP_CONTEXT.
    ///
    /// Direct file opening is intentionally distinct from the Contents command. The latter uses
    /// the `[OPTIONS]` Contents topic and can legitimately point deep into the topic stream.
    pub fn startup_topic_index(&self) -> Option<usize> {
        first_displayable_presentation(&self.presentations)
    }

    pub fn browse_previous_index(&self, topic_index: usize) -> Option<usize> {
        let metadata = &self.topics.topics().get(topic_index)?.metadata;
        metadata
            .browse_back
            .and_then(|offset| self.resolve_topic_offset(offset))
            .or_else(|| {
                metadata
                    .previous_topic_number
                    .and_then(|number| self.navigation.topic_pos_for_hc30_number(number))
                    .and_then(|position| self.topic_index_for_topic_pos(position))
            })
    }

    pub fn browse_next_index(&self, topic_index: usize) -> Option<usize> {
        let metadata = &self.topics.topics().get(topic_index)?.metadata;
        metadata
            .browse_forward
            .and_then(|offset| self.resolve_topic_offset(offset))
            .or_else(|| {
                metadata
                    .next_topic_number
                    .and_then(|number| self.navigation.topic_pos_for_hc30_number(number))
                    .and_then(|position| self.topic_index_for_topic_pos(position))
            })
    }

    pub fn topic_index_for_topic_pos(&self, position: TopicPos) -> Option<usize> {
        self.topics
            .topics()
            .iter()
            .position(|topic| topic.id.0 == position)
    }

    pub fn topic_start_offset(&self, topic_index: usize) -> Option<TopicOffset> {
        self.topic_start_offsets.get(topic_index).copied().flatten()
    }

    pub fn default_window_for_topic(&self, topic_index: usize) -> Option<&WindowDefinition> {
        let offset = self.topic_start_offset(topic_index)?;
        let number = self.navigation.default_window_for_offset(offset)?;
        usize::try_from(number).ok().and_then(|index| self.system.windows.get(index))
    }

    pub fn window_by_name(&self, name: &str) -> Option<&WindowDefinition> {
        self.system.windows.iter().find(|window| {
            window.name.as_deref().is_some_and(|value| value.eq_ignore_ascii_case(name))
        })
    }

    pub fn window_by_number(&self, number: u8) -> Option<&WindowDefinition> {
        self.system.windows.get(usize::from(number))
    }

    /// Returns the internal HLP directory for diagnostics and future stream loaders.
    pub fn directory(&self) -> &[DirectoryEntry] {
        self.file.directory()
    }

    /// Returns the underlying parsed HLP container.
    pub const fn file(&self) -> &HlpFile {
        &self.file
    }
}

/// Resolves picture references after formatting has synchronized LinkData1 and LinkData2.
/// Indexed graphics are cached because the same `|bmN` asset can be reused by many topics.
fn resolve_presentation_pictures(
    file: &HlpFile,
    navigation: &NavigationMetadata,
    presentations: &mut [TopicPresentation],
) {
    let mut cache: BTreeMap<u16, Result<DecodedPicture, String>> = BTreeMap::new();
    for presentation in presentations {
        let mut picture_warnings = Vec::new();
        for record in presentation
            .non_scrolling
            .iter_mut()
            .chain(presentation.scrolling.iter_mut())
        {
            resolve_record_pictures(
                file,
                navigation,
                record,
                &mut cache,
                &mut picture_warnings,
            );
        }
        presentation.warnings.extend(picture_warnings);
    }
}

fn resolve_record_pictures(
    file: &HlpFile,
    navigation: &NavigationMetadata,
    record: &mut FormattedRecord,
    cache: &mut BTreeMap<u16, Result<DecodedPicture, String>>,
    warnings: &mut Vec<String>,
) {
    for paragraph in &mut record.paragraphs {
        for inline in &mut paragraph.inlines {
            let Inline::Picture(picture) = inline else {
                continue;
            };
            resolve_one_picture(file, navigation, picture, cache, warnings);
        }
    }
    resolve_table_pictures(
        file,
        navigation,
        &mut record.table_cells,
        cache,
        warnings,
    );
}

fn resolve_table_pictures(
    file: &HlpFile,
    navigation: &NavigationMetadata,
    cells: &mut [crate::TableCell],
    cache: &mut BTreeMap<u16, Result<DecodedPicture, String>>,
    warnings: &mut Vec<String>,
) {
    for cell in cells {
        match &mut cell.content {
            TableCellContent::Picture(picture) => {
                resolve_one_picture(file, navigation, picture, cache, warnings);
            }
            TableCellContent::Table(table) => {
                let nested = table.as_mut();
                resolve_table_pictures(
                    file,
                    navigation,
                    nested.cells.as_mut_slice(),
                    cache,
                    warnings,
                );
            }
            TableCellContent::Display { .. }
            | TableCellContent::EmbeddedWindow(_)
            | TableCellContent::NoRender { .. }
            | TableCellContent::Unsupported { .. } => {}
        }
    }
}

fn resolve_one_picture(
    file: &HlpFile,
    navigation: &NavigationMetadata,
    picture: &mut PictureReference,
    cache: &mut BTreeMap<u16, Result<DecodedPicture, String>>,
    warnings: &mut Vec<String>,
) {
    match resolve_picture(file, navigation, picture, cache) {
        Ok(resolved_warnings) => warnings.extend(resolved_warnings),
        Err(message) => {
            picture.decode_warning = Some(message.clone());
            warnings.push(message);
        }
    }
}

fn resolve_picture(
    file: &HlpFile,
    navigation: &NavigationMetadata,
    picture: &mut PictureReference,
    cache: &mut BTreeMap<u16, Result<DecodedPicture, String>>,
) -> Result<Vec<String>, String> {
    let result = match &picture.source {
        PictureSource::Indexed(index) => cache
            .entry(*index)
            .or_insert_with(|| decode_indexed_picture(file, *index).map_err(|error| error.to_string()))
            .clone(),
        PictureSource::Embedded(bytes) => {
            decode_embedded_picture(bytes).map_err(|error| error.to_string())
        }
        PictureSource::Unsupported(_) => {
            return Err(format!(
                "picture command 0x{:02X} type 0x{:02X} has an unsupported source selector",
                picture.command, picture.picture_type
            ));
        }
    };

    match result {
        Ok(image) => {
            let (hotspots, warnings) = resolve_graphical_hotspots(navigation, &image);
            picture.hotspots = hotspots;
            picture.image = Some(image);
            picture.decode_warning = None;
            Ok(warnings)
        }
        Err(message) => Err(format!(
            "picture command 0x{:02X} type 0x{:02X}: {message}",
            picture.command, picture.picture_type
        )),
    }
}

fn resolve_graphical_hotspots(
    navigation: &NavigationMetadata,
    image: &DecodedPicture,
) -> (Vec<PictureHotspot>, Vec<String>) {
    let mut hotspots = Vec::with_capacity(image.hotspots.len());
    let mut warnings = Vec::new();
    for source in image.hotspots.iter() {
        let hotspot = match &source.target {
            DecodedPictureHotspotTarget::Macro(text) => Hotspot {
                target: HotspotTarget::Macro(text.clone()),
                emphasized: false,
            },
            DecodedPictureHotspotTarget::Context {
                name,
                popup,
                window_name,
                opcode,
            } => {
                let Some(offset) = navigation.offset_for_context_name(name) else {
                    warnings.push(format!(
                        "graphical hotspot context '{name}' could not be resolved"
                    ));
                    continue;
                };
                let target = if let Some(window_name) = window_name {
                    HotspotTarget::External {
                        opcode: *opcode,
                        type_code: 6,
                        offset,
                        window_number: None,
                        help_file: None,
                        window_name: Some(window_name.clone()),
                    }
                } else {
                    HotspotTarget::Internal {
                        offset,
                        popup: *popup,
                    }
                };
                Hotspot {
                    target,
                    emphasized: false,
                }
            }
        };
        hotspots.push(PictureHotspot {
            x: source.x,
            y: source.y,
            width: source.width,
            height: source.height,
            hotspot,
        });
    }
    (hotspots, warnings)
}

fn resolve_topic_offset_in_anchors(
    anchors: &[TopicOffsetAnchor],
    offset: TopicOffset,
) -> Option<usize> {
    if offset.0 < 0 {
        return None;
    }
    match anchors.binary_search_by_key(&offset.0, |anchor| anchor.offset.0) {
        Ok(index) => Some(anchors[index].topic_index),
        Err(0) => None,
        Err(index) => {
            let anchor = anchors[index - 1];
            let target_block = offset.0 / 32_768;
            let anchor_block = anchor.offset.0 / 32_768;
            (target_block == anchor_block).then_some(anchor.topic_index)
        }
    }
}

fn parse_map_reference(reference: &str) -> Option<i32> {
    if let Some(hex) = reference
        .strip_prefix("0x")
        .or_else(|| reference.strip_prefix("0X"))
    {
        i32::from_str_radix(hex, 16).ok()
    } else {
        reference.parse::<i32>().ok()
    }
}

fn build_offset_anchors(
    topics: &TopicStore,
    system: &SystemInfo,
) -> (Vec<TopicOffsetAnchor>, Vec<Option<TopicOffset>>) {
    let mut records = Vec::new();
    for (topic_index, topic) in topics.topics().iter().enumerate() {
        for record in topic
            .non_scrolling
            .iter()
            .chain(topic.scrolling.iter())
            .chain(topic.unclassified.iter())
        {
            if matches!(
                record.record_type,
                TopicRecordType::Display | TopicRecordType::Table
            ) {
                records.push((record.position, topic_index, record));
            }
        }
    }
    records.sort_by_key(|(position, _, _)| position.0);

    let mut block_counts: BTreeMap<i32, i32> = BTreeMap::new();
    let mut anchors = Vec::new();
    let mut topic_starts = vec![None; topics.topics().len()];
    for (position, topic_index, record) in records {
        let Some(block) = topicpos_block(position, system.topic_decompressed_block_size) else {
            continue;
        };
        let count = block_counts.entry(block).or_insert(0);
        let offset_value = block.saturating_mul(32_768).saturating_add(*count);
        let offset = TopicOffset(offset_value);
        anchors.push(TopicOffsetAnchor {
            offset,
            topic_index,
            record_position: position,
        });
        if topic_starts[topic_index].is_none() {
            topic_starts[topic_index] = Some(offset);
        }
        if let Ok(formatted) = FormattedRecord::decode(record) {
            *count = count.saturating_add(i32::from(formatted.topic_length.unwrap_or(0)));
        }
    }
    anchors.sort_by_key(|anchor| anchor.offset.0);
    (anchors, topic_starts)
}

fn topicpos_block(position: TopicPos, decompressed_block_size: usize) -> Option<i32> {
    if position.0 < 12 || decompressed_block_size == 0 {
        return None;
    }
    let relative = i64::from(position.0 - 12);
    let divisor = i64::try_from(decompressed_block_size).ok()?;
    i32::try_from(relative / divisor).ok()
}


fn first_displayable_presentation(presentations: &[TopicPresentation]) -> Option<usize> {
    presentations
        .iter()
        .position(|topic| !topic.non_scrolling.is_empty() || !topic.scrolling.is_empty())
        .or_else(|| (!presentations.is_empty()).then_some(0))
}

fn decode_records(
    records: &[TopicRecord],
    fonts: &FontTable,
    warnings: &mut Vec<String>,
) -> Vec<FormattedRecord> {
    let records = records.iter().collect::<Vec<_>>();
    decode_record_refs(&records, fonts, warnings)
}

/// Decodes every record of one region, carrying WinHelp's running font selection between them.
///
/// WinHlp32 initialises the selected font once per topic render (`0x41B05D`) and changes it only
/// on character opcode `0x80`, so a record whose first paragraph reuses the previous record's
/// font emits no font command at all. Decoding each record from descriptor 0 made those
/// paragraphs inherit the file's first descriptor, which is usually the bold heading face.
fn decode_record_refs(
    records: &[&TopicRecord],
    fonts: &FontTable,
    warnings: &mut Vec<String>,
) -> Vec<FormattedRecord> {
    let mut font_index = 0_u16;
    records
        .iter()
        .map(|record| match FormattedRecord::decode_with_font_table(record, &mut font_index, fonts) {
            Ok(formatted) if formatted.issues.is_empty() => formatted,
            Ok(formatted) => {
                let requires_fallback = formatted.issues.iter().any(|issue| !issue.layout_safe);
                for issue in &formatted.issues {
                    warnings.push(format!(
                        "TOPICPOS {} {:?} LinkData1+0x{:X}: {}",
                        record.position.0,
                        record.record_type,
                        issue.link_data1_offset,
                        issue.message
                    ));
                }
                if requires_fallback {
                    // A structurally unsafe unknown command can change the interpretation of every
                    // byte that follows it. Prefer complete visible text to misleading partial
                    // formatting. Exact bounded omissions (for example disabled hosted controls or
                    // unresolved hotspot action variants) remain layout-safe and keep their retained
                    // formatted representation instead.
                    let mut fallback = FormattedRecord::from_plain_text(&record.plain_text);
                    fallback.issues = formatted.issues;
                    fallback
                } else {
                    formatted
                }
            }
            Err(error) => {
                warnings.push(format!(
                    "TOPICPOS {} {:?}: {error}",
                    record.position.0, record.record_type
                ));
                FormattedRecord::from_plain_text(&record.plain_text)
            }
        })
        .collect()
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Inline, TopicPos, TopicRecordType, TopicRegion};

    #[test]
    fn direct_open_selects_first_displayable_topic_not_contents_metadata() {
        let blank = TopicPresentation {
            id: crate::TopicId(TopicPos(1)),
            title: "metadata-only".to_owned(),
            non_scrolling: Vec::new(),
            scrolling: Vec::new(),
            warnings: Vec::new(),
        };
        let visible = TopicPresentation {
            id: crate::TopicId(TopicPos(2)),
            title: "first visible".to_owned(),
            non_scrolling: Vec::new(),
            scrolling: vec![FormattedRecord::from_plain_text("hello")],
            warnings: Vec::new(),
        };
        assert_eq!(first_displayable_presentation(&[blank, visible]), Some(1));
    }

    #[test]
    fn unclassified_display_records_are_recovered_for_rendering() {
        let record = TopicRecord {
            position: TopicPos(96),
            record_type: TopicRecordType::Display,
            region: TopicRegion::Unclassified,
            link_data1: Vec::new(),
            link_data2: b"popup body\0".to_vec(),
            plain_text: "popup body".to_owned(),
        };
        let topic = Topic {
            id: crate::TopicId(TopicPos(12)),
            title: String::new(),
            macros: Vec::new(),
            metadata: Default::default(),
            non_scrolling: Vec::new(),
            scrolling: Vec::new(),
            unclassified: vec![record],
            plain_text: "popup body".to_owned(),
        };

        let presentation = TopicPresentation::from_topic(&topic, &FontTable::fallback());
        assert!(presentation.non_scrolling.is_empty());
        assert_eq!(presentation.scrolling.len(), 1);
        assert_eq!(formatted_text(&presentation.scrolling[0]), "popup body");
        assert!(presentation.warnings.iter().any(|warning| {
            warning.contains("recovered 1 display record(s)")
        }));
    }

    #[test]
    fn recovered_records_keep_topicpos_order_with_scrolling_body() {
        let scrolling = TopicRecord {
            position: TopicPos(160),
            record_type: TopicRecordType::Display,
            region: TopicRegion::Scrolling,
            link_data1: Vec::new(),
            link_data2: b"second\0".to_vec(),
            plain_text: "second".to_owned(),
        };
        let recovered = TopicRecord {
            position: TopicPos(128),
            record_type: TopicRecordType::Display,
            region: TopicRegion::Unclassified,
            link_data1: Vec::new(),
            link_data2: b"first\0".to_vec(),
            plain_text: "first".to_owned(),
        };
        let topic = Topic {
            id: crate::TopicId(TopicPos(12)),
            title: String::new(),
            macros: Vec::new(),
            metadata: Default::default(),
            non_scrolling: Vec::new(),
            scrolling: vec![scrolling],
            unclassified: vec![recovered],
            plain_text: "first\nsecond".to_owned(),
        };

        let presentation = TopicPresentation::from_topic(&topic, &FontTable::fallback());
        assert_eq!(presentation.scrolling.len(), 2);
        assert_eq!(formatted_text(&presentation.scrolling[0]), "first");
        assert_eq!(formatted_text(&presentation.scrolling[1]), "second");
    }

    #[test]
    fn layout_safe_special_record_warning_does_not_discard_retained_object() {
        let mut payload = vec![0, 0, 0, 0, 0, 0];
        payload.extend_from_slice(b"BUTTON\0");
        let encoded_size = u16::try_from((payload.len() + 16_384) * 2).unwrap();
        let mut link_data1 = encoded_size.to_le_bytes().to_vec();
        link_data1.push(0); // modern TopicLength 0
        link_data1.extend_from_slice(&payload);
        let record = TopicRecord {
            position: TopicPos(96),
            record_type: TopicRecordType::EmbeddedWindow,
            region: TopicRegion::Scrolling,
            link_data1,
            link_data2: Vec::new(),
            plain_text: String::new(),
        };
        let mut warnings = Vec::new();
        let decoded = decode_records(&[record], &FontTable::fallback(), &mut warnings);

        assert_eq!(decoded.len(), 1);
        assert!(warnings.iter().any(|warning| warning.contains("native authored control execution is disabled")));
        assert!(matches!(
            decoded[0].paragraphs[0].inlines.as_slice(),
            [Inline::EmbeddedWindow(_)]
        ));
    }

    fn formatted_text(record: &FormattedRecord) -> String {
        record
            .paragraphs
            .iter()
            .flat_map(|paragraph| paragraph.inlines.iter())
            .filter_map(|inline| match inline {
                Inline::Text(run) => Some(run.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .concat()
    }

    #[test]
    fn unsupported_formatting_opcode_falls_back_to_complete_plain_text() {
        // Signed-compressed TopicSize = 16, unsigned-compressed TopicLength = 1.
        let mut link_data1 = vec![0x20, 0x80, 2];
        // Minimal ParagraphInfo: compressed signed leading value 0, id = 1, no optional flags.
        link_data1.extend_from_slice(&[0, 0x80, 1, 0, 0, 0]);
        // Deliberately unknown character-formatting opcode.
        link_data1.push(0x90);

        let record = TopicRecord {
            position: TopicPos(12),
            record_type: TopicRecordType::Display,
            region: TopicRegion::Scrolling,
            link_data1,
            link_data2: b"visible text\0".to_vec(),
            plain_text: "visible text".to_owned(),
        };

        let mut warnings = Vec::new();
        let decoded = decode_records(&[record], &FontTable::fallback(), &mut warnings);
        assert_eq!(decoded.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert_eq!(decoded[0].issues.len(), 1);
        let text = decoded[0].paragraphs[0]
            .inlines
            .iter()
            .filter_map(|inline| match inline {
                Inline::Text(run) => Some(run.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .concat();
        assert_eq!(text, "visible text");
    }
}

// Browser-style history and cross-file path resolution live beside HelpDocument.
/// A concrete viewer position that can be restored by Back/Forward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationLocation {
    pub source_path: PathBuf,
    pub topic_index: usize,
    pub topic_offset: Option<TopicOffset>,
    pub window_name: Option<String>,
}

impl NavigationLocation {
    pub fn new(source_path: impl Into<PathBuf>, topic_index: usize) -> Self {
        Self {
            source_path: source_path.into(),
            topic_index,
            topic_offset: None,
            window_name: None,
        }
    }
}

/// Bounded browser-style history. Visiting after Back discards the forward branch.
#[derive(Debug, Clone)]
pub struct NavigationHistory {
    back: Vec<NavigationLocation>,
    forward: Vec<NavigationLocation>,
    capacity: usize,
}

impl Default for NavigationHistory {
    fn default() -> Self {
        Self::new(256)
    }
}

impl NavigationHistory {
    pub fn new(capacity: usize) -> Self {
        Self {
            back: Vec::new(),
            forward: Vec::new(),
            capacity: capacity.max(1),
        }
    }

    /// Returns the Back stack from oldest to newest. The last entry is the next Back destination.
    pub fn back_locations(&self) -> &[NavigationLocation] {
        &self.back
    }

    /// Returns the Forward stack from oldest to newest. The last entry is the next Forward destination.
    pub fn forward_locations(&self) -> &[NavigationLocation] {
        &self.forward
    }

    pub fn visit(&mut self, current: NavigationLocation, next: &NavigationLocation) {
        if &current == next {
            return;
        }
        self.push_back(current);
        self.forward.clear();
    }

    pub fn back(&mut self, current: NavigationLocation) -> Option<NavigationLocation> {
        let destination = self.back.pop()?;
        self.push_forward(current);
        Some(destination)
    }

    pub fn forward(&mut self, current: NavigationLocation) -> Option<NavigationLocation> {
        let destination = self.forward.pop()?;
        self.push_back(current);
        Some(destination)
    }

    pub fn clear(&mut self) {
        self.back.clear();
        self.forward.clear();
    }

    fn push_back(&mut self, location: NavigationLocation) {
        if self.back.len() >= self.capacity {
            self.back.remove(0);
        }
        self.back.push(location);
    }

    fn push_forward(&mut self, location: NavigationLocation) {
        if self.forward.len() >= self.capacity {
            self.forward.remove(0);
        }
        self.forward.push(location);
    }
}

/// Resolves an HLP filename carried by an external hotspot relative to the current HLP file.
pub fn resolve_external_help_path(current_help: &Path, target: &str) -> PathBuf {
    let normalized = if std::path::MAIN_SEPARATOR == '\\' {
        target.replace('/', "\\")
    } else {
        target.replace('\\', "/")
    };
    let candidate = PathBuf::from(normalized);
    if candidate.is_absolute() {
        candidate
    } else {
        current_help
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(candidate)
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    #[test]
    fn cross_file_history_round_trips_locations() {
        let mut history = NavigationHistory::new(8);
        let a = NavigationLocation::new("a.hlp", 3);
        let b = NavigationLocation::new("sub/b.hlp", 7);
        history.visit(a.clone(), &b);
        assert_eq!(history.back(b.clone()), Some(a.clone()));
        assert_eq!(history.forward(a), Some(b));
    }

    #[test]
    fn external_help_paths_are_relative_to_the_current_help_file() {
        let resolved = resolve_external_help_path(
            Path::new("docs/current.hlp"),
            "related/other.hlp",
        );
        assert_eq!(resolved, PathBuf::from("docs/related/other.hlp"));
    }

    #[test]
    fn history_discards_forward_branch_after_new_visit() {
        let mut history = NavigationHistory::new(4);
        let a = NavigationLocation::new("a.hlp", 0);
        let b = NavigationLocation::new("a.hlp", 1);
        let c = NavigationLocation::new("a.hlp", 2);
        history.visit(a.clone(), &b);
        assert_eq!(history.back(b.clone()), Some(a.clone()));
        assert!(!history.forward_locations().is_empty());
        history.visit(a, &c);
        assert!(history.forward_locations().is_empty());
    }
}

