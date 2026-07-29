//! Deterministic retained layout for WinHelp topic presentations.
//!
//! The layout engine deliberately has no wxWidgets dependency. Native drawing code receives a
//! list of positioned boxes, so wrapping and hyperlink hit-testing can be tested headlessly.

use crate::TopicPresentation;
use crate::graphics::{PictureSizing, metafile_pixel_dimensions};
use crate::{
    BorderFlags, BorderStyle, DecodedPicture, EmbeddedWindowReference, FontDescriptor, FontMetric,
    FontTable, FormattedRecord, FormattedTable, HlpFontFamily, Hotspot, Inline, Paragraph,
    ParagraphAlignment, ParagraphFormat, PicturePosition, PictureReference, Rgb, TabAlignment,
    TableCell, TableCellContent, TableInfo, TextRun,
};

const PAGE_MARGIN: i32 = 12;
const DEFAULT_DPI: i32 = 96;
const PICTURE_WIDTH: i32 = 128;
const PICTURE_HEIGHT: i32 = 80;
const STANDARD_BUTTON_INITIAL_WIDTH: i32 = 30;
const STANDARD_BUTTON_EMPTY_SIZE: i32 = 12;
const STANDARD_BUTTON_INITIAL_LABEL_HEIGHT: i32 = 16;
/// Extra breathing room between an authored rule-only display record and the table that follows.
const TABLE_AFTER_RULE_GAP: i32 = 8;
/// Additional vertical separation before the classic Related Topics ALink row.
const RELATED_TOPICS_AFTER_RULE_GAP: i32 = 4;

/// Integer two-dimensional point in document coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// Integer rectangle used for retained layout and hyperlink hit-testing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    /// Reports whether a point falls inside this half-open rectangle.
    pub const fn contains(self, point: Point) -> bool {
        point.x >= self.x
            && point.y >= self.y
            && point.x < self.x.saturating_add(self.width)
            && point.y < self.y.saturating_add(self.height)
    }
}

/// Coarse font family retained for native platform font selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedFontFamily {
    /// Let the GUI toolkit choose the platform's normal proportional UI/document font.
    Proportional,
    /// Let the GUI toolkit choose the platform's normal fixed-width font.
    Monospace,
}

/// Native-paint-ready font and colour values resolved from one `|FONT` descriptor.
///
/// `face_name` retains the original HLP typeface for diagnostics and symbol/decorative faces.
/// The viewer modernizes ordinary faces while preserving the source WinHelp family classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTextStyle {
    pub face_name: String,
    pub family: ResolvedFontFamily,
    /// Original WinHelp family classification retained for native face substitution.
    pub source_family: HlpFontFamily,
    pub point_size: i32,
    /// Authored size in twentieths of a point. Keeping this avoids losing HC30 half-point
    /// precision before native zoom/font creation.
    pub point_size_twips: i32,
    pub weight: i16,
    pub italic: bool,
    pub underline: bool,
    pub strike_out: bool,
    /// HC30 attribute 0x20. Microsoft WinHlp32 renders this with a 2/3-height font.
    pub small_caps: bool,
    pub foreground: Rgb,
    /// True when the descriptor uses WinHlp32's exact RGB(1,1,0) "inherit" sentinel.
    pub foreground_inherits: bool,
    pub background: Rgb,
    /// True when the descriptor background uses the same WinHlp32 inheritance sentinel.
    pub background_inherits: bool,
    /// Per-face GDI charset selected from `|SYSTEM` record 11 when the HLP provides it.
    /// WinHlp32 uses 0xB1/0xB2 to identify Hebrew/Arabic layout records.
    pub charset: Option<u8>,
}

/// Pixel dimensions returned by a text-measurement backend.
///
/// The engine crate remains GUI-independent: the viewer can supply native wxWidgets/GDI
/// measurements while headless tests use the deterministic fallback measurer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextMetrics {
    pub width: i32,
    pub height: i32,
    /// Distance in pixels from the top of the measured text cell to the font baseline.
    /// Native callers should derive this from ascent/descent metrics rather than glyph bounds.
    pub baseline: i32,
}

/// Retained object painted by the wxDragon topic canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutKind {
    Text {
        text: String,
        style: ResolvedTextStyle,
        hotspot: Option<Hotspot>,
        /// Baseline offset from `bounds.y`, retained from the measurement backend.
        baseline: i32,
        /// Semantic paragraph/line identity used by the HTML exporter to reflow browser-shaped
        /// text without treating WinHelp's own automatic wraps as authored hard breaks.
        flow: TextFlow,
    },
    Picture {
        image: DecodedPicture,
    },
    /// Invisible clickable rectangle over a decoded graphical hotspot.
    PictureHotspot {
        hotspot: Hotspot,
    },
    PicturePlaceholder,
    /// Safe visual substitute for an authored 0x05/0x24 native custom control. The original
    /// WinHlp32 creates a child HWND, which this cross-platform viewer deliberately never runs.
    EmbeddedWindowPlaceholder {
        descriptor: String,
        /// `Some` identifies WinHlp32's built-in `!label,macro` BUTTON form. The retained label
        /// may be empty, as it is in CALC.HLP's Related Topics control. `None` is a generic,
        /// safely non-executing placeholder for other hosted-window descriptors.
        standard_button_label: Option<String>,
        /// Viewer-local macro hotspot for the built-in standard-button form.
        hotspot: Option<Hotspot>,
    },
    Border {
        flags: BorderFlags,
        style: BorderStyle,
    },
}

/// Semantic flow metadata retained beside each text token.
///
/// Native painting remains fully retained-mode and ignores this structure. HTML export uses it to
/// preserve explicit line breaks and tab/hanging-indent segments while allowing the browser to
/// choose only the automatic word wraps required by its naturally shaped font metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextFlow {
    pub paragraph_id: usize,
    pub line_index: usize,
    pub hard_break_before: bool,
    pub segment_index: usize,
    pub no_wrap: bool,
    pub reflow_safe: bool,
    pub content_left: i32,
    pub content_right: i32,
}

/// One positioned retained object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutBox {
    pub bounds: Rect,
    pub kind: LayoutKind,
}

impl LayoutBox {
    /// Returns this box's hotspot when it is clickable text.
    pub fn hotspot(&self) -> Option<&Hotspot> {
        match &self.kind {
            LayoutKind::Text { hotspot, .. } => hotspot.as_ref(),
            LayoutKind::PictureHotspot { hotspot } => Some(hotspot),
            LayoutKind::EmbeddedWindowPlaceholder { hotspot, .. } => hotspot.as_ref(),
            LayoutKind::Picture { .. }
            | LayoutKind::PicturePlaceholder
            | LayoutKind::Border { .. } => None,
        }
    }
}

/// Layout of one independently painted WinHelp region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionLayout {
    pub width: i32,
    pub height: i32,
    pub boxes: Vec<LayoutBox>,
}

impl RegionLayout {
    /// Returns the clickable retained box at a point in front-to-back order.
    pub fn hit_test_box(&self, point: Point) -> Option<&LayoutBox> {
        self.boxes
            .iter()
            .rev()
            .find(|item| item.bounds.contains(point) && item.hotspot().is_some())
    }

    /// Hit-tests clickable text in front-to-back retained order.
    pub fn hit_test(&self, point: Point) -> Option<&Hotspot> {
        self.hit_test_box(point).and_then(LayoutBox::hotspot)
    }
}

/// Fixed and scrolling layouts for one topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicLayout {
    pub topic_title: String,
    pub fixed: RegionLayout,
    pub scrolling: RegionLayout,
}

/// Deterministic layout engine. Device DPI is retained per axis because WinHlp32 uses
/// horizontal resolution for x geometry (indents, tabs, tables) and vertical resolution for
/// y geometry (paragraph/line spacing and font-height fallbacks).
#[derive(Debug, Clone, Copy)]
pub struct LayoutEngine {
    dpi_x: i32,
    dpi_y: i32,
    /// Viewer text zoom applied only to authored line-advance metrics. Native text measurement
    /// already includes zoom, but exact/minimum WinHelp line spacing must scale with it too or
    /// enlarged glyph cells can collide vertically.
    text_zoom_percent: i32,
}

impl Default for LayoutEngine {
    fn default() -> Self {
        Self::with_dpi(DEFAULT_DPI, DEFAULT_DPI)
    }
}

impl LayoutEngine {
    /// Constructs a square-DPI engine for compatibility with existing callers/tests.
    pub const fn new(dpi: i32) -> Self {
        Self::with_dpi(dpi, dpi)
    }

    /// Constructs an engine with the actual horizontal and vertical device resolution.
    pub const fn with_dpi(dpi_x: i32, dpi_y: i32) -> Self {
        Self { dpi_x, dpi_y, text_zoom_percent: 100 }
    }

    /// Constructs an engine for the native viewer, retaining device DPI independently from the
    /// user-selected text zoom. DPI continues to describe physical device geometry; zoom affects
    /// only text-driven line advance so pictures and authored physical sizes are not magnified.
    pub const fn with_dpi_and_text_zoom(
        dpi_x: i32,
        dpi_y: i32,
        text_zoom_percent: i32,
    ) -> Self {
        Self { dpi_x, dpi_y, text_zoom_percent }
    }

    /// Lays out both WinHelp regions using the deterministic headless text measurer.
    ///
    /// GUI front ends should prefer [`Self::layout_topic_with_measurer`] so the same native font
    /// metrics used for painting also determine word positions and wrapping.
    pub fn layout_topic(
        &self,
        topic: &TopicPresentation,
        fonts: &FontTable,
        viewport_width: i32,
    ) -> TopicLayout {
        let mut measure = |style: &ResolvedTextStyle, text: &str| {
            self.approximate_text_metrics(style, text)
        };
        self.layout_topic_with_measurer(topic, fonts, viewport_width, &mut measure)
    }

    /// Lays out both WinHelp regions using caller-supplied text metrics.
    ///
    /// This is the normal path for the native viewer. Supplying the same GDI/wxWidgets metrics
    /// used for painting prevents retained coordinates from drifting away from rendered glyphs.
    pub fn layout_topic_with_measurer<F>(
        &self,
        topic: &TopicPresentation,
        fonts: &FontTable,
        viewport_width: i32,
        measure: &mut F,
    ) -> TopicLayout
    where
        F: FnMut(&ResolvedTextStyle, &str) -> TextMetrics,
    {
        let width = viewport_width.max(PAGE_MARGIN * 2 + 32);
        TopicLayout {
            topic_title: topic.title.clone(),
            fixed: self.layout_region_with_measurer(&topic.non_scrolling, fonts, width, measure),
            scrolling: self.layout_region_with_measurer(&topic.scrolling, fonts, width, measure),
        }
    }

    /// Lays out a sequence of display/table records with the deterministic fallback measurer.
    pub fn layout_region(
        &self,
        records: &[FormattedRecord],
        fonts: &FontTable,
        width: i32,
    ) -> RegionLayout {
        let mut measure = |style: &ResolvedTextStyle, text: &str| {
            self.approximate_text_metrics(style, text)
        };
        self.layout_region_with_measurer(records, fonts, width, &mut measure)
    }

    fn layout_region_with_measurer<F>(
        &self,
        records: &[FormattedRecord],
        fonts: &FontTable,
        width: i32,
        measure: &mut F,
    ) -> RegionLayout
    where
        F: FnMut(&ResolvedTextStyle, &str) -> TextMetrics,
    {
        let mut boxes = Vec::new();
        let mut y = PAGE_MARGIN;
        let mut previous_was_rule_only = false;
        let mut previous_rule_left = None::<i32>;
        for record in records {
            let record_box_start = boxes.len();
            // CALC.HLP and other WinHelp files commonly place an authored rule-only display
            // record immediately before a table. Keep the rule geometry intact, but leave a
            // small viewer-side breathing space before the first table row. This is deliberately
            // keyed to the display->table boundary so it cannot accumulate between table rows.
            if record.table.is_some() && previous_was_rule_only {
                y = y.saturating_add(TABLE_AFTER_RULE_GAP);
            }
            if previous_rule_left.is_some() && record_has_standard_alink_button(record) {
                y = y.saturating_add(RELATED_TOPICS_AFTER_RULE_GAP);
            }

            let record_height = if let Some(table) = &record.table {
                self.layout_table(record, table, fonts, width, y, &mut boxes, measure)
            } else {
                self.layout_paragraphs(
                    &record.paragraphs,
                    fonts,
                    width,
                    PAGE_MARGIN,
                    y,
                    &mut boxes,
                    measure,
                )
            };

            // A stock ALink row retains its authored paragraph indents. WinHlp32's final visual
            // result is instead anchored to the preceding rendered rule, whose x-coordinate may
            // differ from the region margin depending on the authored border form. Move only the
            // ALink button and text on its visual line after ordinary paragraph layout has run.
            if record_has_standard_alink_button(record) {
                if let Some(rule_left) = previous_rule_left {
                    align_standard_alink_row_to_rule(
                        &mut boxes[record_box_start..],
                        rule_left,
                    );
                }
            }

            y = y.saturating_add(record_height.max(1));
            previous_was_rule_only = record_is_rule_only(record);
            previous_rule_left = if previous_was_rule_only {
                boxes[record_box_start..].iter().find_map(|item| {
                    matches!(&item.kind, LayoutKind::Border { .. }).then_some(item.bounds.x)
                })
            } else {
                None
            };
        }
        RegionLayout {
            width,
            height: y.saturating_add(PAGE_MARGIN).max(1),
            boxes,
        }
    }

    fn layout_paragraphs<F>(
        &self,
        paragraphs: &[Paragraph],
        fonts: &FontTable,
        right_edge: i32,
        left_edge: i32,
        start_y: i32,
        boxes: &mut Vec<LayoutBox>,
        measure: &mut F,
    ) -> i32
    where
        F: FnMut(&ResolvedTextStyle, &str) -> TextMetrics,
    {
        let mut y = start_y;
        for paragraph in paragraphs {
            let available = (right_edge - left_edge).max(16);
            let height = self.layout_paragraph(
                paragraph,
                fonts,
                left_edge,
                y,
                available,
                boxes,
                measure,
            );
            y = y.saturating_add(height);
        }
        y.saturating_sub(start_y)
    }

    fn layout_table<F>(
        &self,
        record: &FormattedRecord,
        table: &TableInfo,
        fonts: &FontTable,
        width: i32,
        start_y: i32,
        boxes: &mut Vec<LayoutBox>,
        measure: &mut F,
    ) -> i32
    where
        F: FnMut(&ResolvedTextStyle, &str) -> TextMetrics,
    {
        if table.columns.is_empty() {
            return self.layout_paragraphs(
                &record.paragraphs,
                fonts,
                width,
                PAGE_MARGIN,
                start_y,
                boxes,
                measure,
            );
        }

        let available_width = width.saturating_sub(PAGE_MARGIN * 2).max(1);
        if record.table_cells.is_empty() {
            // Compatibility path for hand-constructed FormattedRecord values and older callers.
            // Parsed table records always carry the recursive cell tree from build-fix 13 onward.
            return self.layout_flat_table_paragraphs(
                table,
                &record.paragraphs,
                fonts,
                PAGE_MARGIN,
                available_width,
                start_y,
                boxes,
                measure,
            );
        }

        self.layout_table_cells(
            table,
            &record.table_cells,
            &record.paragraphs,
            fonts,
            PAGE_MARGIN,
            available_width,
            start_y,
            boxes,
            measure,
        )
    }

    /// Lays out the exact recursive table-cell tree used by WinHlp32's generic record dispatcher.
    ///
    /// At 0x4151B6 the Microsoft table walker calls dispatcher 0x417578 for each cell. A nested
    /// table dispatches straight back to 0x414F66. The child receives the parent column's x/y
    /// origin and width, and the returned height at state offset +0x14 is added only to that
    /// parent column's cumulative y cursor (0x4151DC..0x4151E9). This function mirrors that call
    /// graph recursively rather than flattening nested tables into their parent's paragraphs.
    fn layout_table_cells<F>(
        &self,
        table: &TableInfo,
        cells: &[TableCell],
        paragraphs: &[Paragraph],
        fonts: &FontTable,
        origin_x: i32,
        available_width: i32,
        start_y: i32,
        boxes: &mut Vec<LayoutBox>,
        measure: &mut F,
    ) -> i32
    where
        F: FnMut(&ResolvedTextStyle, &str) -> TextMetrics,
    {
        if table.columns.is_empty() {
            return 0;
        }

        let columns = self.table_column_geometry(table, available_width.max(1));
        let mut column_y = vec![0_i32; columns.len()];

        for cell in cells {
            let column = usize::try_from(cell.column.max(0)).unwrap_or(0).min(columns.len() - 1);
            let geometry = columns[column];
            let cell_x = origin_x.saturating_add(geometry.x);
            let cell_y = start_y.saturating_add(column_y[column]);
            let cell_width = geometry.width.max(1);

            let height = match &cell.content {
                TableCellContent::Display {
                    paragraph_start,
                    paragraph_end,
                } => {
                    let start = (*paragraph_start).min(paragraphs.len());
                    let end = (*paragraph_end).min(paragraphs.len()).max(start);
                    self.layout_table_cell_paragraphs(
                        &paragraphs[start..end],
                        fonts,
                        cell_x.saturating_add(cell_width),
                        cell_x,
                        cell_y,
                        boxes,
                        measure,
                    )
                }
                TableCellContent::Picture(picture) => {
                    let (picture_width, picture_height) = self.picture_display_size(picture, cell_width);
                    let bounds = Rect {
                        x: cell_x,
                        y: cell_y,
                        width: picture_width,
                        height: picture_height,
                    };
                    push_picture_boxes(boxes, picture, bounds);
                    picture_height
                }
                TableCellContent::Table(nested) => self.layout_nested_table(
                    nested,
                    paragraphs,
                    fonts,
                    cell_x,
                    cell_width,
                    cell_y,
                    boxes,
                    measure,
                ),
                TableCellContent::EmbeddedWindow(window) => {
                    let (width, height, standard_button_label) =
                        embedded_window_geometry(window, cell_width, self.dpi_x, self.dpi_y);
                    boxes.push(LayoutBox {
                        bounds: Rect {
                            x: cell_x,
                            y: cell_y,
                            width,
                            height,
                        },
                        kind: LayoutKind::EmbeddedWindowPlaceholder {
                            descriptor: window.descriptor.clone(),
                            standard_button_label,
                            hotspot: embedded_window_hotspot(window),
                        },
                    });
                    height
                }
                TableCellContent::NoRender { .. } | TableCellContent::Unsupported { .. } => 0,
            };

            // WinHlp32 adds the renderer-returned cell height directly to this one column.
            column_y[column] = column_y[column].saturating_add(height);
        }

        column_y.into_iter().max().unwrap_or(0)
    }

    /// Lays out display paragraphs inside one table cell using WinHlp32's empty-record rule.
    ///
    /// The reference display path at 0x415B44..0x415BA1 recognizes an empty LinkData2 string
    /// followed immediately by the paragraph terminator and returns before applying paragraph
    /// spacing or a text-line height. Table producers use those zero-height display records as
    /// structural fillers between independently flowing columns. Giving them a synthetic blank
    /// line shifts later cells in only that column and produces the staggered CALC.HLP table.
    fn layout_table_cell_paragraphs<F>(
        &self,
        paragraphs: &[Paragraph],
        fonts: &FontTable,
        right_edge: i32,
        left_edge: i32,
        start_y: i32,
        boxes: &mut Vec<LayoutBox>,
        measure: &mut F,
    ) -> i32
    where
        F: FnMut(&ResolvedTextStyle, &str) -> TextMetrics,
    {
        let mut y = start_y;
        for paragraph in paragraphs {
            if table_cell_paragraph_is_empty_filler(paragraph) {
                continue;
            }
            let available = (right_edge - left_edge).max(16);
            let height = self.layout_paragraph(
                paragraph,
                fonts,
                left_edge,
                y,
                available,
                boxes,
                measure,
            );
            y = y.saturating_add(height);
        }
        y.saturating_sub(start_y)
    }

    fn layout_nested_table<F>(
        &self,
        nested: &FormattedTable,
        paragraphs: &[Paragraph],
        fonts: &FontTable,
        origin_x: i32,
        available_width: i32,
        start_y: i32,
        boxes: &mut Vec<LayoutBox>,
        measure: &mut F,
    ) -> i32
    where
        F: FnMut(&ResolvedTextStyle, &str) -> TextMetrics,
    {
        self.layout_table_cells(
            &nested.info,
            &nested.cells,
            paragraphs,
            fonts,
            origin_x,
            available_width,
            start_y,
            boxes,
            measure,
        )
    }

    /// Compatibility layout for table records created by code predating retained cell trees.
    fn layout_flat_table_paragraphs<F>(
        &self,
        table: &TableInfo,
        paragraphs: &[Paragraph],
        fonts: &FontTable,
        origin_x: i32,
        available_width: i32,
        start_y: i32,
        boxes: &mut Vec<LayoutBox>,
        measure: &mut F,
    ) -> i32
    where
        F: FnMut(&ResolvedTextStyle, &str) -> TextMetrics,
    {
        let columns = self.table_column_geometry(table, available_width);
        let mut column_y = vec![0_i32; columns.len()];

        for paragraph in paragraphs {
            let column = paragraph.format.column.unwrap_or(0).max(0) as usize;
            let column = column.min(columns.len() - 1);
            let geometry = columns[column];
            let cell_x = origin_x.saturating_add(geometry.x);
            let cell_y = start_y.saturating_add(column_y[column]);
            let height = self.layout_paragraph(
                paragraph,
                fonts,
                cell_x,
                cell_y,
                geometry.width.max(1),
                boxes,
                measure,
            );
            column_y[column] = column_y[column].saturating_add(height.max(1));
        }

        column_y.into_iter().max().unwrap_or(0)
    }

    /// Converts the WinHelp table header into device-pixel x positions and widths.
    ///
    /// KB917607 WinHlp32 0x414f66 has two distinct modes:
    /// * type 0: unsigned source values are first converted with `raw * dpi_x / 144`, then
    ///   proportionally mapped from a 32767-unit reference span to
    ///   `max(available_width, minimum_width_px)`;
    /// * nonzero types: the unsigned source metrics are absolute `raw * dpi_x / 144` values.
    ///
    /// Gap-before-column is applied before storing the column x position. Width follows it.
    fn table_column_geometry(&self, table: &TableInfo, available_width: i32) -> Vec<TableColumnGeometry> {
        let effective_width = if table.table_type == 0 {
            let minimum = table
                .minimum_width
                .map_or(0, |raw| self.table_absolute_metric(raw));
            available_width.max(minimum)
        } else {
            available_width
        };

        // The Microsoft code performs the DPI conversion before the proportional scale, so retain
        // that two-stage integer truncation instead of algebraically collapsing the expression.
        let reference_width = self.table_absolute_metric(32_767).max(1);
        let mut cursor = 0_i32;
        let mut result = Vec::with_capacity(table.columns.len());
        for column in &table.columns {
            let gap = if table.table_type == 0 {
                self.table_relative_metric(column.gap_before, effective_width, reference_width)
            } else {
                self.table_absolute_metric(column.gap_before)
            };
            cursor = cursor.saturating_add(gap);

            let width = if table.table_type == 0 {
                self.table_relative_metric(column.width, effective_width, reference_width)
            } else {
                self.table_absolute_metric(column.width)
            };
            result.push(TableColumnGeometry {
                x: cursor,
                width,
            });
            cursor = cursor.saturating_add(width);
        }
        result
    }

    fn table_absolute_metric(&self, raw: u16) -> i32 {
        let scaled = i64::from(raw) * i64::from(self.dpi_x.max(1));
        i32::try_from(scaled / 144).unwrap_or(i32::MAX)
    }

    fn table_relative_metric(&self, raw: u16, effective_width: i32, reference_width: i32) -> i32 {
        let physical = self.table_absolute_metric(raw);
        let scaled = i64::from(physical) * i64::from(effective_width.max(0));
        i32::try_from(scaled / i64::from(reference_width.max(1))).unwrap_or(i32::MAX)
    }

    fn layout_paragraph<F>(
        &self,
        paragraph: &Paragraph,
        fonts: &FontTable,
        origin_x: i32,
        origin_y: i32,
        width: i32,
        boxes: &mut Vec<LayoutBox>,
        measure: &mut F,
    ) -> i32
    where
        F: FnMut(&ResolvedTextStyle, &str) -> TextMetrics,
    {
        let format = &paragraph.format;
        let rtl_locale = fonts.is_rtl_locale();
        let border_only = format.border.is_some()
            && paragraph
                .inlines
                .iter()
                .all(|inline| matches!(inline, Inline::Control85(_)));
        // KB917607 WinHlp32 0x412664..0x41285d converts every paragraph metric
        // independently of |FONT| generation as raw * device-DPI / 144.
        // These are signed in WinHlp32. Negative spacing and negative left/right indents
        // are preserved rather than clamped away (0x415bae, 0x415bd1, 0x415f18).
        let left_indent = self.optional_horizontal_paragraph_metric(format.left_indent);
        let right_indent = self.optional_horizontal_paragraph_metric(format.right_indent);
        let first_indent = self.optional_horizontal_paragraph_metric(format.first_line_indent);
        let spacing_above = self.optional_vertical_paragraph_metric(format.spacing_above);
        let spacing_below = self.optional_vertical_paragraph_metric(format.spacing_below);

        // WinHlp32 reserves fixed device-pixel clearance between paragraph borders and text.
        // 0x415320 returns 5 px for styles 0/4, 6 px for styles 1/3, and 7 px for style 2.
        let border_clearance = format
            .border
            .map_or(0, |border| reference_border_clearance(border.style));
        let border_flags = format.border.map(|border| border.flags).unwrap_or_default();
        // A compact border-only top+bottom paragraph is the common WinHelp separator immediately
        // before headings such as “Related Topics”. The retained flags describe two edges, but in
        // the integrated viewer we deliberately present it as one header rule. Align that rule to
        // the region edge (the following heading's baseline origin) and reserve a clean 12 px gap
        // below it rather than drawing a second horizontal edge.
        let horizontal_separator = border_only
            && !border_flags.box_all
            && border_flags.top
            && border_flags.bottom
            && !border_flags.left
            && !border_flags.right;
        let top_clearance = if horizontal_separator {
            0
        } else if border_flags.box_all || border_flags.top {
            border_clearance
        } else {
            0
        };
        let left_clearance = if border_flags.box_all || border_flags.left { border_clearance } else { 0 };
        let bottom_clearance = if horizontal_separator {
            12
        } else if border_flags.box_all || border_flags.bottom {
            border_clearance
        } else {
            0
        };
        let right_clearance = if border_flags.box_all || border_flags.right { border_clearance } else { 0 };

        let border_left = if horizontal_separator {
            origin_x
        } else {
            origin_x.saturating_add(left_indent)
        };
        let border_right = origin_x
            .saturating_add(width)
            .saturating_sub(right_indent)
            .max(border_left.saturating_add(1));
        let content_left = border_left.saturating_add(left_clearance);
        let content_right = border_right.saturating_sub(right_clearance).max(content_left.saturating_add(16));
        let paragraph_top = origin_y.saturating_add(spacing_above);
        let mut y = paragraph_top.saturating_add(top_clearance);
        let mut first_line = true;
        // Empty lines need a fallback, but populated lines must advance by their measured native
        // height. The old unconditional 16-pixel floor visibly over-spaced 7-8 pt WinHelp text.
        let mut last_text_line_height = None::<i32>;
        let mut floats = Vec::<ActiveFloat>::new();
        let mut pending_tab = None::<PendingTab>;
        // Flow identity is retained solely so HTML export can distinguish authored breaks from
        // automatic WinHelp wraps. Native painting still consumes only the absolute boxes.
        let paragraph_id = boxes.len();
        let tab_count = paragraph
            .inlines
            .iter()
            .filter(|inline| matches!(*inline, Inline::Tab))
            .count();
        let reflow_safe = !format.no_wrap
            && !format.right_to_left
            && tab_count <= 1
            && !paragraph.inlines.iter().any(|inline| {
                matches!(
                    inline,
                    Inline::Picture(_) | Inline::EmbeddedWindow(_) | Inline::Control85(_)
                )
            });
        let mut flow_line_index = 0usize;
        let mut flow_hard_break_before = false;
        let mut flow_segment_index = 0usize;
        // Paragraph bit 13 selects WinHlp32's RTL base-direction path. Its first-line indent is
        // applied from the right edge (0x415d1c..0x415d35), not from the left. At line finish,
        // Arabic/Hebrew locales also receive the reference's per-face charset run reordering.
        let first_line_right = if format.right_to_left {
            content_right.saturating_sub(first_indent)
        } else {
            content_right
        };
        let (mut line, mut line_left, mut line_right) = constrained_line(
            content_left,
            first_line_right,
            y,
            if format.right_to_left { 0 } else { first_indent },
            boxes.len(),
            &floats,
        );

        for inline in &paragraph.inlines {
            match inline {
                Inline::Text(run) => {
                    let style = resolve_style(run, fonts);
                    for token in tokenize_for_charset(&run.text, style.charset) {
                        let metrics = normalized_metrics(
                            measure(&style, token),
                            self.default_line_height(&style),
                        );
                        let token_width = metrics.width;
                        last_text_line_height = Some(metrics.height.max(1));
                        if !format.no_wrap
                            && !token.trim().is_empty()
                            && line.x > line.start_x
                            && line.x.saturating_add(token_width) > line_right
                        {
                            resolve_pending_tab(&mut pending_tab, &mut line, boxes);
                            self.finish_line(format, line_left, line_right, &mut line, boxes, rtl_locale);
                            let natural = if line.height > 0 { line.height } else { metrics.height };
                            y = line.y.saturating_add(self.line_advance(format, natural));
                            first_line = false;
                            flow_line_index = flow_line_index.saturating_add(1);
                            flow_hard_break_before = false;
                            (line, line_left, line_right) = constrained_line(
                                content_left,
                                content_right,
                                y,
                                0,
                                boxes.len(),
                                &floats,
                            );
                        }
                        boxes.push(LayoutBox {
                            bounds: Rect {
                                x: line.x,
                                y: line.y,
                                width: token_width,
                                height: metrics.height,
                            },
                            kind: LayoutKind::Text {
                                text: token.to_owned(),
                                style: style.clone(),
                                hotspot: run.hotspot.clone(),
                                baseline: metrics.baseline,
                                flow: TextFlow {
                                    paragraph_id,
                                    line_index: flow_line_index,
                                    hard_break_before: flow_hard_break_before,
                                    segment_index: flow_segment_index,
                                    no_wrap: format.no_wrap,
                                    reflow_safe,
                                    content_left,
                                    content_right,
                                },
                            },
                        });
                        line.box_end = boxes.len();
                        line.x = line.x.saturating_add(token_width);
                        line.height = line.height.max(metrics.height);
                    }
                }
                Inline::LineBreak => {
                    resolve_pending_tab(&mut pending_tab, &mut line, boxes);
                    self.finish_line(format, line_left, line_right, &mut line, boxes, rtl_locale);
                    let natural = if line.height > 0 {
                        line.height
                    } else {
                        last_text_line_height.unwrap_or(16)
                    };
                    y = line.y.saturating_add(self.line_advance(format, natural));
                    first_line = false;
                    flow_line_index = flow_line_index.saturating_add(1);
                    flow_hard_break_before = true;
                    (line, line_left, line_right) = constrained_line(
                        content_left,
                        content_right,
                        y,
                        0,
                        boxes.len(),
                        &floats,
                    );
                }
                Inline::Tab => {
                    // WinHlp32 resolves an earlier right/center tab only when the following run
                    // is known, then searches custom stops before falling back to the authored
                    // default interval (72 source units when bit 7 is absent).
                    resolve_pending_tab(&mut pending_tab, &mut line, boxes);
                    flow_segment_index = flow_segment_index.saturating_add(1);
                    let mut target = next_tab_target(
                        format,
                        line.x.saturating_sub(content_left),
                        self,
                    );
                    let mut tab_x = content_left.saturating_add(target.position);
                    if !format.no_wrap && tab_x > line_right {
                        self.finish_line(format, line_left, line_right, &mut line, boxes, rtl_locale);
                        let natural = if line.height > 0 {
                            line.height
                        } else {
                            last_text_line_height.unwrap_or(16)
                        };
                        y = line.y.saturating_add(self.line_advance(format, natural));
                        first_line = false;
                        flow_line_index = flow_line_index.saturating_add(1);
                        flow_hard_break_before = false;
                        (line, line_left, line_right) = constrained_line(
                            content_left,
                            content_right,
                            y,
                            0,
                            boxes.len(),
                            &floats,
                        );
                        target = next_tab_target(
                            format,
                            line.x.saturating_sub(content_left),
                            self,
                        );
                        tab_x = content_left.saturating_add(target.position);
                    }
                    match target.alignment {
                        TabAlignment::Right | TabAlignment::Center => {
                            pending_tab = Some(PendingTab {
                                alignment: target.alignment,
                                target_x: tab_x,
                                box_start: boxes.len(),
                            });
                        }
                        TabAlignment::Left | TabAlignment::Unknown(_) => {
                            line.x = tab_x.max(line.start_x);
                        }
                    }
                }
                Inline::Picture(picture) => match picture.position {
                    PicturePosition::Inline => {
                        let available_line_width = line_right.saturating_sub(line_left).max(1);
                        let (mut picture_width, mut picture_height) =
                            self.picture_display_size(picture, available_line_width);
                        if !format.no_wrap
                            && line.x > line.start_x
                            && line.x.saturating_add(picture_width) > line_right
                        {
                            resolve_pending_tab(&mut pending_tab, &mut line, boxes);
                            self.finish_line(format, line_left, line_right, &mut line, boxes, rtl_locale);
                            let natural = if line.height > 0 {
                                line.height
                            } else {
                                last_text_line_height.unwrap_or(16)
                            };
                            y = line.y.saturating_add(self.line_advance(format, natural));
                            first_line = false;
                            flow_line_index = flow_line_index.saturating_add(1);
                            flow_hard_break_before = false;
                            (line, line_left, line_right) = constrained_line(
                                content_left,
                                content_right,
                                y,
                                0,
                                boxes.len(),
                                &floats,
                            );
                            (picture_width, picture_height) = self.picture_display_size(
                                picture,
                                line_right.saturating_sub(line_left).max(1),
                            );
                        }
                        let bounds = Rect {
                            x: line.x,
                            y: line.y,
                            width: picture_width,
                            height: picture_height,
                        };
                        push_picture_boxes(boxes, picture, bounds);
                        line.box_end = boxes.len();
                        line.x = line.x.saturating_add(picture_width);
                        line.height = line.height.max(picture_height);
                    }
                    PicturePosition::FloatLeft | PicturePosition::FloatRight => {
                        // A float starts at the next line boundary when preceding content already
                        // occupies this line. At paragraph start, following text may immediately
                        // share the same vertical band with the picture.
                        if line.box_start < line.box_end {
                            resolve_pending_tab(&mut pending_tab, &mut line, boxes);
                            self.finish_line(format, line_left, line_right, &mut line, boxes, rtl_locale);
                            let natural = if line.height > 0 {
                                line.height
                            } else {
                                last_text_line_height.unwrap_or(16)
                            };
                            y = line.y.saturating_add(self.line_advance(format, natural));
                            first_line = false;
                            flow_line_index = flow_line_index.saturating_add(1);
                            flow_hard_break_before = false;
                            (line, line_left, line_right) = constrained_line(
                                content_left,
                                content_right,
                                y,
                                0,
                                boxes.len(),
                                &floats,
                            );
                        }

                        let span = line_right.saturating_sub(line_left).max(1);
                        let (picture_width, picture_height) = self.picture_display_size(picture, span);
                        let side = match picture.position {
                            PicturePosition::FloatLeft => FloatSide::Left,
                            PicturePosition::FloatRight => FloatSide::Right,
                            PicturePosition::Inline => unreachable!("handled above"),
                        };
                        let x = match side {
                            FloatSide::Left => line_left,
                            FloatSide::Right => line_right.saturating_sub(picture_width),
                        };
                        let bounds = Rect {
                            x,
                            y: line.y,
                            width: picture_width,
                            height: picture_height,
                        };
                        push_picture_boxes(boxes, picture, bounds);
                        floats.push(ActiveFloat { side, bounds });

                        let (first_right, indent) = if first_line && format.right_to_left {
                            (content_right.saturating_sub(first_indent), 0)
                        } else if first_line {
                            (content_right, first_indent)
                        } else {
                            (content_right, 0)
                        };
                        (line, line_left, line_right) = constrained_line(
                            content_left,
                            first_right,
                            line.y,
                            indent,
                            boxes.len(),
                            &floats,
                        );
                    }
                },
                Inline::EmbeddedWindow(window) => {
                    // WinHlp32 0x419281 creates the authored native child control and asks it
                    // for dimensions. We never execute the authored macro, but the built-in
                    // `!label,macro` form has deterministic BUTTON geometry in KB917607 and can
                    // therefore be represented much more faithfully than a generic placeholder.
                    resolve_pending_tab(&mut pending_tab, &mut line, boxes);
                    let available_width = line_right.saturating_sub(line_left).max(1);
                    let (desired_width, height, standard_button_label) =
                        embedded_window_geometry(window, available_width, self.dpi_x, self.dpi_y);
                    if !format.no_wrap
                        && line.x > line.start_x
                        && line.x.saturating_add(desired_width) > line_right
                    {
                        self.finish_line(format, line_left, line_right, &mut line, boxes, rtl_locale);
                        let natural = if line.height > 0 {
                            line.height
                        } else {
                            last_text_line_height.unwrap_or(16)
                        };
                        y = line.y.saturating_add(self.line_advance(format, natural));
                        first_line = false;
                        flow_line_index = flow_line_index.saturating_add(1);
                        flow_hard_break_before = false;
                        (line, line_left, line_right) = constrained_line(
                            content_left,
                            content_right,
                            y,
                            0,
                            boxes.len(),
                            &floats,
                        );
                    }
                    let width = desired_width.min(line_right.saturating_sub(line.x).max(1));
                    boxes.push(LayoutBox {
                        bounds: Rect {
                            x: line.x,
                            y: line.y,
                            width,
                            height,
                        },
                        kind: LayoutKind::EmbeddedWindowPlaceholder {
                            descriptor: window.descriptor.clone(),
                            standard_button_label,
                            hotspot: embedded_window_hotspot(window),
                        },
                    });
                    line.box_end = boxes.len();
                    line.x = line.x.saturating_add(width);
                    line.height = line.height.max(height);
                }
                // KB917607 tokenizer 0x417816 writes the signed WORD directly to render-state
                // +0x38 and returns status 2. That field is the transient horizontal line origin:
                // paragraph setup seeds it from the DPI-converted left/first-line geometry at
                // 0x415CE4..0x415D35, and line finalization reads it again at 0x415FB5 before
                // alignment. The 0x85 value is already in device-coordinate units, so it replaces
                // the current horizontal origin without creating a glyph or width of its own.
                Inline::Control85(marker) => {
                    resolve_pending_tab(&mut pending_tab, &mut line, boxes);
                    line.x = origin_x.saturating_add(i32::from(*marker));
                }
            }
        }

        resolve_pending_tab(&mut pending_tab, &mut line, boxes);
        self.finish_line(format, line_left, line_right, &mut line, boxes, rtl_locale);
        let natural = if line.height > 0 {
            line.height
        } else if border_only {
            // A border-only paragraph is a rule/container, not an authored blank text line.
            // The old unconditional 16 px fallback inserted a synthetic empty line between
            // top/bottom rules and produced the conspicuous double-rule gap seen in CALC.HLP.
            0
        } else {
            last_text_line_height.unwrap_or(16)
        };
        y = if natural > 0 {
            line.y.saturating_add(self.line_advance(format, natural))
        } else {
            line.y
        };
        let float_bottom = floats
            .iter()
            .map(|float| float.bounds.y.saturating_add(float.bounds.height))
            .max()
            .unwrap_or(paragraph_top);
        let paragraph_bottom = y.max(float_bottom).saturating_add(bottom_clearance);

        if let Some(border) = format.border {
            boxes.push(LayoutBox {
                bounds: Rect {
                    x: border_left,
                    y: paragraph_top,
                    width: border_right.saturating_sub(border_left).max(1),
                    height: paragraph_bottom.saturating_sub(paragraph_top).max(1),
                },
                kind: LayoutKind::Border {
                    flags: border.flags,
                    style: border.style,
                },
            });
        }

        paragraph_bottom
            .saturating_add(spacing_below)
            .saturating_sub(origin_y)
            .max(1)
    }

    fn line_advance(&self, format: &ParagraphFormat, natural: i32) -> i32 {
        let natural = natural.max(1);
        let authored = self.optional_vertical_paragraph_metric(format.spacing_lines);
        // Viewer zoom changes the native glyph cell before retained layout runs. WinHelp's signed
        // spacing-lines value is otherwise a fixed device-pixel advance, so leaving it unscaled
        // makes exact negative spacing increasingly smaller than the enlarged text and causes the
        // crowding/overlap visible at 150-200%. Keep 100% byte-for-byte compatible and scale only
        // this text-driven metric, not paragraph indents, pictures, or physical-resolution data.
        let zoom = i64::from(self.text_zoom_percent.clamp(1, 1_000));
        let zoomed_authored = i32::try_from(
            i64::from(authored)
                .saturating_mul(zoom)
                .saturating_add(if authored >= 0 { 50 } else { -50 })
                / 100,
        )
        .unwrap_or(if authored < 0 { i32::MIN } else { i32::MAX });
        if zoomed_authored < 0 {
            // KB917607 WinHlp32 0x4160c5..0x4160e4: negative spacing is exact at native scale.
            zoomed_authored.saturating_abs().max(1)
        } else if zoomed_authored > 0 {
            // Positive spacing is a minimum; the natural measured line height still wins.
            natural.max(zoomed_authored)
        } else {
            natural
        }
    }

    fn finish_line(
        &self,
        format: &ParagraphFormat,
        _line_left: i32,
        line_right: i32,
        line: &mut LineState,
        boxes: &mut [LayoutBox],
        rtl_locale: bool,
    ) {
        if line.box_start >= line.box_end || line.box_end > boxes.len() {
            return;
        }
        let remaining = line_right.saturating_sub(line.x).max(0);
        let shift = match format.alignment {
            ParagraphAlignment::Left => 0,
            ParagraphAlignment::Right => remaining,
            ParagraphAlignment::Center => remaining / 2,
        };
        // `line.start_x` already includes the first-line indent for LTR paragraphs. Using the
        // unindented `line_left` as the lower clamp erased negative first-line indents (hanging
        // indents) after layout. Continuation lines are created with start_x == line_left, so
        // this preserves the indent on the first visual line only without changing later lines.
        let lower_bound = line.start_x.min(line_right);
        for item in &mut boxes[line.box_start..line.box_end] {
            item.bounds.x = item.bounds.x.saturating_add(shift).max(lower_bound);
        }

        if rtl_locale {
            reorder_charset_text_runs(
                &mut boxes[line.box_start..line.box_end],
                format.right_to_left,
            );
        }

        // WinHlp32 aligns text and inline objects on one line baseline. Text contributes the
        // baseline retained by the measurement backend; an inline graphic or the verified stock
        // `!label,macro` BUTTON contributes its bottom edge. This distinction matters both for
        // CALC.HLP bitmap list markers and for its fixed 12x12 Related Topics button when text
        // zoom changes the adjacent font ascent. Float pictures and generic hosted controls are
        // outside this rule and therefore remain unaffected.
        align_inline_runs_to_baseline(&mut boxes[line.box_start..line.box_end]);
        if let Some(bottom) = boxes[line.box_start..line.box_end]
            .iter()
            .map(|item| item.bounds.y.saturating_add(item.bounds.height))
            .max()
        {
            line.height = line.height.max(bottom.saturating_sub(line.y));
        }
    }

    fn default_line_height(&self, style: &ResolvedTextStyle) -> i32 {
        div_round(
            i64::from(effective_point_size_twips(style)).abs().max(20)
                * i64::from(self.dpi_y.max(1)),
            1440,
        )
        .saturating_add(3)
        .max(12)
    }

    fn approximate_text_metrics(&self, style: &ResolvedTextStyle, text: &str) -> TextMetrics {
        let units: i32 = if style.family == ResolvedFontFamily::Monospace {
            i32::try_from(text.chars().count())
                .unwrap_or(i32::MAX)
                .saturating_mul(6)
        } else {
            text.chars()
                .map(|ch| {
                    if ch.is_ascii_whitespace() {
                        3
                    } else if ch.is_ascii_uppercase() {
                        7
                    } else if ch.is_ascii_punctuation() {
                        4
                    } else {
                        6
                    }
                })
                .sum()
        };
        let em = div_round(
            i64::from(effective_point_size_twips(style)).abs().max(20)
                * i64::from(self.dpi_x.max(1)),
            1440,
        )
        .max(1);
        let height = self.default_line_height(style);
        TextMetrics {
            width: i32::try_from(i64::from(units) * i64::from(em) / 10)
                .unwrap_or(i32::MAX)
                .max(0),
            height,
            baseline: height.saturating_mul(5) / 6,
        }
    }

    fn optional_horizontal_paragraph_metric(&self, value: Option<i16>) -> i32 {
        value.map_or(0, |raw| self.paragraph_metric_to_pixels(i32::from(raw)))
    }

    fn optional_vertical_paragraph_metric(&self, value: Option<i16>) -> i32 {
        value.map_or(0, |raw| {
            self.paragraph_vertical_metric_to_pixels(i32::from(raw))
        })
    }

    /// Converts a horizontal LinkData1 paragraph metric exactly as KB917607 WinHlp32 does.
    ///
    /// Indents, tab stops and table x geometry use device LOGPIXELSX and signed integer division
    /// by 144. Rust integer division, like x86 `idiv`, truncates toward zero.
    pub fn paragraph_metric_to_pixels(&self, raw: i32) -> i32 {
        paragraph_metric_for_dpi(raw, self.dpi_x)
    }

    /// Converts a vertical LinkData1 paragraph metric with device LOGPIXELSY.
    pub fn paragraph_vertical_metric_to_pixels(&self, raw: i32) -> i32 {
        paragraph_metric_for_dpi(raw, self.dpi_y)
    }

    /// Legacy helper retained for table experiments whose units have not yet been established
    /// from the Microsoft binary. It remains horizontal for backward compatibility.
    pub fn metric_to_pixels(&self, raw: i32, metric: FontMetric) -> i32 {
        match metric {
            FontMetric::HalfPoints => {
                // Old WinHelp compatibility uses scale 10 with a 5-twip rounding offset for
                // positive encoded metrics before RichEdit performs device conversion. Mirror
                // that offset for signed indents so their magnitude rounds consistently.
                let twips = i64::from(raw) * 10
                    - if raw > 0 { 5 } else if raw < 0 { -5 } else { 0 };
                div_round(twips * i64::from(self.dpi_x.max(1)), 1_440)
            }
            FontMetric::Twips => {
                div_round(i64::from(raw) * i64::from(self.dpi_x.max(1)), 1_440)
            }
        }
    }

    /// Computes the natural display size WinHlp32 derives from the picture record and target DC.
    ///
    /// Raster pixels stay untouched; only layout geometry changes. Bitmap records with both
    /// resolution fields use `pixels * LOGPIXELS / authored_resolution` (`0x4066AF..0x4066CB`).
    /// WMF physical mapping modes likewise use the current horizontal/vertical device DPI.
    fn picture_display_size(&self, picture: &PictureReference, content_width: i32) -> (i32, i32) {
        let Some(image) = &picture.image else {
            return (PICTURE_WIDTH.min(content_width.max(1)), PICTURE_HEIGHT);
        };

        let (natural_width, natural_height) = match image.sizing {
            PictureSizing::Pixels => (image.width, image.height),
            PictureSizing::BitmapResolution {
                x_resolution,
                y_resolution,
            } => (
                scale_bitmap_resolution(image.width, self.dpi_x, x_resolution),
                scale_bitmap_resolution(image.height, self.dpi_y, y_resolution),
            ),
            PictureSizing::Metafile {
                mapping_mode,
                logical_width,
                logical_height,
            } => metafile_pixel_dimensions(
                mapping_mode,
                logical_width,
                logical_height,
                u32::try_from(self.dpi_x.max(1)).unwrap_or(u32::MAX),
                u32::try_from(self.dpi_y.max(1)).unwrap_or(u32::MAX),
            ),
        };

        let natural_width = i32::try_from(natural_width).unwrap_or(i32::MAX).max(1);
        let natural_height = i32::try_from(natural_height).unwrap_or(i32::MAX).max(1);
        let max_width = content_width.max(1);
        if natural_width <= max_width {
            return (natural_width, natural_height);
        }
        let scaled_height = i64::from(natural_height)
            .saturating_mul(i64::from(max_width))
            .checked_div(i64::from(natural_width))
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(1)
            .max(1);
        (max_width, scaled_height)
    }
}

fn scale_bitmap_resolution(pixels: u32, dpi: i32, resolution: u32) -> u32 {
    if resolution == 0 {
        return pixels.max(1);
    }
    let scaled = u64::from(pixels)
        .saturating_mul(u64::try_from(dpi.max(1)).unwrap_or(u64::MAX))
        / u64::from(resolution);
    u32::try_from(scaled).unwrap_or(u32::MAX).max(1)
}


fn paragraph_metric_for_dpi(raw: i32, dpi: i32) -> i32 {
    let scaled = i64::from(raw) * i64::from(dpi.max(1));
    i32::try_from(scaled / 144)
        .unwrap_or_else(|_| if scaled.is_negative() { i32::MIN } else { i32::MAX })
}

/// Aligns text and inline objects to the baseline used by one retained visual line.
///
/// Text contributes the native/fallback baseline stored in `LayoutKind::Text`. WinHlp32's compact
/// inline-object path treats the bottom of an inline graphic as that object's baseline; for a
/// single image this is exactly its retained height. The verified built-in `!label,macro` BUTTON
/// form follows the same inline-row geometry: its bottom edge shares the text baseline. This is
/// particularly important under viewer zoom because the 12x12 stock ALink button remains a fixed
/// control size while the adjacent font baseline moves. Generic authored hosted controls are not
/// included here because their native child-window geometry is runtime-negotiated.
/// `push_picture_boxes()` emits transparent picture-hotspot overlays immediately after the visible
/// picture, so those overlays inherit the same vertical shift and remain registered with the pixels
/// they cover.
fn align_inline_runs_to_baseline(boxes: &mut [LayoutBox]) {
    let max_baseline = boxes
        .iter()
        .filter_map(|item| match &item.kind {
            LayoutKind::Text { baseline, .. } => Some((*baseline).max(0)),
            LayoutKind::Picture { .. } | LayoutKind::PicturePlaceholder => {
                Some(item.bounds.height.max(0))
            }
            LayoutKind::EmbeddedWindowPlaceholder {
                standard_button_label: Some(_),
                ..
            } => Some(item.bounds.height.max(0)),
            _ => None,
        })
        .max();
    let Some(max_baseline) = max_baseline else {
        return;
    };

    let mut picture_shift = 0;
    for item in boxes {
        match &item.kind {
            LayoutKind::Text { baseline, .. } => {
                picture_shift = 0;
                item.bounds.y = item.bounds.y.saturating_add(
                    max_baseline.saturating_sub((*baseline).max(0)),
                );
            }
            LayoutKind::Picture { .. } | LayoutKind::PicturePlaceholder => {
                picture_shift = max_baseline.saturating_sub(item.bounds.height.max(0));
                item.bounds.y = item.bounds.y.saturating_add(picture_shift);
            }
            LayoutKind::PictureHotspot { .. } => {
                // Hotspot overlays are emitted directly after their owning picture.
                item.bounds.y = item.bounds.y.saturating_add(picture_shift);
            }
            LayoutKind::EmbeddedWindowPlaceholder {
                standard_button_label: Some(_),
                ..
            } => {
                picture_shift = 0;
                item.bounds.y = item.bounds.y.saturating_add(
                    max_baseline.saturating_sub(item.bounds.height.max(0)),
                );
            }
            LayoutKind::EmbeddedWindowPlaceholder { .. } | LayoutKind::Border { .. } => {
                picture_shift = 0;
            }
        }
    }
}

/// Repositions contiguous text boxes using the same face-charset distinction that KB917607
/// applies in its Arabic/Hebrew line pass (`0x415F30`, `0x41623B..0x416428`). Text inside each
/// box stays in logical order so the native painter remains responsible for glyph shaping.
fn reorder_charset_text_runs(boxes: &mut [LayoutBox], base_rtl: bool) {
    let mut start = 0;
    while start < boxes.len() {
        let Some(_) = text_box_is_rtl(&boxes[start]) else {
            start += 1;
            continue;
        };

        let mut end = start + 1;
        while end < boxes.len() {
            if text_box_is_rtl(&boxes[end]).is_none()
                || boxes[end].bounds.y != boxes[end - 1].bounds.y
                || boxes[end - 1]
                    .bounds
                    .x
                    .saturating_add(boxes[end - 1].bounds.width)
                    != boxes[end].bounds.x
            {
                break;
            }
            end += 1;
        }
        reorder_contiguous_text_segment(&mut boxes[start..end], base_rtl);
        start = end;
    }
}

fn text_box_is_rtl(item: &LayoutBox) -> Option<bool> {
    match &item.kind {
        LayoutKind::Text { style, .. } => Some(matches!(style.charset, Some(0xB1 | 0xB2))),
        _ => None,
    }
}

fn reorder_contiguous_text_segment(boxes: &mut [LayoutBox], base_rtl: bool) {
    if boxes.len() < 2 || !boxes.iter().any(|item| text_box_is_rtl(item) == Some(true)) {
        return;
    }

    let left = boxes.iter().map(|item| item.bounds.x).min().unwrap_or(0);
    let mut runs = Vec::<(usize, usize, bool)>::new();
    let mut run_start = 0;
    let mut run_rtl = text_box_is_rtl(&boxes[0]).unwrap_or(false);
    for index in 1..boxes.len() {
        let rtl = text_box_is_rtl(&boxes[index]).unwrap_or(false);
        if rtl != run_rtl {
            runs.push((run_start, index, run_rtl));
            run_start = index;
            run_rtl = rtl;
        }
    }
    runs.push((run_start, boxes.len(), run_rtl));

    let mut visual_order = Vec::with_capacity(boxes.len());
    let append_run = |order: &mut Vec<usize>, start: usize, end: usize, rtl: bool| {
        if rtl {
            order.extend((start..end).rev());
        } else {
            order.extend(start..end);
        }
    };
    if base_rtl {
        for &(start, end, rtl) in runs.iter().rev() {
            append_run(&mut visual_order, start, end, rtl);
        }
    } else {
        for &(start, end, rtl) in &runs {
            append_run(&mut visual_order, start, end, rtl);
        }
    }

    let mut x = left;
    for index in visual_order {
        boxes[index].bounds.x = x;
        x = x.saturating_add(boxes[index].bounds.width);
    }
}

fn reference_border_clearance(style: BorderStyle) -> i32 {
    match style {
        BorderStyle::Normal | BorderStyle::ReferenceStyle4 => 5,
        BorderStyle::Thick | BorderStyle::Shadow => 6,
        BorderStyle::Double => 7,
        BorderStyle::Reserved(_) => 0,
    }
}

fn effective_point_size_twips(style: &ResolvedTextStyle) -> i32 {
    if style.small_caps {
        // KB917607 WinHlp32 0x411a59..0x411a6c: HC30 small-caps reduces lfHeight to 2/3.
        style.point_size_twips.saturating_mul(2) / 3
    } else {
        style.point_size_twips
    }
}

fn normalized_metrics(metrics: TextMetrics, fallback_height: i32) -> TextMetrics {
    let height = if metrics.height > 0 {
        metrics.height
    } else {
        fallback_height.max(1)
    };
    let baseline = if metrics.baseline > 0 {
        metrics.baseline.min(height)
    } else {
        height.saturating_mul(5) / 6
    };
    TextMetrics {
        width: metrics.width.max(0),
        height,
        baseline,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TableColumnGeometry {
    x: i32,
    width: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FloatSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy)]
struct ActiveFloat {
    side: FloatSide,
    bounds: Rect,
}

/// Reports whether a retained stock hosted button invokes the associative-link macro.
fn standard_button_is_alink(window: &EmbeddedWindowReference) -> bool {
    let Some((_label, macro_text)) = window.standard_button_parts() else {
        return false;
    };
    let name = macro_text
        .split_once('(')
        .map_or(macro_text, |(name, _)| name)
        .trim();
    name.eq_ignore_ascii_case("AL") || name.eq_ignore_ascii_case("ALink")
}

/// Detects the classic Related Topics paragraph shape used by CALC.HLP and similar files.
fn paragraph_has_standard_alink_button(paragraph: &Paragraph) -> bool {
    paragraph.inlines.iter().any(|inline| {
        matches!(inline, Inline::EmbeddedWindow(window) if standard_button_is_alink(window))
    })
}

/// Detects whether a display record contains a stock ALink control row.
fn record_has_standard_alink_button(record: &FormattedRecord) -> bool {
    record.paragraphs.iter().any(paragraph_has_standard_alink_button)
}

/// Aligns the classic stock ALink row to the actual x-coordinate of the preceding rendered rule.
///
/// Paragraph layout must run first so authored left/first-line indents still participate in
/// wrapping and line construction. Only the blank 12x12 stock button and text boxes sharing its
/// visual line are translated; later wrapped lines and unrelated objects keep their authored x.
fn align_standard_alink_row_to_rule(boxes: &mut [LayoutBox], rule_left: i32) {
    let Some(button_index) = boxes.iter().position(|item| {
        matches!(
            &item.kind,
            LayoutKind::EmbeddedWindowPlaceholder {
                standard_button_label: Some(label),
                ..
            } if label.is_empty()
                && item.bounds.width == STANDARD_BUTTON_EMPTY_SIZE
                && item.bounds.height == STANDARD_BUTTON_EMPTY_SIZE
        )
    }) else {
        return;
    };

    let button_bounds = boxes[button_index].bounds;
    let delta_x = rule_left.saturating_sub(button_bounds.x);
    if delta_x == 0 {
        return;
    }
    let line_top = button_bounds.y;
    let line_bottom = button_bounds
        .y
        .saturating_add(button_bounds.height.max(1));

    for item in boxes {
        let item_bottom = item.bounds.y.saturating_add(item.bounds.height.max(1));
        let shares_visual_line = item.bounds.y < line_bottom && item_bottom > line_top;
        if shares_visual_line
            && matches!(
                &item.kind,
                LayoutKind::Text { .. } | LayoutKind::EmbeddedWindowPlaceholder { .. }
            )
        {
            item.bounds.x = item.bounds.x.saturating_add(delta_x);
        }
    }
}

/// Converts a stock `!label,macro` button into a viewer-local macro hotspot.
fn embedded_window_hotspot(window: &EmbeddedWindowReference) -> Option<Hotspot> {
    let (_label, macro_text) = window.standard_button_parts()?;
    let macro_text = macro_text.trim();
    (!macro_text.is_empty()).then(|| Hotspot {
        target: crate::HotspotTarget::Macro(macro_text.to_owned()),
        emphasized: false,
    })
}

/// Returns deterministic safe geometry for a retained hosted-window object.
///
/// KB917607 factory `0x4240F4` creates its leading-`!` built-in form as a standard `BUTTON`.
/// It initially uses 30x12/16 creation geometry, but the empty-label branch then resizes the child
/// to the final 12x12 dimensions returned to layout. Non-empty labels are measured by WinHlp32
/// after creation; this safe cross-platform substitute keeps the verified initial 30x16 minimum.
///
/// For arbitrary authored controls, `0x4242A8..0x424359` obtains `LOGPIXELSX/Y` and creates the
/// child at exactly two device inches (`2 * DPI_X` by `2 * DPI_Y`). The original then lets the
/// control negotiate a final size through private message `0x706B`, falling back to its actual
/// `GetWindowRect`. Executing document-supplied controls remains outside this viewer's security
/// policy, but the safe placeholder now uses Microsoft's exact pre-negotiation rectangle rather
/// than an invented 180x36 size.
fn embedded_window_geometry(
    window: &EmbeddedWindowReference,
    available_width: i32,
    dpi_x: i32,
    dpi_y: i32,
) -> (i32, i32, Option<String>) {
    let available_width = available_width.max(1);
    if let Some((label, _macro_text)) = window.standard_button_parts() {
        let (width, height) = if label.is_empty() {
            (STANDARD_BUTTON_EMPTY_SIZE, STANDARD_BUTTON_EMPTY_SIZE)
        } else {
            (STANDARD_BUTTON_INITIAL_WIDTH, STANDARD_BUTTON_INITIAL_LABEL_HEIGHT)
        };
        (width.min(available_width), height, Some(label.to_owned()))
    } else {
        let width = dpi_x.max(1).saturating_mul(2).min(available_width);
        let height = dpi_y.max(1).saturating_mul(2);
        (width, height, None)
    }
}

/// Finds the horizontal text span at `y`, skipping below floats when neither side leaves enough
/// room for a useful line. WinHelp `bml`/`bmr` pictures therefore behave as true paragraph floats
/// rather than large inline glyphs.
fn constrained_line(
    content_left: i32,
    content_right: i32,
    mut y: i32,
    first_indent: i32,
    box_start: usize,
    floats: &[ActiveFloat],
) -> (LineState, i32, i32) {
    loop {
        let mut line_left = content_left;
        let mut line_right = content_right;
        let mut next_bottom: Option<i32> = None;

        for float in floats {
            let top = float.bounds.y;
            let bottom = top.saturating_add(float.bounds.height);
            if y < top || y >= bottom {
                continue;
            }
            next_bottom = Some(next_bottom.map_or(bottom, |current| current.min(bottom)));
            match float.side {
                FloatSide::Left => {
                    line_left = line_left.max(float.bounds.x.saturating_add(float.bounds.width));
                }
                FloatSide::Right => {
                    line_right = line_right.min(float.bounds.x);
                }
            }
        }

        if line_right.saturating_sub(line_left) >= 16 {
            let start_x = line_left.saturating_add(first_indent).min(line_right);
            return (LineState::new(start_x, y, box_start), line_left, line_right);
        }

        let Some(bottom) = next_bottom else {
            let start_x = line_left.saturating_add(first_indent).min(line_right);
            return (LineState::new(start_x, y, box_start), line_left, line_right);
        };
        if bottom <= y {
            let start_x = line_left.saturating_add(first_indent).min(line_right);
            return (LineState::new(start_x, y, box_start), line_left, line_right);
        }
        y = bottom;
    }
}

/// Adds the visible image and then transparent hotspot overlays in authored image coordinates.
fn push_picture_boxes(boxes: &mut Vec<LayoutBox>, picture: &PictureReference, bounds: Rect) {
    boxes.push(LayoutBox {
        bounds,
        kind: picture.image.as_ref().map_or(
            LayoutKind::PicturePlaceholder,
            |image| LayoutKind::Picture {
                image: image.clone(),
            },
        ),
    });

    let Some(image) = picture.image.as_ref() else {
        return;
    };
    if image.width == 0 || image.height == 0 || bounds.width <= 0 || bounds.height <= 0 {
        return;
    }

    for hotspot in &picture.hotspots {
        let x0 = scale_picture_coordinate(hotspot.x, image.width, bounds.width);
        let y0 = scale_picture_coordinate(hotspot.y, image.height, bounds.height);
        let x1 = scale_picture_coordinate(
            hotspot.x.saturating_add(hotspot.width),
            image.width,
            bounds.width,
        );
        let y1 = scale_picture_coordinate(
            hotspot.y.saturating_add(hotspot.height),
            image.height,
            bounds.height,
        );
        let local_x0 = x0.clamp(0, bounds.width);
        let local_y0 = y0.clamp(0, bounds.height);
        let local_x1 = x1.clamp(local_x0, bounds.width);
        let local_y1 = y1.clamp(local_y0, bounds.height);
        let width = local_x1.saturating_sub(local_x0);
        let height = local_y1.saturating_sub(local_y0);
        if width == 0 || height == 0 {
            continue;
        }
        boxes.push(LayoutBox {
            bounds: Rect {
                x: bounds.x.saturating_add(local_x0),
                y: bounds.y.saturating_add(local_y0),
                width,
                height,
            },
            kind: LayoutKind::PictureHotspot {
                hotspot: hotspot.hotspot.clone(),
            },
        });
    }
}

fn scale_picture_coordinate(value: u32, source_extent: u32, display_extent: i32) -> i32 {
    if source_extent == 0 || display_extent <= 0 {
        return 0;
    }
    let scaled = u64::from(value)
        .saturating_mul(u64::try_from(display_extent).unwrap_or(0))
        / u64::from(source_extent);
    i32::try_from(scaled).unwrap_or(i32::MAX)
}

#[derive(Debug, Clone, Copy)]
struct LineState {
    start_x: i32,
    x: i32,
    y: i32,
    height: i32,
    box_start: usize,
    box_end: usize,
}

impl LineState {
    fn new(x: i32, y: i32, box_start: usize) -> Self {
        Self {
            start_x: x,
            x,
            y,
            height: 0,
            box_start,
            box_end: box_start,
        }
    }
}

fn resolve_style(run: &TextRun, fonts: &FontTable) -> ResolvedTextStyle {
    let descriptor = fonts
        .descriptor(run.font_index)
        .or_else(|| fonts.descriptors().first());
    descriptor.map_or_else(
        || ResolvedTextStyle {
            face_name: "MS Sans Serif".to_owned(),
            family: ResolvedFontFamily::Proportional,
            source_family: HlpFontFamily::Swiss,
            point_size: 10,
            point_size_twips: 200,
            weight: 400,
            italic: false,
            underline: run.hotspot.as_ref().is_some_and(|value| value.emphasized),
            strike_out: false,
            small_caps: false,
            foreground: Rgb {
                red: 0,
                green: 0,
                blue: 0,
            },
            foreground_inherits: true,
            background: Rgb {
                red: 255,
                green: 255,
                blue: 255,
            },
            background_inherits: true,
            charset: None,
        },
        |font| style_from_font(font, run.hotspot.as_ref()),
    )
}

fn style_from_font(font: &FontDescriptor, hotspot: Option<&Hotspot>) -> ResolvedTextStyle {
    let emphasized = hotspot.is_some_and(|value| value.emphasized);
    ResolvedTextStyle {
        face_name: font.face_name.clone(),
        family: if font.is_fixed_pitch() {
            ResolvedFontFamily::Monospace
        } else {
            ResolvedFontFamily::Proportional
        },
        source_family: font.family,
        point_size: font.point_size(),
        point_size_twips: font.point_size_twips,
        weight: font.weight,
        italic: font.italic,
        underline: font.underline || emphasized,
        strike_out: font.strike_out,
        small_caps: font.small_caps,
        // The verified Microsoft KB917607 WinHlp32 compares the descriptor COLORREF
        // against exactly 0x00000101 (RGB 1,1,0). That value means "keep the current
        // text colour"; nearby dark colours are ordinary authored colours.
        foreground: if emphasized {
            Rgb { red: 0, green: 128, blue: 0 }
        } else {
            font.foreground
        },
        foreground_inherits: !emphasized && is_inherit_colour(font.foreground),
        background: font.background,
        background_inherits: is_inherit_colour(font.background),
        charset: font.charset,
    }
}

/// True for a compact display paragraph that WinHlp32 consumes without vertical advance.
///
/// A border makes an otherwise-empty paragraph visible, while explicit tabs/line breaks, pictures,
/// and hosted controls are layout-bearing commands. The 0x85 transient marker remains glyphless,
/// but it can reset the current horizontal line origin.
fn table_cell_paragraph_is_empty_filler(paragraph: &Paragraph) -> bool {
    paragraph.format.border.is_none()
        && paragraph
            .inlines
            .iter()
            .all(|inline| matches!(inline, Inline::Control85(_)))
}

/// Detects a top-level display record that carries only authored paragraph rules.
fn record_is_rule_only(record: &FormattedRecord) -> bool {
    if record.table.is_some() || record.paragraphs.is_empty() {
        return false;
    }

    let mut has_border = false;
    for paragraph in &record.paragraphs {
        has_border |= paragraph.format.border.is_some();
        if paragraph
            .inlines
            .iter()
            .any(|inline| !matches!(inline, Inline::Control85(_)))
        {
            return false;
        }
    }
    has_border
}

fn is_inherit_colour(colour: Rgb) -> bool {
    colour == Rgb { red: 1, green: 1, blue: 0 }
}

fn tokenize(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices();
    let Some((_, first)) = chars.next() else {
        return result;
    };
    let mut last_space = first.is_whitespace();
    for (index, ch) in chars {
        let space = ch.is_whitespace();
        if space != last_space {
            result.push(&text[start..index]);
            start = index;
            last_space = space;
        }
    }
    result.push(&text[start..]);
    result
}

fn tokenize_for_charset(text: &str, charset: Option<u8>) -> Vec<&str> {
    if !matches!(charset, Some(0x80 | 0x81 | 0x82 | 0x86 | 0x88)) {
        return tokenize(text);
    }
    tokenize_cjk(text)
}

/// Produces legacy CJK line-break units after Shift-JIS/CP949/GBK/Big5 decoding.
///
/// Legacy CJK text cannot rely on ASCII spaces as its only wrapping opportunity. Keep Latin words
/// and whitespace runs intact, expose conservative break opportunities around ordinary CJK
/// characters, and bind common opening/closing punctuation so it is not stranded at a line edge.
fn tokenize_cjk(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }

    let chars = text.char_indices().collect::<Vec<_>>();
    let mut ranges = Vec::<(usize, usize)>::new();
    let mut index = 0;
    while index < chars.len() {
        let (start, ch) = chars[index];
        let end_of = |position: usize| {
            chars
                .get(position + 1)
                .map_or(text.len(), |(next, _)| *next)
        };

        if ch.is_whitespace() {
            let mut end_index = index;
            while end_index + 1 < chars.len() && chars[end_index + 1].1.is_whitespace() {
                end_index += 1;
            }
            ranges.push((start, end_of(end_index)));
            index = end_index + 1;
            continue;
        }

        if is_cjk_character(ch) {
            ranges.push((start, end_of(index)));
            index += 1;
            continue;
        }

        let mut end_index = index;
        while end_index + 1 < chars.len() {
            let next = chars[end_index + 1].1;
            if next.is_whitespace() || is_cjk_character(next) {
                break;
            }
            end_index += 1;
        }
        ranges.push((start, end_of(end_index)));
        index = end_index + 1;
    }

    let mut merged = Vec::<(usize, usize)>::with_capacity(ranges.len());
    for range in ranges {
        let current = &text[range.0..range.1];
        let current_first = current.chars().next();
        let previous_last = merged
            .last()
            .and_then(|(start, end)| text[*start..*end].chars().next_back());
        let merge_with_previous = !current.chars().all(char::is_whitespace)
            && merged.last().is_some_and(|(start, end)| {
                !text[*start..*end].chars().all(char::is_whitespace)
            })
            && (current_first.is_some_and(is_cjk_closing_punctuation)
                || previous_last.is_some_and(is_cjk_opening_punctuation));
        if merge_with_previous {
            if let Some(previous) = merged.last_mut() {
                previous.1 = range.1;
            }
        } else {
            merged.push(range);
        }
    }

    merged
        .into_iter()
        .map(|(start, end)| &text[start..end])
        .collect()
}

fn is_cjk_character(ch: char) -> bool {
    matches!(
        ch as u32,
        0x2E80..=0x2FFF
            | 0x3000..=0x303F
            | 0x3040..=0x30FF
            | 0x3100..=0x312F
            | 0x3130..=0x318F
            | 0x31A0..=0x31BF
            | 0x31F0..=0x31FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
            | 0xFF00..=0xFFEF
            | 0x20000..=0x2FA1F
    )
}

fn is_cjk_opening_punctuation(ch: char) -> bool {
    matches!(ch, '(' | '[' | '{' | '（' | '［' | '｛' | '〈' | '《' | '「' | '『' | '【' | '〔' | '〖' | '〘' | '〚' | '‘' | '“')
}

fn is_cjk_closing_punctuation(ch: char) -> bool {
    matches!(
        ch,
        ')' | ']' | '}' | '）' | '］' | '｝' | '〉' | '》' | '」' | '』' | '】' | '〕'
            | '〗' | '〙' | '〛' | '、' | '。' | '，' | '．' | '！' | '？' | '：' | '；'
            | '’' | '”' | '・' | 'ー' | '々' | 'ぁ' | 'ぃ' | 'ぅ' | 'ぇ' | 'ぉ' | 'っ'
            | 'ゃ' | 'ゅ' | 'ょ' | 'ァ' | 'ィ' | 'ゥ' | 'ェ' | 'ォ' | 'ッ' | 'ャ' | 'ュ'
            | 'ョ' | 'ヮ' | 'ヵ' | 'ヶ'
    )
}

#[derive(Debug, Clone, Copy)]
struct TabTarget {
    position: i32,
    alignment: TabAlignment,
}

#[derive(Debug, Clone, Copy)]
struct PendingTab {
    alignment: TabAlignment,
    target_x: i32,
    box_start: usize,
}

fn next_tab_target(
    format: &ParagraphFormat,
    current: i32,
    engine: &LayoutEngine,
) -> TabTarget {
    for stop in &format.tabs {
        let position = engine.paragraph_metric_to_pixels(i32::from(stop.position));
        if position > current {
            return TabTarget {
                position,
                alignment: stop.alignment,
            };
        }
    }

    let raw_default = i32::from(format.default_tab_interval.unwrap_or(72));
    let default = engine.paragraph_metric_to_pixels(raw_default).abs().max(1);
    TabTarget {
        position: ((current / default) + 1).saturating_mul(default),
        alignment: TabAlignment::Left,
    }
}

fn resolve_pending_tab(
    pending: &mut Option<PendingTab>,
    line: &mut LineState,
    boxes: &mut [LayoutBox],
) {
    let Some(tab) = pending.take() else {
        return;
    };
    if tab.box_start >= line.box_end || line.box_end > boxes.len() {
        return;
    }
    let segment = &boxes[tab.box_start..line.box_end];
    let left = segment.iter().map(|item| item.bounds.x).min().unwrap_or(line.x);
    let right = segment
        .iter()
        .map(|item| item.bounds.x.saturating_add(item.bounds.width))
        .max()
        .unwrap_or(line.x);
    let anchor = match tab.alignment {
        TabAlignment::Right => right,
        TabAlignment::Center => left.saturating_add(right.saturating_sub(left) / 2),
        TabAlignment::Left | TabAlignment::Unknown(_) => return,
    };
    let shift = tab.target_x.saturating_sub(anchor);
    // WinHlp32's 0x412525 declines a deferred-tab shift when it would be negative.
    if shift < 0 {
        return;
    }
    for item in &mut boxes[tab.box_start..line.box_end] {
        item.bounds.x = item.bounds.x.saturating_add(shift);
    }
    line.x = line.x.saturating_add(shift);
}

fn div_round(numerator: i64, denominator: i64) -> i32 {
    if numerator >= 0 {
        i32::try_from((numerator + denominator / 2) / denominator).unwrap_or(i32::MAX)
    } else {
        i32::try_from((numerator - denominator / 2) / denominator).unwrap_or(i32::MIN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BorderInfo, FormattedRecord, HotspotTarget, ParagraphFormat, TableColumn, TopicId,
        TopicOffset, TopicPos,
    };

    fn topic_with_text(text: &str, hotspot: Option<Hotspot>) -> TopicPresentation {
        TopicPresentation {
            id: TopicId(TopicPos(12)),
            title: "Test".to_owned(),
            non_scrolling: Vec::new(),
            scrolling: vec![FormattedRecord {
                topic_size: 0,
                topic_length: None,
                table: None,
                table_cells: Vec::new(),
                paragraphs: vec![Paragraph {
                    format: ParagraphFormat::default(),
                    inlines: vec![Inline::Text(TextRun {
                        text: text.to_owned(),
                        font_index: 0,
                        hotspot,
                    })],
                }],
                issues: Vec::new(),
            }],
            warnings: Vec::new(),
        }
    }

    #[test]
    fn rectangle_hit_test_is_half_open() {
        let rect = Rect { x: 10, y: 20, width: 30, height: 40 };
        assert!(rect.contains(Point { x: 10, y: 20 }));
        assert!(rect.contains(Point { x: 39, y: 59 }));
        assert!(!rect.contains(Point { x: 40, y: 59 }));
        assert!(!rect.contains(Point { x: 39, y: 60 }));
    }

    #[test]
    fn rectangle_hit_test_does_not_overflow_at_i32_limits() {
        let rect = Rect { x: i32::MAX - 4, y: 0, width: 100, height: 10 };
        assert!(rect.contains(Point { x: i32::MAX - 1, y: 5 }));
    }

    #[test]
    fn tokenizer_classifies_unicode_whitespace_by_character() {
        assert_eq!(tokenize("é α"), vec!["é", " ", "α"]);
    }

    #[test]
    fn dbcs_tokenizer_exposes_cjk_break_opportunities_without_splitting_latin_words() {
        assert_eq!(
            tokenize_for_charset("日本語 Help 中文", Some(0x80)),
            vec!["日", "本", "語", " ", "Help", " ", "中", "文"]
        );
    }

    #[test]
    fn dbcs_tokenizer_keeps_cjk_punctuation_with_its_neighbor() {
        assert_eq!(
            tokenize_for_charset("「日本」、語", Some(0x86)),
            vec!["「日", "本」、", "語"]
        );
    }

    #[test]
    fn cjk_text_wraps_without_ascii_spaces() {
        let topic = topic_with_text("日本語", None);
        let mut fonts = FontTable::fallback();
        fonts.apply_system_metadata(&[0x80], Some(0x0411));
        let mut measure = |_style: &ResolvedTextStyle, text: &str| TextMetrics {
            width: i32::try_from(text.chars().count()).unwrap_or(i32::MAX) * 20,
            height: 20,
            baseline: 16,
        };
        let layout = LayoutEngine::default().layout_topic_with_measurer(
            &topic,
            &fonts,
            64,
            &mut measure,
        );
        let boxes = layout
            .scrolling
            .boxes
            .iter()
            .filter(|item| matches!(&item.kind, LayoutKind::Text { .. }))
            .collect::<Vec<_>>();
        assert_eq!(boxes.len(), 3);
        assert_eq!(boxes[0].bounds.y, boxes[1].bounds.y);
        assert!(boxes[2].bounds.y > boxes[1].bounds.y);
    }

    #[test]
    fn caller_supplied_metrics_control_spacing_and_positions() {
        let topic = topic_with_text("A B", None);
        let mut measure = |_style: &ResolvedTextStyle, text: &str| TextMetrics {
            width: match text {
                "A" => 11,
                " " => 7,
                "B" => 13,
                _ => 0,
            },
            height: 20,
            baseline: 16,
        };
        let layout = LayoutEngine::default().layout_topic_with_measurer(
            &topic,
            &FontTable::fallback(),
            320,
            &mut measure,
        );
        let text_boxes: Vec<_> = layout
            .scrolling
            .boxes
            .iter()
            .filter(|item| matches!(&item.kind, LayoutKind::Text { .. }))
            .collect();
        assert_eq!(text_boxes.len(), 3);
        assert_eq!(text_boxes[0].bounds.width, 11);
        assert_eq!(text_boxes[1].bounds.x, text_boxes[0].bounds.x + 11);
        assert_eq!(text_boxes[1].bounds.width, 7);
        assert_eq!(text_boxes[2].bounds.x, text_boxes[1].bounds.x + 7);
        assert_eq!(text_boxes[2].bounds.width, 13);
    }

    #[test]
    fn text_flow_metadata_distinguishes_automatic_wraps_from_hard_breaks() {
        let mut topic = topic_with_text("", None);
        topic.scrolling[0].paragraphs[0].inlines = vec![
            Inline::Text(TextRun {
                text: "Alpha Beta".to_owned(),
                font_index: 0,
                hotspot: None,
            }),
            Inline::LineBreak,
            Inline::Text(TextRun {
                text: "Gamma".to_owned(),
                font_index: 0,
                hotspot: None,
            }),
        ];
        let mut measure = |_style: &ResolvedTextStyle, text: &str| TextMetrics {
            width: match text {
                "Alpha" | "Beta" | "Gamma" => 34,
                " " => 8,
                _ => 0,
            },
            height: 20,
            baseline: 16,
        };
        let layout = LayoutEngine::default().layout_topic_with_measurer(
            &topic,
            &FontTable::fallback(),
            80,
            &mut measure,
        );
        let text = layout
            .scrolling
            .boxes
            .iter()
            .filter_map(|item| match &item.kind {
                LayoutKind::Text { text, flow, .. } if !text.trim().is_empty() => {
                    Some((text.as_str(), *flow))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(text.len(), 3);
        assert_eq!(text[0].0, "Alpha");
        assert_eq!(text[0].1.line_index, 0);
        assert!(!text[0].1.hard_break_before);
        assert_eq!(text[1].0, "Beta");
        assert_eq!(text[1].1.line_index, 1);
        assert!(!text[1].1.hard_break_before, "layout wrap is not an authored break");
        assert_eq!(text[2].0, "Gamma");
        assert_eq!(text[2].1.line_index, 2);
        assert!(text[2].1.hard_break_before, "explicit Inline::LineBreak must survive export");
        assert!(text.iter().all(|(_, flow)| flow.reflow_safe));
    }

    #[test]
    fn equal_height_text_runs_with_different_baselines_align_correctly() {
        let mut topic = topic_with_text("", None);
        topic.scrolling[0].paragraphs[0].inlines = vec![
            Inline::Text(TextRun {
                text: "■".to_owned(),
                font_index: 0,
                hotspot: None,
            }),
            Inline::Text(TextRun {
                text: "Body".to_owned(),
                font_index: 0,
                hotspot: None,
            }),
        ];
        let mut measure = |_style: &ResolvedTextStyle, text: &str| TextMetrics {
            width: if text == "■" { 6 } else { 32 },
            height: 20,
            baseline: if text == "■" { 10 } else { 16 },
        };

        let layout = LayoutEngine::default().layout_topic_with_measurer(
            &topic,
            &FontTable::fallback(),
            320,
            &mut measure,
        );
        let text_boxes: Vec<_> = layout
            .scrolling
            .boxes
            .iter()
            .filter(|item| matches!(&item.kind, LayoutKind::Text { .. }))
            .collect();

        assert_eq!(text_boxes.len(), 2);
        assert_eq!(text_boxes[0].bounds.y, text_boxes[1].bounds.y + 6);
        let bullet_baseline = match &text_boxes[0].kind {
            LayoutKind::Text { baseline, .. } => *baseline,
            _ => unreachable!(),
        };
        let body_baseline = match &text_boxes[1].kind {
            LayoutKind::Text { baseline, .. } => *baseline,
            _ => unreachable!(),
        };
        assert_eq!(
            text_boxes[0].bounds.y + bullet_baseline,
            text_boxes[1].bounds.y + body_baseline,
            "different-font runs should share the measurement backend's real baseline"
        );
    }

    #[test]
    fn inline_picture_marker_bottom_aligns_to_the_text_baseline() {
        let hotspot = Hotspot {
            target: HotspotTarget::Internal {
                offset: TopicOffset(7),
                popup: false,
            },
            emphasized: false,
        };
        let picture = PictureReference {
            command: 0x86,
            position: PicturePosition::Inline,
            picture_type: 0x22,
            encoded_size: 4,
            hotspot_count: Some(1),
            source: crate::PictureSource::Indexed(0),
            image: Some(DecodedPicture {
                width: 3,
                height: 7,
                rgba: std::sync::Arc::from(vec![255_u8; 3 * 7 * 4]),
                has_alpha: false,
                sizing: PictureSizing::Pixels,
                hotspots: std::sync::Arc::from([]),
            }),
            hotspots: vec![crate::PictureHotspot {
                x: 0,
                y: 0,
                width: 3,
                height: 7,
                hotspot,
            }],
            decode_warning: None,
        };
        let topic = TopicPresentation {
            id: TopicId(TopicPos(12)),
            title: "Inline marker baseline".to_owned(),
            non_scrolling: Vec::new(),
            scrolling: vec![FormattedRecord {
                topic_size: 0,
                topic_length: None,
                table: None,
                table_cells: Vec::new(),
                paragraphs: vec![Paragraph {
                    format: ParagraphFormat::default(),
                    inlines: vec![
                        Inline::Picture(picture),
                        Inline::Text(TextRun {
                            text: "Body".to_owned(),
                            font_index: 0,
                            hotspot: None,
                        }),
                    ],
                }],
                issues: Vec::new(),
            }],
            warnings: Vec::new(),
        };
        let mut measure = |_style: &ResolvedTextStyle, _text: &str| TextMetrics {
            width: 32,
            height: 20,
            baseline: 15,
        };

        let layout = LayoutEngine::default().layout_topic_with_measurer(
            &topic,
            &FontTable::fallback(),
            320,
            &mut measure,
        );
        let picture_box = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::Picture { .. }))
            .expect("inline marker picture");
        let hotspot_box = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::PictureHotspot { .. }))
            .expect("marker hotspot overlay");
        let text_box = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::Text { .. }))
            .expect("body text");
        let text_baseline = match &text_box.kind {
            LayoutKind::Text { baseline, .. } => *baseline,
            _ => unreachable!(),
        };

        assert_eq!(picture_box.bounds.y, text_box.bounds.y + 8);
        assert_eq!(
            picture_box.bounds.y + picture_box.bounds.height,
            text_box.bounds.y + text_baseline,
            "the bottom of an inline bitmap should share the text baseline"
        );
        assert_eq!(
            hotspot_box.bounds.y,
            picture_box.bounds.y,
            "picture hotspot overlays must follow the picture's vertical baseline shift"
        );
    }

    #[test]
    fn rtl_locale_reorders_hebrew_charset_boxes_but_keeps_text_logical() {
        let mut topic = topic_with_text("אחד שני", None);
        topic.scrolling[0].paragraphs[0].format.right_to_left = true;
        let mut fonts = FontTable::fallback();
        fonts.apply_system_metadata(&[0xB1], Some(0x040D));
        let mut measure = |_style: &ResolvedTextStyle, text: &str| TextMetrics {
            width: i32::try_from(text.chars().count()).unwrap_or(i32::MAX) * 10,
            height: 20,
            baseline: 16,
        };

        let layout = LayoutEngine::default().layout_topic_with_measurer(
            &topic,
            &fonts,
            320,
            &mut measure,
        );
        let text: Vec<_> = layout
            .scrolling
            .boxes
            .iter()
            .filter_map(|item| match &item.kind {
                LayoutKind::Text { text, style, .. } => Some((text.as_str(), style.charset, item.bounds.x)),
                _ => None,
            })
            .collect();

        assert_eq!(text.iter().map(|(value, _, _)| *value).collect::<Vec<_>>(), vec!["אחד", " ", "שני"]);
        assert!(text.iter().all(|(_, charset, _)| *charset == Some(0xB1)));
        assert!(text[0].2 > text[2].2, "logical first Hebrew word must be painted to the right");
    }

    #[test]
    fn charset_run_reordering_handles_mixed_base_directions() {
        let mut rtl_font = FontTable::fallback().descriptors()[0].clone();
        rtl_font.charset = Some(0xB1);
        let rtl = style_from_font(&rtl_font, None);
        let ltr = style_from_font(&FontTable::fallback().descriptors()[0], None);
        let make = |x, text: &str, style: ResolvedTextStyle| LayoutBox {
            bounds: Rect { x, y: 0, width: 10, height: 10 },
            kind: LayoutKind::Text {
                text: text.to_owned(),
                style,
                hotspot: None,
                baseline: 8,
                flow: TextFlow {
                    paragraph_id: 0,
                    line_index: 0,
                    hard_break_before: false,
                    segment_index: 0,
                    no_wrap: false,
                    reflow_safe: true,
                    content_left: 0,
                    content_right: 100,
                },
            },
        };
        let source = vec![
            make(0, "A", ltr.clone()),
            make(10, "א", rtl.clone()),
            make(20, "ב", rtl),
            make(30, "B", ltr),
        ];

        let mut ltr_base = source.clone();
        reorder_charset_text_runs(&mut ltr_base, false);
        assert_eq!(ltr_base.iter().map(|item| item.bounds.x).collect::<Vec<_>>(), vec![0, 20, 10, 30]);

        let mut rtl_base = source;
        reorder_charset_text_runs(&mut rtl_base, true);
        assert_eq!(rtl_base.iter().map(|item| item.bounds.x).collect::<Vec<_>>(), vec![30, 20, 10, 0]);
    }

    #[test]
    fn narrow_viewport_wraps_text_to_multiple_lines() {
        let topic = topic_with_text("alpha beta gamma delta epsilon", None);
        let layout = LayoutEngine::default().layout_topic(&topic, &FontTable::fallback(), 100);
        let ys: std::collections::BTreeSet<_> = layout
            .scrolling
            .boxes
            .iter()
            .filter_map(|item| matches!(&item.kind, LayoutKind::Text { .. }).then_some(item.bounds.y))
            .collect();
        assert!(ys.len() >= 2);
    }

    #[test]
    fn right_aligned_first_line_indent_stays_inside_viewport() {
        let mut topic = topic_with_text("aligned", None);
        topic.scrolling[0].paragraphs[0].format.alignment = ParagraphAlignment::Right;
        topic.scrolling[0].paragraphs[0].format.first_line_indent = Some(40);
        let layout = LayoutEngine::default().layout_topic(&topic, &FontTable::fallback(), 200);
        let rightmost = layout
            .scrolling
            .boxes
            .iter()
            .map(|item| item.bounds.x.saturating_add(item.bounds.width))
            .max()
            .unwrap();
        assert!(rightmost <= 200);
    }

    #[test]
    fn negative_first_line_indent_is_not_clamped_or_repeated_after_wrap() {
        let mut topic = topic_with_text("alpha beta gamma delta epsilon zeta eta theta", None);
        topic.scrolling[0].paragraphs[0].format.left_indent = Some(36);
        topic.scrolling[0].paragraphs[0].format.first_line_indent = Some(-18);
        let layout = LayoutEngine::default().layout_topic(&topic, &FontTable::fallback(), 110);

        let mut line_starts = std::collections::BTreeMap::<i32, i32>::new();
        for item in &layout.scrolling.boxes {
            if matches!(&item.kind, LayoutKind::Text { .. }) {
                line_starts
                    .entry(item.bounds.y)
                    .and_modify(|x| *x = (*x).min(item.bounds.x))
                    .or_insert(item.bounds.x);
            }
        }
        let starts: Vec<_> = line_starts.into_values().collect();
        assert!(starts.len() >= 2, "fixture should wrap onto continuation lines");
        assert!(
            starts[0] < starts[1],
            "the negative first-line indent must outdent only the first visual line"
        );
        assert!(
            starts[1..].iter().all(|x| *x == starts[1]),
            "continuation lines must return to the paragraph's ordinary left indent"
        );
    }

    #[test]
    fn empty_table_display_cells_do_not_stagger_independent_columns() {
        let visible = |text: &str| Paragraph {
            format: ParagraphFormat::default(),
            inlines: vec![Inline::Text(TextRun {
                text: text.to_owned(),
                font_index: 0,
                hotspot: None,
            })],
        };
        let empty = || Paragraph {
            format: ParagraphFormat::default(),
            inlines: Vec::new(),
        };

        let topic = TopicPresentation {
            id: TopicId(TopicPos(12)),
            title: "Table".to_owned(),
            non_scrolling: Vec::new(),
            scrolling: vec![FormattedRecord {
                topic_size: 0,
                topic_length: None,
                table: Some(TableInfo {
                    table_type: 1,
                    minimum_width: None,
                    columns: vec![
                        TableColumn { width: 100, gap_before: 0 },
                        TableColumn { width: 100, gap_before: 0 },
                    ],
                }),
                table_cells: vec![
                    TableCell {
                        column: 0,
                        content: TableCellContent::Display {
                            paragraph_start: 0,
                            paragraph_end: 2,
                        },
                    },
                    // CALC.HLP uses cells like this as structural fillers before later content
                    // in the same independent column. WinHlp32 gives this display zero height.
                    TableCell {
                        column: 1,
                        content: TableCellContent::Display {
                            paragraph_start: 2,
                            paragraph_end: 3,
                        },
                    },
                    TableCell {
                        column: 1,
                        content: TableCellContent::Display {
                            paragraph_start: 3,
                            paragraph_end: 5,
                        },
                    },
                ],
                paragraphs: vec![visible("A"), empty(), empty(), visible("B"), empty()],
                issues: Vec::new(),
            }],
            warnings: Vec::new(),
        };
        let mut measure = |_style: &ResolvedTextStyle, text: &str| TextMetrics {
            width: i32::try_from(text.chars().count()).unwrap_or(i32::MAX) * 10,
            height: 20,
            baseline: 16,
        };

        let layout = LayoutEngine::default().layout_topic_with_measurer(
            &topic,
            &FontTable::fallback(),
            360,
            &mut measure,
        );
        let mut text_boxes = layout.scrolling.boxes.iter().filter_map(|item| match &item.kind {
            LayoutKind::Text { text, .. } if text == "A" || text == "B" => {
                Some((text.as_str(), item.bounds))
            }
            _ => None,
        });
        let first = text_boxes.next().expect("first table cell text");
        let second = text_boxes.next().expect("second table cell text");
        assert_eq!(first.0, "A");
        assert_eq!(second.0, "B");
        assert_eq!(
            first.1.y, second.1.y,
            "empty compact display cells must not push one table column below its peers"
        );
    }

    #[test]
    fn rule_only_record_leaves_gap_before_following_table() {
        let rule = FormattedRecord {
            topic_size: 0,
            topic_length: None,
            table: None,
            table_cells: Vec::new(),
            paragraphs: vec![Paragraph {
                format: ParagraphFormat {
                    border: Some(BorderInfo {
                        flags: BorderFlags {
                            top: true,
                            ..BorderFlags::default()
                        },
                        style: BorderStyle::Normal,
                        raw_extra: [0, 0],
                    }),
                    ..ParagraphFormat::default()
                },
                inlines: Vec::new(),
            }],
            issues: Vec::new(),
        };
        let table = FormattedRecord {
            topic_size: 0,
            topic_length: None,
            table: Some(TableInfo {
                table_type: 1,
                minimum_width: None,
                columns: vec![TableColumn { width: 100, gap_before: 0 }],
            }),
            table_cells: vec![TableCell {
                column: 0,
                content: TableCellContent::Display {
                    paragraph_start: 0,
                    paragraph_end: 1,
                },
            }],
            paragraphs: vec![Paragraph {
                format: ParagraphFormat::default(),
                inlines: vec![Inline::Text(TextRun {
                    text: "header".to_owned(),
                    font_index: 0,
                    hotspot: None,
                })],
            }],
            issues: Vec::new(),
        };
        let topic = TopicPresentation {
            id: TopicId(TopicPos(12)),
            title: "Rule + table".to_owned(),
            non_scrolling: Vec::new(),
            scrolling: vec![rule, table],
            warnings: Vec::new(),
        };
        let mut measure = |_style: &ResolvedTextStyle, text: &str| TextMetrics {
            width: i32::try_from(text.chars().count()).unwrap_or(i32::MAX) * 10,
            height: 20,
            baseline: 16,
        };

        let layout = LayoutEngine::default().layout_topic_with_measurer(
            &topic,
            &FontTable::fallback(),
            360,
            &mut measure,
        );
        let rule_box = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::Border { .. }))
            .expect("rule border");
        let header = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::Text { text, .. } if text == "header"))
            .expect("table header");
        let rule_bottom = rule_box.bounds.y.saturating_add(rule_box.bounds.height);
        assert_eq!(
            header.bounds.y.saturating_sub(rule_bottom),
            TABLE_AFTER_RULE_GAP,
            "the first table row should start after the requested post-rule breathing room"
        );
    }

    #[test]
    fn border_only_paragraph_does_not_gain_a_synthetic_blank_text_line() {
        let mut topic = topic_with_text("", None);
        topic.scrolling[0].paragraphs[0].inlines.clear();
        topic.scrolling[0].paragraphs[0].format.left_indent = Some(18);
        topic.scrolling[0].paragraphs[0].format.right_indent = Some(24);
        topic.scrolling[0].paragraphs[0].format.border = Some(BorderInfo {
            flags: BorderFlags {
                top: true,
                bottom: true,
                ..BorderFlags::default()
            },
            style: BorderStyle::Normal,
            raw_extra: [0, 0],
        });

        let layout = LayoutEngine::default().layout_topic(&topic, &FontTable::fallback(), 320);
        let border = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::Border { .. }))
            .expect("border box should be retained");
        assert_eq!(
            border.bounds.height, 12,
            "compact Related Topics separators reserve one 12 px post-rule gap"
        );
        assert_eq!(
            border.bounds.x,
            PAGE_MARGIN,
            "separator ignores its own left indent and aligns to the following heading edge"
        );
        let expected_right = 320
            - LayoutEngine::default().optional_horizontal_paragraph_metric(Some(24));
        assert_eq!(
            border.bounds.x + border.bounds.width,
            expected_right,
            "separator preserves its authored right-side inset while fixing left alignment"
        );
    }

    #[test]
    fn hotspot_survives_layout_and_hit_testing() {
        let hotspot = Hotspot {
            target: HotspotTarget::Internal { offset: TopicOffset(77), popup: false },
            emphasized: true,
        };
        let topic = topic_with_text("click me", Some(hotspot.clone()));
        let layout = LayoutEngine::default().layout_topic(&topic, &FontTable::fallback(), 320);
        let text_box = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| item.hotspot().is_some())
            .unwrap();
        let hit_point = Point { x: text_box.bounds.x, y: text_box.bounds.y };
        let hit = layout.scrolling.hit_test(hit_point);
        assert_eq!(hit, Some(&hotspot));
        assert_eq!(layout.scrolling.hit_test_box(hit_point), Some(text_box));
    }

    #[test]
    fn resolved_style_preserves_original_face_but_exposes_native_family_intent() {
        let mut font = FontTable::fallback().descriptors()[0].clone();
        font.face_name = "Terminal".to_owned();
        font.family = crate::HlpFontFamily::Modern;
        font.point_size_twips = 150;
        let style = style_from_font(&font, None);
        assert_eq!(style.face_name, "Terminal");
        assert_eq!(style.family, ResolvedFontFamily::Monospace);
        assert_eq!(style.source_family, crate::HlpFontFamily::Modern);
        assert_eq!(style.point_size, 8);
        assert_eq!(style.point_size_twips, 150);

        font.family = crate::HlpFontFamily::Roman;
        let style = style_from_font(&font, None);
        assert_eq!(style.family, ResolvedFontFamily::Proportional);
        assert_eq!(style.source_family, crate::HlpFontFamily::Roman);
    }

    #[test]
    fn winhlp32_colour_inheritance_is_exact_not_a_near_black_heuristic() {
        assert!(is_inherit_colour(Rgb { red: 1, green: 1, blue: 0 }));
        assert!(!is_inherit_colour(Rgb { red: 0, green: 0, blue: 0 }));
        assert!(!is_inherit_colour(Rgb { red: 2, green: 1, blue: 0 }));
        assert!(!is_inherit_colour(Rgb { red: 8, green: 8, blue: 8 }));
        assert!(!is_inherit_colour(Rgb { red: 128, green: 0, blue: 128 }));

        let mut font = FontTable::fallback().descriptors()[0].clone();
        font.foreground = Rgb { red: 1, green: 1, blue: 0 };
        font.background = Rgb { red: 1, green: 1, blue: 0 };
        let inherited = style_from_font(&font, None);
        assert!(inherited.foreground_inherits);
        assert!(inherited.background_inherits);

        font.foreground = Rgb { red: 8, green: 8, blue: 8 };
        font.background = Rgb { red: 7, green: 6, blue: 5 };
        let authored = style_from_font(&font, None);
        assert!(!authored.foreground_inherits);
        assert!(!authored.background_inherits);
        assert_eq!(authored.foreground, Rgb { red: 8, green: 8, blue: 8 });
        assert_eq!(authored.background, Rgb { red: 7, green: 6, blue: 5 });
    }

    #[test]
    fn border_clearance_matches_reference_style_helper() {
        assert_eq!(reference_border_clearance(BorderStyle::Normal), 5);
        assert_eq!(reference_border_clearance(BorderStyle::Thick), 6);
        assert_eq!(reference_border_clearance(BorderStyle::Double), 7);
        assert_eq!(reference_border_clearance(BorderStyle::Shadow), 6);
        assert_eq!(reference_border_clearance(BorderStyle::ReferenceStyle4), 5);
        assert_eq!(reference_border_clearance(BorderStyle::Reserved(7)), 0);
    }

    #[test]
    fn paragraph_metrics_use_reference_dpi_over_144_truncation() {
        let engine = LayoutEngine::new(96);
        assert_eq!(engine.paragraph_metric_to_pixels(72), 48);
        assert_eq!(engine.paragraph_metric_to_pixels(1), 0);
        assert_eq!(engine.paragraph_metric_to_pixels(-1), 0);
        assert_eq!(engine.paragraph_metric_to_pixels(-72), -48);
    }

    #[test]
    fn paragraph_metrics_use_the_correct_device_axis() {
        let engine = LayoutEngine::with_dpi(120, 144);
        assert_eq!(engine.paragraph_metric_to_pixels(72), 60);
        assert_eq!(engine.paragraph_vertical_metric_to_pixels(72), 72);
        assert_eq!(engine.paragraph_metric_to_pixels(-72), -60);
        assert_eq!(engine.paragraph_vertical_metric_to_pixels(-72), -72);
    }

    #[test]
    fn table_geometry_matches_reference_type_zero_and_absolute_modes() {
        let engine = LayoutEngine::new(96);

        let relative = TableInfo {
            table_type: 0,
            minimum_width: Some(0),
            columns: vec![
                crate::TableColumn { width: 16_384, gap_before: 0 },
                crate::TableColumn { width: 16_384, gap_before: 0 },
            ],
        };
        let geometry = engine.table_column_geometry(&relative, 300);
        assert_eq!(geometry.len(), 2);
        assert_eq!(geometry[0], TableColumnGeometry { x: 0, width: 150 });
        assert_eq!(geometry[1], TableColumnGeometry { x: 150, width: 150 });

        let absolute = TableInfo {
            table_type: 1,
            minimum_width: None,
            columns: vec![crate::TableColumn { width: 144, gap_before: 72 }],
        };
        let geometry = engine.table_column_geometry(&absolute, 300);
        assert_eq!(geometry[0], TableColumnGeometry { x: 48, width: 96 });
    }

    #[test]
    fn table_columns_advance_independently_instead_of_forming_rows() {
        let table = TableInfo {
            table_type: 1,
            minimum_width: None,
            columns: vec![
                crate::TableColumn { width: 216, gap_before: 0 },
                crate::TableColumn { width: 216, gap_before: 0 },
            ],
        };
        let make_paragraph = |column, text: &str| Paragraph {
            format: ParagraphFormat {
                column: Some(column),
                ..ParagraphFormat::default()
            },
            inlines: vec![Inline::Text(TextRun {
                text: text.to_owned(),
                font_index: 0,
                hotspot: None,
            })],
        };
        let record = FormattedRecord {
            topic_size: 0,
            topic_length: None,
            table: Some(table),
            table_cells: Vec::new(),
            paragraphs: vec![
                make_paragraph(0, "A"),
                make_paragraph(1, "B"),
                make_paragraph(0, "C"),
            ],
            issues: Vec::new(),
        };
        let topic = TopicPresentation {
            id: TopicId(TopicPos(12)),
            title: "table".to_owned(),
            non_scrolling: Vec::new(),
            scrolling: vec![record],
            warnings: Vec::new(),
        };
        let mut measure = |_style: &ResolvedTextStyle, text: &str| TextMetrics {
            width: 10,
            height: if text == "B" { 30 } else { 10 },
            baseline: 16,
        };
        let layout = LayoutEngine::new(96).layout_topic_with_measurer(
            &topic,
            &FontTable::fallback(),
            360,
            &mut measure,
        );
        let text: Vec<_> = layout
            .scrolling
            .boxes
            .iter()
            .filter_map(|item| match &item.kind {
                LayoutKind::Text { text, .. } => Some((text.as_str(), item.bounds)),
                _ => None,
            })
            .collect();
        let a = text.iter().find(|(value, _)| *value == "A").unwrap().1;
        let b = text.iter().find(|(value, _)| *value == "B").unwrap().1;
        let c = text.iter().find(|(value, _)| *value == "C").unwrap().1;
        assert_eq!(a.y, b.y);
        assert_eq!(c.y, a.y + 10);
        assert!(c.y < b.y + b.height);
    }

    #[test]
    fn recursively_nested_table_height_advances_only_the_parent_column() {
        let outer = TableInfo {
            table_type: 1,
            minimum_width: None,
            columns: vec![crate::TableColumn {
                width: 432,
                gap_before: 0,
            }],
        };
        let inner = FormattedTable {
            info: TableInfo {
                table_type: 1,
                minimum_width: None,
                columns: vec![
                    crate::TableColumn {
                        width: 216,
                        gap_before: 0,
                    },
                    crate::TableColumn {
                        width: 216,
                        gap_before: 0,
                    },
                ],
            },
            cells: vec![
                TableCell {
                    column: 0,
                    content: TableCellContent::Display {
                        paragraph_start: 0,
                        paragraph_end: 1,
                    },
                },
                TableCell {
                    column: 1,
                    content: TableCellContent::Display {
                        paragraph_start: 1,
                        paragraph_end: 2,
                    },
                },
            ],
        };
        let paragraph = |text: &str| Paragraph {
            format: ParagraphFormat::default(),
            inlines: vec![Inline::Text(TextRun {
                text: text.to_owned(),
                font_index: 0,
                hotspot: None,
            })],
        };
        let record = FormattedRecord {
            topic_size: 0,
            topic_length: None,
            table: Some(outer),
            table_cells: vec![
                TableCell {
                    column: 0,
                    content: TableCellContent::Table(Box::new(inner)),
                },
                TableCell {
                    column: 0,
                    content: TableCellContent::Display {
                        paragraph_start: 2,
                        paragraph_end: 3,
                    },
                },
            ],
            paragraphs: vec![paragraph("A"), paragraph("B"), paragraph("C")],
            issues: Vec::new(),
        };
        let topic = TopicPresentation {
            id: TopicId(TopicPos(12)),
            title: "nested table".to_owned(),
            non_scrolling: Vec::new(),
            scrolling: vec![record],
            warnings: Vec::new(),
        };
        let mut measure = |_style: &ResolvedTextStyle, text: &str| TextMetrics {
            width: 10,
            height: if text == "B" { 30 } else { 10 },
            baseline: 16,
        };
        let layout = LayoutEngine::new(96).layout_topic_with_measurer(
            &topic,
            &FontTable::fallback(),
            600,
            &mut measure,
        );
        let text: Vec<_> = layout
            .scrolling
            .boxes
            .iter()
            .filter_map(|item| match &item.kind {
                LayoutKind::Text { text, .. } => Some((text.as_str(), item.bounds)),
                _ => None,
            })
            .collect();
        let a = text.iter().find(|(value, _)| *value == "A").unwrap().1;
        let b = text.iter().find(|(value, _)| *value == "B").unwrap().1;
        let c = text.iter().find(|(value, _)| *value == "C").unwrap().1;

        // The nested table uses the outer column's x origin and splits that width between its
        // own columns. Its returned height is max(10, 30) = 30, so the following outer cell
        // starts 30 pixels below A/B rather than after only the first inner column.
        assert_eq!(a.y, b.y);
        assert!(b.x > a.x);
        assert_eq!(c.x, a.x);
        assert_eq!(c.y, a.y + 30);
    }

    #[test]
    fn hosted_window_placeholder_wraps_before_shrinking_into_line_remainder() {
        let record = FormattedRecord {
            topic_size: 0,
            topic_length: None,
            table: None,
            table_cells: Vec::new(),
            paragraphs: vec![Paragraph {
                format: ParagraphFormat::default(),
                inlines: vec![
                    Inline::Text(TextRun {
                        text: "prefix".to_owned(),
                        font_index: 0,
                        hotspot: None,
                    }),
                    Inline::EmbeddedWindow(crate::EmbeddedWindowReference {
                        record_type: crate::TopicRecordType::EmbeddedWindow,
                        raw_prefix: [0; 6],
                        descriptor: "BUTTON".to_owned(),
                        payload_size: 13,
                    }),
                ],
            }],
            issues: Vec::new(),
        };
        let topic = TopicPresentation {
            id: TopicId(TopicPos(12)),
            title: "hosted window".to_owned(),
            non_scrolling: Vec::new(),
            scrolling: vec![record],
            warnings: Vec::new(),
        };
        let mut measure = |_style: &ResolvedTextStyle, _text: &str| TextMetrics {
            width: 80,
            height: 10,
            baseline: 8,
        };
        let layout = LayoutEngine::new(96).layout_topic_with_measurer(
            &topic,
            &FontTable::fallback(),
            240,
            &mut measure,
        );
        let text = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::Text { .. }))
            .unwrap();
        let hosted = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::EmbeddedWindowPlaceholder { .. }))
            .unwrap();

        assert!(hosted.bounds.y > text.bounds.y);
        assert_eq!(hosted.bounds.x, text.bounds.x);
        assert_eq!(hosted.bounds.width, 192);
        assert_eq!(hosted.bounds.height, 192);
    }

    #[test]
    fn related_topics_alink_row_aligns_to_rendered_rule_and_moves_down() {
        let mut rule_format = ParagraphFormat::default();
        // Use a one-edge rule with an authored indent so its rendered x is deliberately not the
        // region PAGE_MARGIN. The regression must therefore follow actual border geometry rather
        // than accidentally passing because both objects were normalized to the page edge.
        rule_format.left_indent = Some(36);
        rule_format.border = Some(BorderInfo {
            flags: BorderFlags {
                top: true,
                ..BorderFlags::default()
            },
            style: BorderStyle::Normal,
            raw_extra: [0, 0],
        });
        let rule = FormattedRecord {
            topic_size: 0,
            topic_length: None,
            table: None,
            table_cells: Vec::new(),
            paragraphs: vec![Paragraph {
                format: rule_format,
                inlines: Vec::new(),
            }],
            issues: Vec::new(),
        };
        let mut related_format = ParagraphFormat::default();
        related_format.left_indent = Some(-21);
        related_format.first_line_indent = Some(9);
        let related = FormattedRecord {
            topic_size: 0,
            topic_length: None,
            table: None,
            table_cells: Vec::new(),
            paragraphs: vec![Paragraph {
                format: related_format,
                inlines: vec![
                    Inline::EmbeddedWindow(crate::EmbeddedWindowReference {
                        record_type: crate::TopicRecordType::EmbeddedWindow30,
                        raw_prefix: [1, 0, 0, 0, 0x38, 0x5C],
                        descriptor: r#"!,AL("FIRST;SECOND")"#.to_owned(),
                        payload_size: 32,
                    }),
                    Inline::Text(TextRun {
                        text: " Related Topics".to_owned(),
                        font_index: 0,
                        hotspot: None,
                    }),
                ],
            }],
            issues: Vec::new(),
        };
        let topic = TopicPresentation {
            id: TopicId(TopicPos(12)),
            title: "Related".to_owned(),
            non_scrolling: Vec::new(),
            scrolling: vec![rule, related],
            warnings: Vec::new(),
        };
        let layout = LayoutEngine::new(96).layout_topic(&topic, &FontTable::fallback(), 360);
        let rule_box = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::Border { .. }))
            .expect("rule");
        let button = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::EmbeddedWindowPlaceholder { .. }))
            .expect("button");
        let label = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::Text { .. }))
            .expect("related topics label");

        assert_ne!(
            rule_box.bounds.x,
            PAGE_MARGIN,
            "test rule must exercise a non-margin anchor"
        );
        assert_eq!(button.bounds.x, rule_box.bounds.x);
        assert_eq!(label.bounds.x, button.bounds.x + button.bounds.width);
        assert_eq!(
            label
                .bounds
                .y
                .saturating_sub(rule_box.bounds.y.saturating_add(rule_box.bounds.height)),
            RELATED_TOPICS_AFTER_RULE_GAP,
            "the visual line still starts at the authored post-rule gap"
        );
        let label_baseline = match &label.kind {
            LayoutKind::Text { baseline, .. } => *baseline,
            _ => unreachable!(),
        };
        assert_eq!(
            button.bounds.y.saturating_add(button.bounds.height),
            label.bounds.y.saturating_add(label_baseline),
            "the stock ALink button bottom must share the adjacent text baseline"
        );
        assert!(matches!(
            button.hotspot(),
            Some(Hotspot { target: crate::HotspotTarget::Macro(text), .. }) if text == r#"AL("FIRST;SECOND")"#
        ));
    }

    #[test]
    fn stock_alink_button_tracks_zoomed_text_baseline() {
        let record = FormattedRecord {
            topic_size: 0,
            topic_length: None,
            table: None,
            table_cells: Vec::new(),
            paragraphs: vec![Paragraph {
                format: ParagraphFormat::default(),
                inlines: vec![
                    Inline::EmbeddedWindow(crate::EmbeddedWindowReference {
                        record_type: crate::TopicRecordType::EmbeddedWindow30,
                        raw_prefix: [1, 0, 0, 0, 0x38, 0x5C],
                        descriptor: r#"!,AL("FIRST;SECOND")"#.to_owned(),
                        payload_size: 32,
                    }),
                    Inline::Text(TextRun {
                        text: " Related Topics".to_owned(),
                        font_index: 0,
                        hotspot: None,
                    }),
                ],
            }],
            issues: Vec::new(),
        };
        let topic = TopicPresentation {
            id: TopicId(TopicPos(12)),
            title: "Related".to_owned(),
            non_scrolling: Vec::new(),
            scrolling: vec![record],
            warnings: Vec::new(),
        };

        for (height, baseline) in [(14, 10), (20, 16), (40, 32)] {
            let mut measure = |_style: &ResolvedTextStyle, text: &str| TextMetrics {
                width: i32::try_from(text.chars().count()).unwrap_or(i32::MAX) * 10,
                height,
                baseline,
            };
            let layout = LayoutEngine::with_dpi_and_text_zoom(96, 96, 200)
                .layout_topic_with_measurer(
                    &topic,
                    &FontTable::fallback(),
                    360,
                    &mut measure,
                );
            let button = layout
                .scrolling
                .boxes
                .iter()
                .find(|item| {
                    matches!(
                        &item.kind,
                        LayoutKind::EmbeddedWindowPlaceholder {
                            standard_button_label: Some(_),
                            ..
                        }
                    )
                })
                .expect("stock ALink button");
            let label = layout
                .scrolling
                .boxes
                .iter()
                .find(|item| matches!(&item.kind, LayoutKind::Text { .. }))
                .expect("related topics text");
            let text_baseline = match &label.kind {
                LayoutKind::Text { baseline, .. } => *baseline,
                _ => unreachable!(),
            };

            assert_eq!(
                button.bounds.y.saturating_add(button.bounds.height),
                label.bounds.y.saturating_add(text_baseline),
                "the fixed-size stock button must follow the measured text baseline at every zoom"
            );
        }
    }

    #[test]
    fn standalone_alink_row_preserves_authored_paragraph_indents() {
        let mut format = ParagraphFormat::default();
        format.left_indent = Some(30);
        format.first_line_indent = Some(15);
        let record = FormattedRecord {
            topic_size: 0,
            topic_length: None,
            table: None,
            table_cells: Vec::new(),
            paragraphs: vec![Paragraph {
                format,
                inlines: vec![Inline::EmbeddedWindow(crate::EmbeddedWindowReference {
                    record_type: crate::TopicRecordType::EmbeddedWindow30,
                    raw_prefix: [1, 0, 0, 0, 0x38, 0x5C],
                    descriptor: r#"!,AL("ONLY")"#.to_owned(),
                    payload_size: 24,
                })],
            }],
            issues: Vec::new(),
        };
        let topic = TopicPresentation {
            id: TopicId(TopicPos(12)),
            title: "Related".to_owned(),
            non_scrolling: Vec::new(),
            scrolling: vec![record],
            warnings: Vec::new(),
        };
        let engine = LayoutEngine::new(96);
        let layout = engine.layout_topic(&topic, &FontTable::fallback(), 360);
        let button = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::EmbeddedWindowPlaceholder { .. }))
            .expect("button");
        let expected_x = PAGE_MARGIN
            + engine.paragraph_metric_to_pixels(30)
            + engine.paragraph_metric_to_pixels(15);
        assert_eq!(button.bounds.x, expected_x);
    }

    #[test]
    fn calc_related_topics_hosted_object_uses_native_blank_button_geometry() {
        let window = crate::EmbeddedWindowReference {
            record_type: crate::TopicRecordType::EmbeddedWindow30,
            raw_prefix: [1, 0, 0, 0, 0x38, 0x5C],
            descriptor: r#"!,AL("A_CALC_LIST_EQUIV;A_CALC_KEYB_SEQ")"#.to_owned(),
            payload_size: 48,
        };
        let (width, height, label) = embedded_window_geometry(&window, 400, 96, 96);
        assert_eq!(width, STANDARD_BUTTON_EMPTY_SIZE);
        assert_eq!(height, STANDARD_BUTTON_EMPTY_SIZE);
        assert_eq!(label, Some(String::new()));
    }

    #[test]
    fn authored_hosted_control_placeholder_uses_reference_two_inch_creation_size() {
        let window = crate::EmbeddedWindowReference {
            record_type: crate::TopicRecordType::EmbeddedWindow,
            raw_prefix: [0; 6],
            descriptor: "CUSTOM_CONTROL".to_owned(),
            payload_size: 20,
        };
        let (width, height, label) = embedded_window_geometry(&window, 1000, 120, 144);
        assert_eq!(width, 240);
        assert_eq!(height, 288);
        assert_eq!(label, None);

        let (bounded_width, _, _) = embedded_window_geometry(&window, 180, 120, 144);
        assert_eq!(bounded_width, 180);
    }

    #[test]
    fn control85_resets_horizontal_line_origin_without_emitting_a_box() {
        let record = FormattedRecord {
            topic_size: 0,
            topic_length: None,
            table: None,
            table_cells: Vec::new(),
            paragraphs: vec![Paragraph {
                format: ParagraphFormat::default(),
                inlines: vec![
                    Inline::Control85(60),
                    Inline::Text(TextRun {
                        text: "X".to_owned(),
                        font_index: 0,
                        hotspot: None,
                    }),
                ],
            }],
            issues: Vec::new(),
        };
        let topic = TopicPresentation {
            id: TopicId(TopicPos(12)),
            title: "control85".to_owned(),
            non_scrolling: Vec::new(),
            scrolling: vec![record],
            warnings: Vec::new(),
        };
        let mut measure = |_style: &ResolvedTextStyle, _text: &str| TextMetrics {
            width: 10,
            height: 10,
            baseline: 8,
        };
        let layout = LayoutEngine::new(96).layout_topic_with_measurer(
            &topic,
            &FontTable::fallback(),
            320,
            &mut measure,
        );
        let text = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::Text { .. }))
            .expect("text after 0x85");
        assert_eq!(text.bounds.x, PAGE_MARGIN + 60);
    }

    #[test]
    fn old_half_point_metrics_follow_native_winhelp_rounding() {
        let engine = LayoutEngine::default();
        assert_eq!(engine.metric_to_pixels(16, FontMetric::HalfPoints), 10);
        assert_eq!(engine.metric_to_pixels(0, FontMetric::HalfPoints), 0);
        assert_eq!(engine.metric_to_pixels(-16, FontMetric::HalfPoints), -10);
    }

    #[test]
    fn native_text_height_is_not_inflated_by_the_approximate_fallback() {
        let native = normalized_metrics(
            TextMetrics { width: 20, height: 9, baseline: 7 },
            14,
        );
        assert_eq!(native.height, 9);
        assert_eq!(native.baseline, 7);
        let missing = normalized_metrics(
            TextMetrics { width: 20, height: 0, baseline: 0 },
            14,
        );
        assert_eq!(missing.height, 14);
        assert_eq!(missing.baseline, 11);
    }

    fn topic_with_picture(position: PicturePosition, hotspots: Vec<crate::PictureHotspot>, text: &str) -> TopicPresentation {
        let picture = PictureReference {
            command: match position {
                PicturePosition::Inline => 0x86,
                PicturePosition::FloatLeft => 0x87,
                PicturePosition::FloatRight => 0x88,
            },
            position,
            picture_type: 0x03,
            encoded_size: 4,
            hotspot_count: Some(u16::try_from(hotspots.len()).unwrap()),
            source: crate::PictureSource::Indexed(1),
            image: Some(DecodedPicture {
                width: 80,
                height: 64,
                rgba: std::sync::Arc::from(vec![255_u8; 80 * 64 * 4]),
                has_alpha: false,
                sizing: PictureSizing::Pixels,
                hotspots: std::sync::Arc::from([]),
            }),
            hotspots,
            decode_warning: None,
        };
        TopicPresentation {
            id: TopicId(TopicPos(12)),
            title: "Picture test".to_owned(),
            non_scrolling: Vec::new(),
            scrolling: vec![FormattedRecord {
                topic_size: 0,
                topic_length: None,
                table: None,
                table_cells: Vec::new(),
                paragraphs: vec![Paragraph {
                    format: ParagraphFormat::default(),
                    inlines: vec![
                        Inline::Picture(picture),
                        Inline::Text(TextRun {
                            text: text.to_owned(),
                            font_index: 0,
                            hotspot: None,
                        }),
                    ],
                }],
                issues: Vec::new(),
            }],
            warnings: Vec::new(),
        }
    }

    #[test]
    fn picture_hotspots_scale_with_the_displayed_image_and_hit_test() {
        let hotspot = Hotspot {
            target: HotspotTarget::Internal {
                offset: TopicOffset(99),
                popup: false,
            },
            emphasized: false,
        };
        let topic = topic_with_picture(
            PicturePosition::Inline,
            vec![crate::PictureHotspot {
                x: 20,
                y: 16,
                width: 40,
                height: 32,
                hotspot: hotspot.clone(),
            }],
            "",
        );
        let layout = LayoutEngine::default().layout_topic(&topic, &FontTable::fallback(), 320);
        let image = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::Picture { .. }))
            .unwrap();
        let overlay = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::PictureHotspot { .. }))
            .unwrap();
        assert_eq!(overlay.bounds.x, image.bounds.x + 20);
        assert_eq!(overlay.bounds.y, image.bounds.y + 16);
        assert_eq!(overlay.bounds.width, 40);
        assert_eq!(overlay.bounds.height, 32);
        let hit = layout.scrolling.hit_test(Point {
            x: overlay.bounds.x + 1,
            y: overlay.bounds.y + 1,
        });
        assert_eq!(hit, Some(&hotspot));
    }

    #[test]
    fn left_float_wraps_text_then_releases_full_width_below_picture() {
        let topic = topic_with_picture(
            PicturePosition::FloatLeft,
            Vec::new(),
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron",
        );
        let layout = LayoutEngine::default().layout_topic(&topic, &FontTable::fallback(), 220);
        let picture = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::Picture { .. }))
            .unwrap();
        let text: Vec<_> = layout
            .scrolling
            .boxes
            .iter()
            .filter(|item| matches!(&item.kind, LayoutKind::Text { .. }))
            .collect();
        assert!(text.iter().any(|item| {
            item.bounds.y < picture.bounds.y + picture.bounds.height
                && item.bounds.x >= picture.bounds.x + picture.bounds.width
        }));
        assert!(text.iter().any(|item| {
            item.bounds.y >= picture.bounds.y + picture.bounds.height
                && item.bounds.x < picture.bounds.x + picture.bounds.width
        }));
    }

    #[test]
    fn right_float_keeps_overlapping_text_to_its_left() {
        let topic = topic_with_picture(
            PicturePosition::FloatRight,
            Vec::new(),
            "alpha beta gamma delta epsilon zeta eta theta iota",
        );
        let layout = LayoutEngine::default().layout_topic(&topic, &FontTable::fallback(), 220);
        let picture = layout
            .scrolling
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, LayoutKind::Picture { .. }))
            .unwrap();
        let overlapping_text: Vec<_> = layout
            .scrolling
            .boxes
            .iter()
            .filter(|item| {
                matches!(&item.kind, LayoutKind::Text { .. })
                    && item.bounds.y < picture.bounds.y + picture.bounds.height
            })
            .collect();
        assert!(!overlapping_text.is_empty());
        assert!(overlapping_text.iter().all(|item| {
            item.bounds.x.saturating_add(item.bounds.width) <= picture.bounds.x
        }));
    }

    #[test]
    fn decoded_picture_uses_natural_size_and_shrinks_proportionally() {
        let picture = PictureReference {
            command: 0x86,
            position: PicturePosition::Inline,
            picture_type: 0x03,
            encoded_size: 4,
            hotspot_count: None,
            source: crate::PictureSource::Indexed(1),
            image: Some(DecodedPicture {
                width: 800,
                height: 400,
                rgba: std::sync::Arc::from(Vec::<u8>::new()),
                has_alpha: false,
                sizing: PictureSizing::Pixels,
                hotspots: std::sync::Arc::from([]),
            }),
            hotspots: Vec::new(),
            decode_warning: None,
        };
        let engine = LayoutEngine::default();
        assert_eq!(engine.picture_display_size(&picture, 1_000), (800, 400));
        assert_eq!(engine.picture_display_size(&picture, 200), (200, 100));
    }

    #[test]
    fn bitmap_authored_resolution_uses_separate_device_axes() {
        let mut picture = PictureReference {
            command: 0x86,
            position: PicturePosition::Inline,
            picture_type: 0x03,
            encoded_size: 4,
            hotspot_count: None,
            source: crate::PictureSource::Indexed(1),
            image: Some(DecodedPicture {
                width: 240,
                height: 288,
                rgba: std::sync::Arc::from(Vec::<u8>::new()),
                has_alpha: false,
                sizing: PictureSizing::BitmapResolution {
                    x_resolution: 120,
                    y_resolution: 144,
                },
                hotspots: std::sync::Arc::from([]),
            }),
            hotspots: Vec::new(),
            decode_warning: None,
        };
        let engine = LayoutEngine::with_dpi(144, 192);
        assert_eq!(engine.picture_display_size(&picture, 2_000), (288, 384));

        picture.image.as_mut().unwrap().sizing = PictureSizing::Pixels;
        assert_eq!(engine.picture_display_size(&picture, 2_000), (240, 288));
    }

    #[test]
    fn wmf_physical_mapping_uses_target_device_dpi() {
        let picture = PictureReference {
            command: 0x86,
            position: PicturePosition::Inline,
            picture_type: 0x03,
            encoded_size: 4,
            hotspot_count: None,
            source: crate::PictureSource::Indexed(1),
            image: Some(DecodedPicture {
                // Stable 96-DPI decode raster; layout must not mistake it for natural size.
                width: 96,
                height: 96,
                rgba: std::sync::Arc::from(Vec::<u8>::new()),
                has_alpha: false,
                sizing: PictureSizing::Metafile {
                    mapping_mode: 8,
                    logical_width: 2_540,
                    logical_height: 2_540,
                },
                hotspots: std::sync::Arc::from([]),
            }),
            hotspots: Vec::new(),
            decode_warning: None,
        };
        let engine = LayoutEngine::with_dpi(120, 144);
        assert_eq!(engine.picture_display_size(&picture, 2_000), (120, 144));
    }


    #[test]
    fn reference_default_tab_is_half_an_inch_at_96_dpi() {
        let format = ParagraphFormat::default();
        let target = next_tab_target(&format, 0, &LayoutEngine::new(96));
        assert_eq!(target.position, 48);
        assert_eq!(target.alignment, TabAlignment::Left);
    }

    #[test]
    fn authored_tab_interval_and_custom_alignment_are_preserved() {
        let mut format = ParagraphFormat {
            default_tab_interval: Some(144),
            ..ParagraphFormat::default()
        };
        let engine = LayoutEngine::new(96);
        assert_eq!(
            next_tab_target(&format, 0, &engine).position,
            96
        );
        format.tabs.push(crate::TabStop {
            position: 72,
            alignment: TabAlignment::Right,
        });
        let target = next_tab_target(&format, 0, &engine);
        assert_eq!(target.position, 48);
        assert_eq!(target.alignment, TabAlignment::Right);
    }

    #[test]
    fn deferred_right_and_center_tabs_align_the_following_segment() {
        let text_style = resolve_style(
            &TextRun { text: String::new(), font_index: 0, hotspot: None },
            &FontTable::fallback(),
        );
        let make_box = |x, width| LayoutBox {
            bounds: Rect { x, y: 0, width, height: 10 },
            kind: LayoutKind::Text {
                text: "x".to_owned(),
                style: text_style.clone(),
                hotspot: None,
                baseline: 8,
                flow: TextFlow {
                    paragraph_id: 0,
                    line_index: 0,
                    hard_break_before: false,
                    segment_index: 0,
                    no_wrap: false,
                    reflow_safe: true,
                    content_left: 0,
                    content_right: 100,
                },
            },
        };

        let mut right_boxes = vec![make_box(0, 10), make_box(10, 20)];
        let mut right_line = LineState {
            start_x: 0, x: 30, y: 0, height: 10, box_start: 0, box_end: 2,
        };
        let mut pending = Some(PendingTab {
            alignment: TabAlignment::Right, target_x: 100, box_start: 1,
        });
        resolve_pending_tab(&mut pending, &mut right_line, &mut right_boxes);
        assert_eq!(right_boxes[0].bounds.x, 0);
        assert_eq!(right_boxes[1].bounds.x + right_boxes[1].bounds.width, 100);

        let mut center_boxes = vec![make_box(0, 10), make_box(10, 20)];
        let mut center_line = LineState {
            start_x: 0, x: 30, y: 0, height: 10, box_start: 0, box_end: 2,
        };
        let mut pending = Some(PendingTab {
            alignment: TabAlignment::Center, target_x: 100, box_start: 1,
        });
        resolve_pending_tab(&mut pending, &mut center_line, &mut center_boxes);
        assert_eq!(center_boxes[1].bounds.x + center_boxes[1].bounds.width / 2, 100);
    }

    #[test]
    fn signed_line_spacing_matches_winhlp32_minimum_and_exact_semantics() {
        let engine = LayoutEngine::new(96);
        let minimum = ParagraphFormat {
            spacing_lines: Some(72),
            ..ParagraphFormat::default()
        };
        assert_eq!(engine.line_advance(&minimum, 20), 48);
        assert_eq!(engine.line_advance(&minimum, 60), 60);

        let exact = ParagraphFormat {
            spacing_lines: Some(-72),
            ..ParagraphFormat::default()
        };
        assert_eq!(engine.line_advance(&exact, 60), 48);
    }

    #[test]
    fn viewer_zoom_scales_authored_line_advance_without_scaling_device_dpi() {
        let exact = ParagraphFormat {
            spacing_lines: Some(-72),
            ..ParagraphFormat::default()
        };
        let minimum = ParagraphFormat {
            spacing_lines: Some(72),
            ..ParagraphFormat::default()
        };
        let native = LayoutEngine::with_dpi_and_text_zoom(96, 96, 100);
        let doubled = LayoutEngine::with_dpi_and_text_zoom(96, 96, 200);

        assert_eq!(native.line_advance(&exact, 60), 48);
        assert_eq!(doubled.line_advance(&exact, 120), 96);
        assert_eq!(native.line_advance(&minimum, 20), 48);
        assert_eq!(doubled.line_advance(&minimum, 40), 96);
        // Device paragraph geometry remains anchored to the real DPI instead of being doubled.
        assert_eq!(native.paragraph_metric_to_pixels(144), 96);
        assert_eq!(doubled.paragraph_metric_to_pixels(144), 96);
    }

    #[test]
    fn small_caps_uses_the_reference_two_thirds_font_height() {
        let run = TextRun { text: "Test".to_owned(), font_index: 0, hotspot: None };
        let fonts = FontTable::fallback();
        let mut style = resolve_style(&run, &fonts);
        let original = style.point_size_twips;
        style.small_caps = true;
        assert_eq!(effective_point_size_twips(&style), original * 2 / 3);
    }
    #[test]
    fn metric_conversion_distinguishes_half_points_and_twips() {
        let engine = LayoutEngine::new(96);
        assert_eq!(engine.metric_to_pixels(144, FontMetric::HalfPoints), 96);
        assert_eq!(engine.metric_to_pixels(1440, FontMetric::Twips), 96);
    }
}
