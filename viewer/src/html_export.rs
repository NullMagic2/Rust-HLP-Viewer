//! Self-contained HTML export for decoded WinHelp documents.
//!
//! The exporter consumes the same formatting-decoded presentations as the native viewer, but maps
//! ordinary WinHelp document semantics to ordinary HTML flow instead of replaying GDI text boxes.
//! Paragraph margins, first-line indents, alignment, authored line breaks, tabs, fonts, borders,
//! tables, pictures, and hotspots are translated from the decoded HLP structures. Retained layout
//! remains available only as a compatibility fallback for structures that cannot be represented
//! safely as document flow. Legacy retained-box helpers remain isolated in the module, but normal
//! exported topic templates do not invoke them. This keeps browser font shaping and word wrapping
//! native and prevents metric-reconciliation code from squeezing, overlapping, or double-wrapping text.
//!
//! One exported HTML file can contain the navigation-root HLP plus relative cross-document HLPs
//! referenced by Contents, hotspots, CONFIG/topic macros, and the root's integrated Index/Search
//! catalogs. The navigation pane intentionally stays anchored to the original document, matching
//! the desktop viewer's cross-document policy.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use hlp::{
    BorderFlags, BorderInfo, BorderStyle, EmbeddedWindowReference, FontDescriptor, FontTable,
    FormattedRecord, HelpDocument, HelpMacro, HelpMacroProgram, Hotspot, HotspotTarget,
    Inline, LayoutBox, LayoutKind, Paragraph, ParagraphAlignment, ParagraphFormat,
    PicturePosition, PictureReference, RegionLayout, ResolvedFontFamily, ResolvedTextStyle, Rgb,
    SafeHelpMacro, TabAlignment, TableCellContent, TableInfo, TextRun, WindowDefinition,
    resolve_external_help_path,
};

pub const DEFAULT_EXPORT_LAYOUT_WIDTH: i32 = 860;
const MIN_EXPORT_LAYOUT_WIDTH: i32 = 320;
const MAX_EXPORT_LAYOUT_WIDTH: i32 = 4096;
const EXPORT_DPI: i32 = 96;
/// Rendered size in CSS pixels of a 10 pt authored font.
///
/// WinHlp32 draws at the host display DPI, so the verified 96 DPI reference conversion puts a 10 pt
/// authored font at 13.333 px, which reads slightly small in a browser. Every authored font size is
/// therefore multiplied by `EXPORT_BASE_FONT_PX / 13.333`, a single factor that leaves every
/// relative size relationship in the document exactly as authored. Paragraph indents, tab stops,
/// spacing, and table geometry deliberately keep the unscaled 96 DPI reference conversion, so this
/// constant is the one place the exported type scale is tuned.
const EXPORT_BASE_FONT_PX: f64 = 14.0;
const EXPORT_BASE_POINT_SIZE: f64 = 10.0;
const MAX_EXPORT_DOCUMENTS: usize = 32;
const HELP_BACKGROUND: Rgb = Rgb { red: 255, green: 255, blue: 228 };
const CONTENT_HOST_BACKGROUND: Rgb = Rgb { red: 212, green: 212, blue: 212 };
const WINHELP_INFO_BACKGROUND: Rgb = Rgb { red: 249, green: 249, blue: 158 };

/// Inputs that are meaningful to the HTML shell but do not belong to the HLP parser crate.
pub struct HtmlExportRequest<'a> {
    /// User-opened navigation root. Contents/Index/Search stay anchored here even when the active
    /// topic comes from another HLP.
    pub navigation_document: &'a HelpDocument,
    /// Document currently displayed in the main viewer.
    pub active_document: &'a HelpDocument,
    /// Current topic index in `active_document`.
    pub active_topic_index: usize,
    /// One-hop `:Index`/`:Link` documents already loaded by the native viewer.
    pub catalog_documents: &'a [HelpDocument],
    /// Current topic viewport width. The exported surface itself is fluid, so this is retained as
    /// the authored reference width for objects that carry absolute WinHelp geometry (table
    /// columns in particular) rather than as a fixed width for prose.
    pub layout_width: i32,
    /// Current viewer text zoom. The HTML shell applies this as display zoom after semantic layout
    /// and exposes the same minus/plus controls.
    pub text_zoom_percent: i32,
}

/// Summary returned to the GUI after a successful export.
#[derive(Debug, Clone)]
pub struct HtmlExportReport {
    pub output_path: PathBuf,
    pub document_count: usize,
    pub topic_count: usize,
    pub warning_count: usize,
}

#[derive(Debug, Clone)]
struct ExportDocument {
    document: HelpDocument,
}

#[derive(Default)]
struct ActionRegistry {
    actions: Vec<String>,
}

impl ActionRegistry {
    fn push(&mut self, action_js: String) -> usize {
        let id = self.actions.len();
        self.actions.push(action_js);
        id
    }

    fn push_open(
        &mut self,
        doc: usize,
        topic: usize,
        mode: &'static str,
        window: Option<&WindowDefinition>,
    ) -> usize {
        self.push(open_action_js(doc, topic, mode, window))
    }

    fn push_program(&mut self, program_js: String) -> usize {
        self.push(format!("{{kind:\"program\",ops:{program_js}}}"))
    }

    fn push_noop(&mut self, message: &str) -> usize {
        self.push(format!(
            "{{kind:\"noop\",message:{}}}",
            js_string(message)
        ))
    }
}

/// Returns the deterministic default output path used by the headless command-line exporter.
/// `manual.hlp` becomes `manual.html` in the same directory.
pub fn default_output_path(source_path: &Path) -> PathBuf {
    source_path.with_extension("html")
}

/// Exports a complete, self-contained interactive HTML help viewer.
pub fn export_to_html(
    request: HtmlExportRequest<'_>,
    output_path: &Path,
) -> Result<HtmlExportReport, String> {
    if request.active_topic_index >= request.active_document.presentations().len() {
        return Err(format!(
            "current topic index {} is outside {}",
            request.active_topic_index,
            request.active_document.source_path().display()
        ));
    }

    let (documents, mut warnings) = collect_export_documents(&request)?;
    let path_map = build_path_map(&documents);
    let root_index = document_index_for_path(&path_map, request.navigation_document.source_path())
        .ok_or_else(|| "HTML export lost the navigation-root document".to_owned())?;
    let active_index = document_index_for_path(&path_map, request.active_document.source_path())
        .ok_or_else(|| "HTML export lost the active document".to_owned())?;

    let catalog_keys = request
        .catalog_documents
        .iter()
        .map(|document| path_identity(document.source_path()))
        .chain(std::iter::once(path_identity(
            request.navigation_document.source_path(),
        )))
        .collect::<BTreeSet<_>>();
    let catalog_indices = documents
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            catalog_keys
                .contains(&path_identity(item.document.source_path()))
                .then_some(index)
        })
        .collect::<Vec<_>>();

    let layout_width = request
        .layout_width
        .clamp(MIN_EXPORT_LAYOUT_WIDTH, MAX_EXPORT_LAYOUT_WIDTH);
    let text_zoom_percent = request.text_zoom_percent.clamp(70, 200);
    let mut actions = ActionRegistry::default();
    let root_contents_action = documents[root_index]
        .document
        .contents_topic_index()
        .map(|topic| actions.push_open(root_index, topic, "main", None));

    let mut templates = String::new();
    let mut topic_count = 0_usize;
    for (doc_index, item) in documents.iter().enumerate() {
        for topic_index in 0..item.document.presentations().len() {
            topic_count += 1;
            render_topic_template(
                &mut templates,
                &documents,
                &path_map,
                &mut actions,
                doc_index,
                topic_index,
                layout_width,
                HELP_BACKGROUND,
            )?;
        }
    }

    let contents_js = build_contents_js(
        &documents,
        &path_map,
        &mut actions,
        root_index,
        &mut warnings,
    );
    let all_topics_js = build_all_topics_js(&documents[root_index].document, root_index, &mut actions);
    let index_js = build_index_js(&documents, &catalog_indices, &mut actions);
    let search_js = build_search_js(&documents, &catalog_indices);
    let docs_js = build_documents_js(&documents, &path_map, &mut actions, &mut warnings);

    let start_action = actions.push_open(
        active_index,
        request.active_topic_index,
        "main",
        None,
    );

    let title = request
        .navigation_document
        .system()
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            request
                .navigation_document
                .source_path()
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Windows Help".to_owned())
        });

    let mut html = String::with_capacity(templates.len().saturating_add(128 * 1024));
    html.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    writeln!(html, "<title>{}</title>", html_escape(&title)).map_err(fmt_error)?;
    html.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n");
    html.push_str("<style>\n");
    html.push_str(EXPORT_CSS);
    writeln!(
        html,
        "\n:root{{--host:{};--page:{};--info:{};--topic-width:{}px}}",
        css_rgb(CONTENT_HOST_BACKGROUND),
        css_rgb(HELP_BACKGROUND),
        css_rgb(WINHELP_INFO_BACKGROUND),
        layout_width,
    )
    .map_err(fmt_error)?;
    html.push_str("</style>\n</head>\n<body>\n");
    html.push_str(EXPORT_SHELL);
    html.push_str("\n<div id=\"templates\" hidden>\n");
    html.push_str(&templates);
    html.push_str("</div>\n<script>\n");
    writeln!(html, "const EXPORT_TITLE = {};", js_string(&title)).map_err(fmt_error)?;
    writeln!(html, "const ROOT_DOC = {root_index};").map_err(fmt_error)?;
    writeln!(html, "const START_ACTION = {start_action};").map_err(fmt_error)?;
    writeln!(html, "const INITIAL_ZOOM = {text_zoom_percent};").map_err(fmt_error)?;
    writeln!(
        html,
        "const ROOT_CONTENTS_ACTION = {};",
        root_contents_action.map_or_else(|| "null".to_owned(), |value| value.to_string())
    )
    .map_err(fmt_error)?;
    writeln!(html, "const DOCUMENTS = {docs_js};").map_err(fmt_error)?;
    writeln!(html, "const CONTENTS = {contents_js};").map_err(fmt_error)?;
    writeln!(html, "const ALL_TOPICS = {all_topics_js};").map_err(fmt_error)?;
    writeln!(html, "const INDEX_ROWS = {index_js};").map_err(fmt_error)?;
    writeln!(html, "const SEARCH_TOPICS = {search_js};").map_err(fmt_error)?;
    html.push_str("const ACTIONS = [\n");
    for action in &actions.actions {
        writeln!(html, "{action},").map_err(fmt_error)?;
    }
    html.push_str("];\n");
    html.push_str(EXPORT_JS);
    html.push_str("\n</script>\n</body>\n</html>\n");

    let output_path = normalize_html_extension(output_path);
    fs::write(&output_path, html)
        .map_err(|error| format!("could not write '{}': {error}", output_path.display()))?;

    Ok(HtmlExportReport {
        output_path,
        document_count: documents.len(),
        topic_count,
        warning_count: warnings.len(),
    })
}

fn collect_export_documents(
    request: &HtmlExportRequest<'_>,
) -> Result<(Vec<ExportDocument>, Vec<String>), String> {
    let mut documents = Vec::<ExportDocument>::new();
    let mut seen = BTreeSet::new();
    let mut warnings = Vec::new();

    let mut add_seed = |document: &HelpDocument| {
        let key = path_identity(document.source_path());
        if seen.insert(key) {
            documents.push(ExportDocument {
                document: document.clone(),
            });
        }
    };
    add_seed(request.navigation_document);
    add_seed(request.active_document);
    for document in request.catalog_documents {
        add_seed(document);
    }

    let mut cursor = 0_usize;
    while cursor < documents.len() {
        let current = documents[cursor].document.clone();
        cursor += 1;
        let references = referenced_help_files(&current, request.text_zoom_percent);
        for reference in references {
            if reference.trim().is_empty() || !automatic_export_reference_allowed(&reference) {
                if !reference.trim().is_empty() {
                    warnings.push(format!(
                        "{}: skipped automatic export of non-relative linked help file {reference:?}",
                        current.source_path().display()
                    ));
                }
                continue;
            }
            let path = resolve_external_help_path(current.source_path(), &reference);
            let key = path_identity(&path);
            if seen.contains(&key) {
                continue;
            }
            if documents.len() >= MAX_EXPORT_DOCUMENTS {
                warnings.push(format!(
                    "linked-help export limit ({MAX_EXPORT_DOCUMENTS}) reached; '{}' was not embedded",
                    path.display()
                ));
                continue;
            }
            match HelpDocument::open(&path) {
                Ok(document) => {
                    seen.insert(key);
                    documents.push(ExportDocument { document });
                }
                Err(error) => warnings.push(format!(
                    "could not embed linked help file '{}': {error}",
                    path.display()
                )),
            }
        }
    }

    Ok((documents, warnings))
}

fn referenced_help_files(document: &HelpDocument, _text_zoom_percent: i32) -> Vec<String> {
    let mut result = Vec::new();
    if let Some(contents) = document.contents_file() {
        if let Some(base) = &contents.base {
            push_unique_string(&mut result, &base.help_file);
        }
        for entry in &contents.items {
            if let Some(help_file) = entry.target.as_ref().and_then(|target| target.help_file.as_deref()) {
                push_unique_string(&mut result, help_file);
            }
        }
        for link in contents.index_links.iter().chain(contents.search_links.iter()) {
            push_unique_string(&mut result, &link.help_file);
        }
    }

    for macro_text in &document.system().config_macros {
        collect_macro_help_files(macro_text, &mut result);
    }
    for topic in document.topics() {
        for macro_text in &topic.macros {
            collect_macro_help_files(macro_text, &mut result);
        }
    }

    for presentation in document.presentations() {
        collect_formatted_record_help_files(&presentation.non_scrolling, &mut result);
        collect_formatted_record_help_files(&presentation.scrolling, &mut result);
    }
    result
}

fn collect_formatted_record_help_files(records: &[FormattedRecord], output: &mut Vec<String>) {
    for record in records {
        for paragraph in &record.paragraphs {
            for inline in &paragraph.inlines {
                match inline {
                    Inline::Text(run) => {
                        if let Some(hotspot) = &run.hotspot {
                            collect_hotspot_help_file(hotspot, output);
                        }
                    }
                    Inline::Picture(picture) => collect_picture_help_files(picture, output),
                    Inline::EmbeddedWindow(window) => {
                        if let Some((_label, macro_text)) = window.standard_button_parts() {
                            collect_macro_help_files(macro_text, output);
                        }
                    }
                    Inline::LineBreak | Inline::Tab | Inline::Control85(_) => {}
                }
            }
        }
        for cell in &record.table_cells {
            collect_table_cell_help_files(cell, output);
        }
    }
}

fn collect_table_cell_help_files(cell: &hlp::TableCell, output: &mut Vec<String>) {
    match &cell.content {
        TableCellContent::Display { .. } | TableCellContent::NoRender { .. } | TableCellContent::Unsupported { .. } => {}
        TableCellContent::Picture(picture) => collect_picture_help_files(picture, output),
        TableCellContent::EmbeddedWindow(window) => {
            if let Some((_label, macro_text)) = window.standard_button_parts() {
                collect_macro_help_files(macro_text, output);
            }
        }
        TableCellContent::Table(table) => {
            for nested in &table.cells {
                collect_table_cell_help_files(nested, output);
            }
        }
    }
}

fn collect_picture_help_files(picture: &PictureReference, output: &mut Vec<String>) {
    for hotspot in &picture.hotspots {
        collect_hotspot_help_file(&hotspot.hotspot, output);
    }
}

fn collect_hotspot_help_file(hotspot: &Hotspot, output: &mut Vec<String>) {
    match &hotspot.target {
        HotspotTarget::External { help_file: Some(help_file), .. } => push_unique_string(output, help_file),
        HotspotTarget::Macro(text) => collect_macro_help_files(text, output),
        HotspotTarget::Internal { .. }
        | HotspotTarget::ContextHash { .. }
        | HotspotTarget::External { help_file: None, .. } => {}
    }
}

fn collect_macro_help_files(text: &str, output: &mut Vec<String>) {
    let Ok(program) = HelpMacroProgram::parse(text) else {
        return;
    };
    for macro_ in program.macros {
        let HelpMacro::Allowed(command) = macro_ else {
            continue;
        };
        match command {
            SafeHelpMacro::JumpContents { help_file, .. }
            | SafeHelpMacro::JumpContext { help_file, .. }
            | SafeHelpMacro::JumpHash { help_file, .. }
            | SafeHelpMacro::PopupContext { help_file, .. }
            | SafeHelpMacro::PopupHash { help_file, .. }
            | SafeHelpMacro::PopupId { help_file, .. } => push_unique_string(output, &help_file),
            SafeHelpMacro::JumpId { path_window, .. } => {
                let (help_file, _) = split_macro_path_window(&path_window);
                push_unique_string(output, help_file);
            }
            SafeHelpMacro::ALink { .. }
            | SafeHelpMacro::About
            | SafeHelpMacro::Back
            | SafeHelpMacro::BackFlush
            | SafeHelpMacro::BookmarkDefine
            | SafeHelpMacro::BookmarkMore
            | SafeHelpMacro::BrowseButtons
            | SafeHelpMacro::Contents
            | SafeHelpMacro::Finder
            | SafeHelpMacro::FocusWindow { .. }
            | SafeHelpMacro::History
            | SafeHelpMacro::OpenUrl { .. }
            | SafeHelpMacro::Next
            | SafeHelpMacro::Prev
            | SafeHelpMacro::Search
            | SafeHelpMacro::SetPopupColor { .. } => {}
        }
    }
}

fn build_path_map(documents: &[ExportDocument]) -> BTreeMap<String, usize> {
    documents
        .iter()
        .enumerate()
        .map(|(index, item)| (path_identity(item.document.source_path()), index))
        .collect()
}

fn document_index_for_path(map: &BTreeMap<String, usize>, path: &Path) -> Option<usize> {
    map.get(&path_identity(path)).copied()
}

fn build_documents_js(
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    actions: &mut ActionRegistry,
    warnings: &mut Vec<String>,
) -> String {
    let mut output = String::from("[");
    for (doc_index, item) in documents.iter().enumerate() {
        if doc_index != 0 {
            output.push(',');
        }
        let document = &item.document;
        let name = document
            .source_path()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| document.source_path().to_string_lossy().into_owned());
        let title = document
            .system()
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or(&name);
        let config = macro_program_js(
            document,
            documents,
            path_map,
            actions,
            &document.system().config_macros,
            warnings,
        );
        let _ = write!(
            output,
            "{{id:{doc_index},name:{},title:{},config:{config},topics:[",
            js_string(&name),
            js_string(title)
        );
        for topic_index in 0..document.presentations().len() {
            if topic_index != 0 {
                output.push(',');
            }
            let presentation = &document.presentations()[topic_index];
            let topic_macros = document
                .topics()
                .get(topic_index)
                .map(|topic| topic.macros.as_slice())
                .unwrap_or(&[]);
            let macros = macro_program_js(
                document,
                documents,
                path_map,
                actions,
                topic_macros,
                warnings,
            );
            let browse_prev = document
                .browse_previous_index(topic_index)
                .map_or_else(|| "null".to_owned(), |value| value.to_string());
            let browse_next = document
                .browse_next_index(topic_index)
                .map_or_else(|| "null".to_owned(), |value| value.to_string());
            let _ = write!(
                output,
                "{{title:{},template:{},browsePrev:{browse_prev},browseNext:{browse_next},macros:{macros}}}",
                js_string(&topic_label(&presentation.title, topic_index)),
                js_string(&template_id(doc_index, topic_index)),
            );
        }
        output.push_str("]}");
    }
    output.push(']');
    output
}

fn build_contents_js(
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    actions: &mut ActionRegistry,
    root_index: usize,
    warnings: &mut Vec<String>,
) -> String {
    let root = &documents[root_index].document;
    let Some(contents) = root.contents_file() else {
        return "[]".to_owned();
    };
    let mut output = String::from("[");
    for (index, entry) in contents.items.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        let action = entry.target.as_ref().and_then(|target| {
            let help_file = target.help_file.as_deref().or_else(|| {
                contents.base.as_ref().map(|base| base.help_file.as_str())
            });
            let target_doc = resolve_export_document_index(root, help_file, path_map)?;
            let document = &documents[target_doc].document;
            let topic = document.topic_index_for_reference(&target.context)?;
            Some(actions.push_open(target_doc, topic, navigation_mode(document, topic, target.window_name.as_deref()), explicit_or_default_window(document, topic, target.window_name.as_deref())))
        });
        if entry.target.is_some() && action.is_none() {
            warnings.push(format!(
                "{}: unresolved exported Contents target {:?}",
                root.source_path().display(),
                entry.title
            ));
        }
        let action_js = action.map_or_else(|| "null".to_owned(), |value| value.to_string());
        let _ = write!(
            output,
            "{{level:{},title:{},action:{action_js}}}",
            entry.level,
            js_string(&entry.title)
        );
    }
    output.push(']');
    output
}

fn build_all_topics_js(document: &HelpDocument, doc_index: usize, actions: &mut ActionRegistry) -> String {
    let mut output = String::from("[");
    for (topic_index, topic) in document.presentations().iter().enumerate() {
        if topic_index != 0 {
            output.push(',');
        }
        let action = actions.push_open(doc_index, topic_index, "main", None);
        let _ = write!(
            output,
            "{{title:{},action:{action}}}",
            js_string(&topic_label(&topic.title, topic_index))
        );
    }
    output.push(']');
    output
}

fn build_index_js(
    documents: &[ExportDocument],
    catalog_indices: &[usize],
    actions: &mut ActionRegistry,
) -> String {
    // Merge case-insensitively, but deduplicate by destination rather than action id. Registering a
    // fresh action before checking for duplicates can never detect the same keyword target twice.
    let mut merged: BTreeMap<String, (String, BTreeSet<(usize, usize)>)> = BTreeMap::new();
    for &doc_index in catalog_indices {
        let document = &documents[doc_index].document;
        for keyword in document.resolved_keywords() {
            let key = keyword.keyword.to_lowercase();
            let row = merged
                .entry(key)
                .or_insert_with(|| (keyword.keyword.clone(), BTreeSet::new()));
            for &topic_index in &keyword.topic_indices {
                row.1.insert((doc_index, topic_index));
            }
        }
    }

    let mut output = String::from("[");
    for (index, (_, (keyword, destinations))) in merged.into_iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        let _ = write!(output, "{{keyword:{},actions:[", js_string(&keyword));
        for (action_index, (doc_index, topic_index)) in destinations.into_iter().enumerate() {
            if action_index != 0 {
                output.push(',');
            }
            let action = actions.push_open(doc_index, topic_index, "main", None);
            let _ = write!(output, "{action}");
        }
        output.push_str("]}");
    }
    output.push(']');
    output
}

fn build_search_js(documents: &[ExportDocument], catalog_indices: &[usize]) -> String {
    let mut output = String::from("[");
    let mut first = true;
    for &doc_index in catalog_indices {
        let document = &documents[doc_index].document;
        for (topic_index, topic) in document.topics().iter().enumerate() {
            if !first {
                output.push(',');
            }
            first = false;
            let title = document
                .presentations()
                .get(topic_index)
                .map(|presentation| topic_label(&presentation.title, topic_index))
                .unwrap_or_else(|| format!("Topic {}", topic_index + 1));
            let _ = write!(
                output,
                "{{doc:{doc_index},topic:{topic_index},title:{},text:{}}}",
                js_string(&title),
                js_string(&topic.plain_text)
            );
        }
    }
    output.push(']');
    output
}

/// Sequential state carried while one topic region is translated into HTML block flow.
#[derive(Debug)]
struct SemanticFlow {
    /// True until the first paragraph with visible content has been emitted, which is how the
    /// decoded topic title is recognised.
    first_visible_paragraph: bool,
    /// Vertical space in CSS pixels that the previous paragraph's space-below still owes to the
    /// next paragraph. WinHelp adds space-below and space-above, but adjacent CSS margins collapse
    /// to the larger of the two, so the exporter carries the value forward instead.
    pending_space_above: i32,
}

impl SemanticFlow {
    const fn new() -> Self {
        Self { first_visible_paragraph: true, pending_space_above: 0 }
    }

    /// Consumes the space owed to the next paragraph and records what this paragraph owes.
    fn advance(&mut self, spacing_above: i32, spacing_below: i32) -> i32 {
        let margin_top = spacing_above.saturating_add(self.pending_space_above);
        self.pending_space_above = spacing_below;
        margin_top
    }
}

fn render_topic_template(
    output: &mut String,
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    actions: &mut ActionRegistry,
    doc_index: usize,
    topic_index: usize,
    layout_width: i32,
    default_background: Rgb,
) -> Result<(), String> {
    let document = &documents[doc_index].document;
    let presentation = document
        .presentations()
        .get(topic_index)
        .ok_or_else(|| format!("missing decoded presentation for topic {topic_index}"))?;
    // The authored width is retained only as a reference metric for objects that carry absolute
    // WinHelp geometry. The surface itself is fluid so a resized window, a resized navigation
    // pane, or a changed text zoom re-wraps the topic inside the visible page instead of clipping
    // it against a frozen export-time width.
    writeln!(
        output,
        "<template id=\"{}\"><div class=\"topic-view semantic-topic\" data-doc=\"{doc_index}\" data-topic=\"{topic_index}\" style=\"--hlp-authored-width:{layout_width}px\">",
        template_id(doc_index, topic_index)
    )
    .map_err(fmt_error)?;
    let mut flow = SemanticFlow::new();
    render_semantic_region(
        output,
        documents,
        path_map,
        actions,
        document,
        doc_index,
        &presentation.non_scrolling,
        "fixed-region",
        layout_width,
        default_background,
        &presentation.title,
        &mut flow,
    )?;
    render_semantic_region(
        output,
        documents,
        path_map,
        actions,
        document,
        doc_index,
        &presentation.scrolling,
        "scrolling-region",
        layout_width,
        default_background,
        &presentation.title,
        &mut flow,
    )?;
    output.push_str("</div></template>\n");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_semantic_region(
    output: &mut String,
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    actions: &mut ActionRegistry,
    document: &HelpDocument,
    doc_index: usize,
    records: &[FormattedRecord],
    class_name: &str,
    width: i32,
    default_background: Rgb,
    topic_title: &str,
    flow: &mut SemanticFlow,
) -> Result<(), String> {
    // A region is a separate block of flow, so no paragraph spacing is owed across its boundary.
    flow.pending_space_above = 0;
    if records.is_empty() {
        writeln!(
            output,
            "<div class=\"topic-region semantic-region semantic-region-empty {class_name}\" style=\"background:{}\"></div>",
            css_rgb(default_background),
        )
        .map_err(fmt_error)?;
        return Ok(());
    }
    writeln!(
        output,
        "<div class=\"topic-region semantic-region {class_name}\" style=\"background:{}\"><div class=\"semantic-region-inner\" style=\"{}\">",
        css_rgb(default_background),
        style_attribute(&semantic_region_base_css(document.fonts())),
    )
    .map_err(fmt_error)?;
    // Objects that carry absolute WinHelp geometry (tables in particular) still resolve against
    // the authored content width; ordinary prose is laid out by the browser at the real width.
    let available_width = width.saturating_sub(24).max(1);
    for record in records {
        render_semantic_record(
            output,
            documents,
            path_map,
            actions,
            document,
            doc_index,
            record,
            available_width,
            default_background,
            topic_title,
            flow,
        )?;
    }
    output.push_str("</div></div>\n");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_semantic_record(
    output: &mut String,
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    actions: &mut ActionRegistry,
    document: &HelpDocument,
    doc_index: usize,
    record: &FormattedRecord,
    available_width: i32,
    default_background: Rgb,
    topic_title: &str,
    flow: &mut SemanticFlow,
) -> Result<(), String> {
    if let Some(table) = &record.table {
        if !record.table_cells.is_empty() {
            return render_semantic_table(
                output,
                documents,
                path_map,
                actions,
                document,
                doc_index,
                table,
                &record.table_cells,
                &record.paragraphs,
                available_width,
                default_background,
                topic_title,
                flow,
            );
        }
    }
    for paragraph in &record.paragraphs {
        render_semantic_paragraph(
            output,
            documents,
            path_map,
            actions,
            document,
            doc_index,
            paragraph,
            default_background,
            topic_title,
            flow,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_semantic_table(
    output: &mut String,
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    actions: &mut ActionRegistry,
    document: &HelpDocument,
    doc_index: usize,
    table: &TableInfo,
    cells: &[hlp::TableCell],
    paragraphs: &[Paragraph],
    available_width: i32,
    default_background: Rgb,
    topic_title: &str,
    flow: &mut SemanticFlow,
) -> Result<(), String> {
    let columns = semantic_table_column_geometry(table, available_width);
    if columns.is_empty() {
        for paragraph in paragraphs {
            render_semantic_paragraph(
                output, documents, path_map, actions, document, doc_index, paragraph,
                default_background, topic_title, flow,
            )?;
        }
        return Ok(());
    }
    // Space owed by the paragraph before the table belongs to the table itself; each cell then
    // starts a fresh vertical flow.
    let table_margin = flow.advance(0, 0);
    write!(output, "<div class=\"hlp-table\" style=\"margin-top:{table_margin}px\">").map_err(fmt_error)?;
    for (column_index, (gap, width)) in columns.iter().copied().enumerate() {
        write!(
            output,
            "<div class=\"hlp-table-column\" style=\"margin-left:{gap}px;width:{}px\">",
            width.max(1),
        )
        .map_err(fmt_error)?;
        for cell in cells.iter().filter(|cell| cell.column.max(0) as usize == column_index) {
            output.push_str("<div class=\"hlp-table-cell\">");
            flow.pending_space_above = 0;
            match &cell.content {
                TableCellContent::Display { paragraph_start, paragraph_end } => {
                    let start = (*paragraph_start).min(paragraphs.len());
                    let end = (*paragraph_end).min(paragraphs.len()).max(start);
                    for paragraph in &paragraphs[start..end] {
                        if semantic_table_cell_paragraph_is_empty_filler(paragraph) {
                            continue;
                        }
                        render_semantic_paragraph(
                            output, documents, path_map, actions, document, doc_index, paragraph,
                            default_background, topic_title, flow,
                        )?;
                    }
                }
                TableCellContent::Picture(picture) => {
                    render_semantic_picture(
                        output, documents, path_map, actions, document, doc_index, picture,
                    )?;
                }
                TableCellContent::Table(nested) => {
                    render_semantic_table(
                        output, documents, path_map, actions, document, doc_index,
                        &nested.info, &nested.cells, paragraphs, width.max(1), default_background,
                        topic_title, flow,
                    )?;
                }
                TableCellContent::EmbeddedWindow(window) => {
                    render_semantic_embedded_window(
                        output, documents, path_map, actions, document, doc_index, window,
                    )?;
                }
                TableCellContent::NoRender { .. } => {}
                TableCellContent::Unsupported { record_type, .. } => {
                    write!(
                        output,
                        "<span class=\"placeholder semantic-placeholder\">[unsupported {record_type:?}]</span>",
                    )
                    .map_err(fmt_error)?;
                }
            }
            output.push_str("</div>");
        }
        output.push_str("</div>");
    }
    output.push_str("</div>");
    Ok(())
}

/// Reports whether the paragraph's first visible object is a hosted WinHelp control.
///
/// WinHelp's negative (hanging) first-line indent is authored for a *text* marker: the marker sits
/// in the hanging area and the prose that follows lines up with the paragraph's left indent. A
/// hosted control such as the Related Topics / `ALink` button is instead pulled bodily into the
/// margin by that indent, where its box no longer lines up with the rules and text of the
/// paragraphs around it. Leading pictures are deliberately excluded: authored bullet and list
/// bitmaps are real hanging markers and must keep hanging.
fn semantic_control_leads_paragraph(paragraph: &Paragraph) -> bool {
    paragraph
        .inlines
        .iter()
        .find(|inline| !matches!(inline, Inline::Control85(_)))
        .is_some_and(|inline| matches!(inline, Inline::EmbeddedWindow(_)))
}

#[allow(clippy::too_many_arguments)]
fn render_semantic_paragraph(
    output: &mut String,
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    actions: &mut ActionRegistry,
    document: &HelpDocument,
    doc_index: usize,
    paragraph: &Paragraph,
    default_background: Rgb,
    topic_title: &str,
    flow: &mut SemanticFlow,
) -> Result<(), String> {
    let format = &paragraph.format;
    let visible_text = semantic_paragraph_text(paragraph);
    let is_visible = !visible_text.trim().is_empty()
        || paragraph.inlines.iter().any(|inline| matches!(inline, Inline::Picture(_) | Inline::EmbeddedWindow(_)));
    let normalized_title = topic_title.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized_text = visible_text.split_whitespace().collect::<Vec<_>>().join(" ");
    let topic_heading = is_visible
        && flow.first_visible_paragraph
        && !normalized_title.is_empty()
        && normalized_text.eq_ignore_ascii_case(&normalized_title);
    if is_visible {
        flow.first_visible_paragraph = false;
    }

    let border = format.border;
    let border_clearance = border.map_or(0, |info| semantic_border_clearance(info.style));
    let flags = border.map(|info| info.flags).unwrap_or_default();
    let left_indent = semantic_paragraph_metric(format.left_indent);
    let right_indent = semantic_paragraph_metric(format.right_indent);
    let first_indent = semantic_paragraph_metric(format.first_line_indent);
    let first_indent = if semantic_control_leads_paragraph(paragraph) {
        first_indent.max(0)
    } else {
        first_indent
    };
    let spacing_above = semantic_vertical_metric(format.spacing_above);
    let spacing_below = semantic_vertical_metric(format.spacing_below);
    // The bottom margin is deliberately zero: `SemanticFlow` carries this paragraph's space-below
    // into the next paragraph's top margin so the two authored distances add up the way WinHelp
    // adds them, instead of collapsing to the larger of the two the way sibling CSS margins do.
    let margin_top = flow.advance(spacing_above, spacing_below);
    let mut css = format!(
        "margin:{margin_top}px {right_indent}px 0 {left_indent}px;text-align:{};direction:{};",
        semantic_alignment(format.alignment),
        if format.right_to_left { "rtl" } else { "ltr" },
    );
    if format.no_wrap {
        css.push_str("white-space:nowrap;");
    } else {
        css.push_str("white-space:normal;overflow-wrap:normal;word-break:normal;hyphens:none;");
    }
    if let Some(lines) = format.spacing_lines {
        let pixels = semantic_vertical_metric(Some(lines));
        if pixels < 0 {
            let _ = write!(css, "line-height:{}px;", pixels.saturating_abs().max(1));
        } else if pixels > 0 {
            let _ = write!(css, "line-height:max(1em,{}px);", pixels.max(1));
        }
    }
    if flags.box_all || flags.left {
        let _ = write!(css, "padding-left:{border_clearance}px;");
    }
    if flags.box_all || flags.right {
        let _ = write!(css, "padding-right:{border_clearance}px;");
    }
    if flags.box_all || flags.top {
        let _ = write!(css, "padding-top:{border_clearance}px;");
    }
    if flags.box_all || flags.bottom {
        let _ = write!(css, "padding-bottom:{border_clearance}px;");
    }
    if let Some(border) = border {
        css.push_str(&semantic_border_css(border));
    }

    let tab_count = paragraph.inlines.iter().filter(|inline| matches!(inline, Inline::Tab)).count();
    if tab_count == 0 && first_indent != 0 {
        let _ = write!(css, "text-indent:{first_indent}px;");
    }
    let class = if topic_heading {
        "hlp-paragraph topic-heading"
    } else if is_visible {
        "hlp-paragraph"
    } else {
        // An authored paragraph with no visible content is a blank line in WinHelp. An empty HTML
        // paragraph has no line box at all, so it would silently disappear and pull the surrounding
        // text together; the class gives it back exactly one line of the topic's own font.
        "hlp-paragraph hlp-blank-paragraph"
    };
    let css = style_attribute(&css);
    if !is_visible {
        write!(output, "<p class=\"{class}\" style=\"{css}\"></p>").map_err(fmt_error)?;
        return Ok(());
    }
    if tab_count == 0 {
        write!(output, "<p class=\"{class}\" style=\"{css}\">").map_err(fmt_error)?;
        render_semantic_inline_sequence(
            output, documents, path_map, actions, document, doc_index, &paragraph.inlines,
            default_background,
        )?;
        output.push_str("</p>");
        return Ok(());
    }

    let segments = semantic_tab_segments(&paragraph.inlines);
    let tab_targets = semantic_tab_targets(format, segments.len().saturating_sub(1));
    write!(
        output,
        "<p class=\"{class} hlp-tabbed-paragraph\" style=\"{css}\"><span class=\"hlp-tab-grid\" style=\"{}\">",
        semantic_tab_grid_css(&tab_targets),
    )
    .map_err(fmt_error)?;
    for (index, segment) in segments.iter().enumerate() {
        let alignment = if index == 0 {
            "left"
        } else {
            semantic_tab_alignment(tab_targets.get(index - 1).map(|value| value.1).unwrap_or(TabAlignment::Left))
        };
        let first_margin = if index == 0 { first_indent } else { 0 };
        write!(
            output,
            "<span class=\"hlp-tab-segment\" style=\"text-align:{alignment};margin-left:{first_margin}px\">",
        )
        .map_err(fmt_error)?;
        render_semantic_inline_sequence(
            output, documents, path_map, actions, document, doc_index, segment, default_background,
        )?;
        output.push_str("</span>");
    }
    output.push_str("</span></p>");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_semantic_inline_sequence(
    output: &mut String,
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    actions: &mut ActionRegistry,
    document: &HelpDocument,
    doc_index: usize,
    inlines: &[Inline],
    default_background: Rgb,
) -> Result<(), String> {
    for inline in inlines {
        match inline {
            Inline::Text(run) => {
                render_semantic_text_run(
                    output, documents, path_map, actions, document, doc_index, run, default_background,
                )?;
            }
            Inline::LineBreak => output.push_str("<br class=\"hlp-hard-break\">"),
            Inline::Tab => {}
            Inline::Control85(origin) => {
                let x = semantic_raw_metric(i32::from(*origin));
                write!(
                    output,
                    "<span class=\"hlp-origin-reset\" aria-hidden=\"true\" style=\"--hlp-origin:{x}px\"></span>",
                )
                .map_err(fmt_error)?;
            }
            Inline::Picture(picture) => {
                render_semantic_picture(output, documents, path_map, actions, document, doc_index, picture)?;
            }
            Inline::EmbeddedWindow(window) => {
                render_semantic_embedded_window(output, documents, path_map, actions, document, doc_index, window)?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_semantic_text_run(
    output: &mut String,
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    actions: &mut ActionRegistry,
    document: &HelpDocument,
    doc_index: usize,
    run: &TextRun,
    default_background: Rgb,
) -> Result<(), String> {
    let style = semantic_text_style(run, document.fonts());
    let foreground = if style.foreground_inherits {
        Rgb { red: 0, green: 0, blue: 0 }
    } else {
        style.foreground
    };
    let background = if style.background_inherits { default_background } else { style.background };
    let mut style_css = semantic_text_style_css(&style, &run.text);
    let _ = write!(style_css, "color:{};", css_rgb(foreground));
    if background != default_background {
        let _ = write!(style_css, "background:{};", css_rgb(background));
    }
    let style_css = style_attribute(&style_css);
    if let Some(hotspot) = &run.hotspot {
        let action = register_hotspot_action(
            documents, path_map, actions, document, doc_index, hotspot,
        );
        let title_attr = exported_hotspot_tooltip(documents, path_map, document, hotspot)
            .map(|title| format!(" title=\"{}\"", html_escape(&title)))
            .unwrap_or_default();
        write!(
            output,
            "<a class=\"hlp-run hlp-link interactive\" href=\"#\" data-action=\"{action}\"{title_attr} style=\"{style_css}\">{}</a>",
            html_escape(&run.text),
        )
        .map_err(fmt_error)?;
    } else {
        write!(
            output,
            "<span class=\"hlp-run\" style=\"{style_css}\">{}</span>",
            html_escape(&run.text),
        )
        .map_err(fmt_error)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_semantic_picture(
    output: &mut String,
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    actions: &mut ActionRegistry,
    document: &HelpDocument,
    doc_index: usize,
    picture: &PictureReference,
) -> Result<(), String> {
    let class = match picture.position {
        PicturePosition::Inline => "hlp-picture hlp-picture-inline",
        PicturePosition::FloatLeft => "hlp-picture hlp-picture-left",
        PicturePosition::FloatRight => "hlp-picture hlp-picture-right",
    };
    let Some(image) = &picture.image else {
        output.push_str("<span class=\"placeholder semantic-placeholder\">[embedded picture]</span>");
        return Ok(());
    };
    let rgba = base64_encode(image.rgba.as_ref());
    write!(
        output,
        "<span class=\"{class}\" style=\"width:{}px;height:{}px\"><canvas class=\"picture\" width=\"{}\" height=\"{}\" data-rgba=\"{rgba}\"></canvas>",
        image.width, image.height, image.width, image.height,
    )
    .map_err(fmt_error)?;
    for hotspot in &picture.hotspots {
        let action = register_hotspot_action(
            documents, path_map, actions, document, doc_index, &hotspot.hotspot,
        );
        let title_attr = exported_hotspot_tooltip(documents, path_map, document, &hotspot.hotspot)
            .map(|title| format!(" title=\"{}\"", html_escape(&title)))
            .unwrap_or_default();
        write!(
            output,
            "<a href=\"#\" class=\"picture-hotspot interactive\" data-action=\"{action}\"{title_attr} style=\"left:{}px;top:{}px;width:{}px;height:{}px\"></a>",
            hotspot.x, hotspot.y, hotspot.width, hotspot.height,
        )
        .map_err(fmt_error)?;
    }
    output.push_str("</span>");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_semantic_embedded_window(
    output: &mut String,
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    actions: &mut ActionRegistry,
    document: &HelpDocument,
    doc_index: usize,
    window: &EmbeddedWindowReference,
) -> Result<(), String> {
    if let Some((label, macro_text)) = window.standard_button_parts() {
        let macro_text = macro_text.trim();
        let hotspot = (!macro_text.is_empty()).then(|| Hotspot {
            target: HotspotTarget::Macro(macro_text.to_owned()),
            emphasized: false,
        });
        let size_class = if label.is_empty() { " empty-label" } else { "" };
        if let Some(hotspot) = hotspot {
            let action = register_hotspot_action(
                documents, path_map, actions, document, doc_index, &hotspot,
            );
            let title_attr = exported_hotspot_tooltip(documents, path_map, document, &hotspot)
                .map(|title| format!(" title=\"{}\"", html_escape(&title)))
                .unwrap_or_default();
            write!(
                output,
                "<button type=\"button\" class=\"semantic-control interactive{size_class}\" data-action=\"{action}\"{title_attr}>{}</button>",
                html_escape(label),
            )
            .map_err(fmt_error)?;
        } else {
            write!(
                output,
                "<button type=\"button\" class=\"semantic-control{size_class}\" disabled>{}</button>",
                html_escape(label),
            )
            .map_err(fmt_error)?;
        }
    } else {
        write!(
            output,
            "<span class=\"placeholder semantic-placeholder semantic-hosted\">{}</span>",
            html_escape(&window.descriptor),
        )
        .map_err(fmt_error)?;
    }
    Ok(())
}

fn semantic_text_style(run: &TextRun, fonts: &FontTable) -> ResolvedTextStyle {
    let descriptor = fonts
        .descriptor(run.font_index)
        .or_else(|| fonts.descriptors().first());
    descriptor.map_or_else(
        || ResolvedTextStyle {
            face_name: "MS Sans Serif".to_owned(),
            family: ResolvedFontFamily::Proportional,
            source_family: hlp::HlpFontFamily::Swiss,
            point_size: 10,
            point_size_twips: 200,
            weight: 400,
            italic: false,
            underline: run.hotspot.is_some(),
            strike_out: false,
            small_caps: false,
            foreground: if run.hotspot.is_some() { Rgb { red: 0, green: 128, blue: 0 } } else { Rgb { red: 0, green: 0, blue: 0 } },
            foreground_inherits: run.hotspot.is_none(),
            background: Rgb { red: 255, green: 255, blue: 255 },
            background_inherits: true,
            charset: None,
        },
        |font| {
            semantic_style_from_font_with_background_inheritance(
                font,
                run.hotspot.as_ref(),
                fonts.background_inherits(run.font_index),
            )
        },
    )
}

fn semantic_style_from_font(font: &FontDescriptor, hotspot: Option<&Hotspot>) -> ResolvedTextStyle {
    let inherited_background = font.background == (Rgb { red: 1, green: 1, blue: 0 });
    semantic_style_from_font_with_background_inheritance(font, hotspot, inherited_background)
}

fn semantic_style_from_font_with_background_inheritance(
    font: &FontDescriptor,
    hotspot: Option<&Hotspot>,
    inherited_background: bool,
) -> ResolvedTextStyle {
    let emphasized = hotspot.is_some();
    let inherited = font.foreground == (Rgb { red: 1, green: 1, blue: 0 });
    ResolvedTextStyle {
        face_name: font.face_name.clone(),
        family: if font.is_fixed_pitch() { ResolvedFontFamily::Monospace } else { ResolvedFontFamily::Proportional },
        source_family: font.family,
        point_size: font.point_size(),
        point_size_twips: font.point_size_twips,
        weight: font.weight,
        italic: font.italic,
        underline: font.underline || emphasized,
        strike_out: font.strike_out,
        small_caps: font.small_caps,
        foreground: if emphasized { Rgb { red: 0, green: 128, blue: 0 } } else { font.foreground },
        foreground_inherits: !emphasized && inherited,
        background: font.background,
        background_inherits: inherited_background,
        charset: font.charset,
    }
}

fn semantic_paragraph_text(paragraph: &Paragraph) -> String {
    let mut text = String::new();
    for inline in &paragraph.inlines {
        match inline {
            Inline::Text(run) => text.push_str(&run.text),
            Inline::LineBreak => text.push(' '),
            Inline::Tab => text.push(' '),
            Inline::Control85(_) | Inline::Picture(_) | Inline::EmbeddedWindow(_) => {}
        }
    }
    text
}

fn semantic_tab_segments(inlines: &[Inline]) -> Vec<&[Inline]> {
    let mut result = Vec::new();
    let mut start = 0usize;
    for (index, inline) in inlines.iter().enumerate() {
        if matches!(inline, Inline::Tab) {
            result.push(&inlines[start..index]);
            start = index.saturating_add(1);
        }
    }
    result.push(&inlines[start..]);
    result
}

fn semantic_tab_targets(format: &ParagraphFormat, count: usize) -> Vec<(i32, TabAlignment)> {
    let mut result = Vec::with_capacity(count);
    let mut current = 0i32;
    let default = semantic_raw_metric(i32::from(format.default_tab_interval.unwrap_or(72)))
        .abs()
        .max(1);
    for _ in 0..count {
        // WinHlp32 does not consume custom stops by ordinal. For each Tab command it selects the
        // first authored stop strictly to the right of the current x position; only after those
        // stops are exhausted does it advance to the next default-tab multiple.
        let explicit = format.tabs.iter().find_map(|stop| {
            let position = semantic_raw_metric(i32::from(stop.position));
            (position > current).then_some((position, stop.alignment))
        });
        let (position, alignment) = explicit.unwrap_or_else(|| {
            let next = ((current / default) + 1).saturating_mul(default);
            (next, TabAlignment::Left)
        });
        result.push((position, alignment));
        current = position;
    }
    result
}

fn semantic_tab_grid_css(targets: &[(i32, TabAlignment)]) -> String {
    if targets.is_empty() {
        return "grid-template-columns:1fr".to_owned();
    }
    let mut css = String::from("grid-template-columns:");
    let mut previous = 0i32;
    for (position, _) in targets {
        let width = position.saturating_sub(previous).max(1);
        let _ = write!(css, "{width}px ");
        previous = *position;
    }
    css.push_str("minmax(0,1fr)");
    css
}

fn semantic_tab_alignment(alignment: TabAlignment) -> &'static str {
    match alignment {
        TabAlignment::Right => "right",
        TabAlignment::Center => "center",
        TabAlignment::Left | TabAlignment::Unknown(_) => "left",
    }
}

fn semantic_alignment(alignment: ParagraphAlignment) -> &'static str {
    match alignment {
        ParagraphAlignment::Left => "left",
        ParagraphAlignment::Right => "right",
        ParagraphAlignment::Center => "center",
    }
}

fn semantic_paragraph_metric(value: Option<i16>) -> i32 {
    value.map_or(0, |raw| semantic_raw_metric(i32::from(raw)))
}

fn semantic_vertical_metric(value: Option<i16>) -> i32 {
    value.map_or(0, |raw| semantic_raw_metric(i32::from(raw)))
}

fn semantic_raw_metric(raw: i32) -> i32 {
    let scaled = i64::from(raw).saturating_mul(i64::from(EXPORT_DPI));
    i32::try_from(scaled / 144).unwrap_or(if raw < 0 { i32::MIN } else { i32::MAX })
}

fn semantic_border_clearance(style: BorderStyle) -> i32 {
    match style {
        BorderStyle::Normal | BorderStyle::ReferenceStyle4 => 5,
        BorderStyle::Thick | BorderStyle::Shadow => 6,
        BorderStyle::Double => 7,
        BorderStyle::Reserved(_) => 0,
    }
}

fn semantic_border_css(border: BorderInfo) -> String {
    let flags = border.flags;
    if matches!(border.style, BorderStyle::Reserved(_)) {
        return String::new();
    }
    let width = if matches!(border.style, BorderStyle::Thick) { 2 } else { 1 };
    let border_kind = if matches!(border.style, BorderStyle::Double) { "double" } else { "solid" };
    let mut css = String::new();
    if flags.box_all || flags.top { let _ = write!(css, "border-top:{width}px {border_kind} #000;"); }
    if flags.box_all || flags.left { let _ = write!(css, "border-left:{width}px {border_kind} #000;"); }
    if flags.box_all || flags.bottom { let _ = write!(css, "border-bottom:{width}px {border_kind} #000;"); }
    if flags.box_all || flags.right { let _ = write!(css, "border-right:{width}px {border_kind} #000;"); }
    if matches!(border.style, BorderStyle::Shadow) { css.push_str("box-shadow:1px 1px 0 #000;"); }
    css
}

fn semantic_table_column_geometry(table: &TableInfo, available_width: i32) -> Vec<(i32, i32)> {
    if table.columns.is_empty() {
        return Vec::new();
    }
    let absolute = |raw: u16| -> i32 {
        let scaled = i64::from(raw).saturating_mul(i64::from(EXPORT_DPI));
        i32::try_from(scaled / 144).unwrap_or(i32::MAX)
    };
    let effective_width = if table.table_type == 0 {
        table.minimum_width.map_or(0, absolute).max(available_width)
    } else {
        available_width
    };
    let reference_width = absolute(32_767).max(1);
    table
        .columns
        .iter()
        .map(|column| {
            let convert = |raw: u16| {
                if table.table_type == 0 {
                    let physical = absolute(raw);
                    i32::try_from(i64::from(physical).saturating_mul(i64::from(effective_width.max(0))) / i64::from(reference_width)).unwrap_or(i32::MAX)
                } else {
                    absolute(raw)
                }
            };
            (convert(column.gap_before), convert(column.width).max(1))
        })
        .collect()
}

fn semantic_table_cell_paragraph_is_empty_filler(paragraph: &Paragraph) -> bool {
    paragraph.format.border.is_none()
        && paragraph.inlines.iter().all(|inline| matches!(inline, Inline::Control85(_)))
}

#[allow(clippy::too_many_arguments)]
fn render_region(
    output: &mut String,
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    actions: &mut ActionRegistry,
    document: &HelpDocument,
    doc_index: usize,
    region: &RegionLayout,
    class_name: &str,
    default_background: Rgb,
) -> Result<(), String> {
    writeln!(
        output,
        "<div class=\"topic-region {class_name}\" data-retained-height=\"{}\" style=\"width:{}px;height:{}px;background:{}\">",
        region.height.max(0),
        region.width.max(1),
        region.height.max(0),
        css_rgb(default_background)
    )
    .map_err(fmt_error)?;
    for item in &region.boxes {
        render_layout_box(
            output,
            documents,
            path_map,
            actions,
            document,
            doc_index,
            item,
            default_background,
        )?;
    }
    output.push_str("</div>\n");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_layout_box(
    output: &mut String,
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    actions: &mut ActionRegistry,
    document: &HelpDocument,
    doc_index: usize,
    item: &LayoutBox,
    default_background: Rgb,
) -> Result<(), String> {
    let geometry = geometry_css(item);
    match &item.kind {
        LayoutKind::Text { text, style, hotspot, baseline, flow } => {
            let action = hotspot.as_ref().map(|hotspot| {
                register_hotspot_action(
                    documents,
                    path_map,
                    actions,
                    document,
                    doc_index,
                    hotspot,
                )
            });
            let action_attr = action
                .map(|id| format!(" data-action=\"{id}\" tabindex=\"0\" role=\"link\""))
                .unwrap_or_default();
            let title_attr = hotspot
                .as_ref()
                .and_then(|hotspot| {
                    exported_hotspot_tooltip(
                        documents,
                        path_map,
                        document,
                        hotspot,
                    )
                })
                .map(|title| format!(" title=\"{}\"", html_escape(&title)))
                .unwrap_or_default();
            let class = if action.is_some() { "box text-box interactive" } else { "box text-box" };
            let foreground = if style.foreground_inherits {
                Rgb { red: 0, green: 0, blue: 0 }
            } else {
                style.foreground
            };
            let background = if style.background_inherits {
                default_background
            } else {
                style.background
            };
            let background_css = if background == default_background {
                "transparent".to_owned()
            } else {
                css_rgb(background)
            };
            writeln!(
                output,
                "<span class=\"{class}\"{action_attr}{title_attr} data-baseline=\"{}\" data-paragraph=\"{}\" data-flow-line=\"{}\" data-hard-break=\"{}\" data-segment=\"{}\" data-no-wrap=\"{}\" data-reflow-safe=\"{}\" data-content-left=\"{}\" data-content-right=\"{}\" style=\"{geometry};{};color:{};background:{}\"><span class=\"text-glyphs\">{}</span></span>",
                item.bounds.y.saturating_add(*baseline),
                flow.paragraph_id,
                flow.line_index,
                u8::from(flow.hard_break_before),
                flow.segment_index,
                u8::from(flow.no_wrap),
                u8::from(flow.reflow_safe),
                flow.content_left,
                flow.content_right,
                text_style_css(style),
                css_rgb(foreground),
                background_css,
                html_escape(text)
            )
            .map_err(fmt_error)?;
        }
        LayoutKind::Picture { image } => {
            let rgba = base64_encode(image.rgba.as_ref());
            writeln!(
                output,
                "<canvas class=\"box picture\" style=\"{geometry}\" width=\"{}\" height=\"{}\" data-rgba=\"{rgba}\"></canvas>",
                image.width,
                image.height
            )
            .map_err(fmt_error)?;
        }
        LayoutKind::PictureHotspot { hotspot } => {
            let action = register_hotspot_action(
                documents,
                path_map,
                actions,
                document,
                doc_index,
                hotspot,
            );
            let title_attr = exported_hotspot_tooltip(
                documents,
                path_map,
                document,
                hotspot,
            )
            .map(|title| format!(" title=\"{}\"", html_escape(&title)))
            .unwrap_or_default();
            writeln!(
                output,
                "<span class=\"box picture-hotspot interactive\" data-action=\"{action}\" tabindex=\"0\" role=\"link\"{title_attr} style=\"{geometry}\"></span>"
            )
            .map_err(fmt_error)?;
        }
        LayoutKind::PicturePlaceholder => {
            writeln!(
                output,
                "<span class=\"box placeholder\" style=\"{geometry}\">[embedded picture]</span>"
            )
            .map_err(fmt_error)?;
        }
        LayoutKind::EmbeddedWindowPlaceholder {
            descriptor,
            standard_button_label,
            hotspot,
        } => {
            let action = hotspot.as_ref().map(|hotspot| {
                register_hotspot_action(
                    documents,
                    path_map,
                    actions,
                    document,
                    doc_index,
                    hotspot,
                )
            });
            let label = standard_button_label
                .as_deref()
                .unwrap_or(descriptor.as_str());
            if let Some(action) = action {
                let title_attr = hotspot
                    .as_ref()
                    .and_then(|hotspot| {
                        exported_hotspot_tooltip(
                            documents,
                            path_map,
                            document,
                            hotspot,
                        )
                    })
                    .map(|title| format!(" title=\"{}\"", html_escape(&title)))
                    .unwrap_or_default();
                writeln!(
                    output,
                    "<button type=\"button\" class=\"box embedded-control interactive\" data-action=\"{action}\"{title_attr} style=\"{geometry}\">{}</button>",
                    html_escape(label)
                )
                .map_err(fmt_error)?;
            } else {
                writeln!(
                    output,
                    "<span class=\"box placeholder\" style=\"{geometry}\">{}</span>",
                    html_escape(label)
                )
                .map_err(fmt_error)?;
            }
        }
        LayoutKind::Border { flags, style } => {
            writeln!(
                output,
                "<span class=\"box paragraph-border\" style=\"{geometry};{}\"></span>",
                border_style_css(*flags, *style, item.bounds.height)
            )
            .map_err(fmt_error)?;
        }
    }
    Ok(())
}

fn exported_hotspot_tooltip(
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    current_document: &HelpDocument,
    hotspot: &Hotspot,
) -> Option<String> {
    let (document, topic_index, popup) = match &hotspot.target {
        HotspotTarget::Internal { offset, popup } => (
            current_document,
            current_document.resolve_topic_offset(*offset)?,
            *popup,
        ),
        HotspotTarget::ContextHash { hash, popup } => (
            current_document,
            current_document
                .topic_index_for_context_hash(*hash)
                .or_else(|| current_document.resolve_topic_offset(hlp::TopicOffset(*hash)))?,
            *popup,
        ),
        HotspotTarget::External {
            opcode,
            offset,
            help_file,
            ..
        } => {
            let target_doc_index = resolve_export_document_index(
                current_document,
                help_file.as_deref(),
                path_map,
            )?;
            let target_document = &documents.get(target_doc_index)?.document;
            (
                target_document,
                target_document.resolve_topic_offset(*offset)?,
                opcode & 1 == 0,
            )
        }
        HotspotTarget::Macro(_) => return None,
    };

    // The desktop viewer's build-fix 20 behavior is intentionally preserved: ordinary links expose
    // the resolved destination title, while popup-marked links expose the visible popup-topic body.
    // All activation still follows build-fix 16's single-main-surface policy in the HTML shell.
    super::destination_topic_tooltip_by_index(document, topic_index, popup)
}

fn register_hotspot_action(
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    actions: &mut ActionRegistry,
    current_document: &HelpDocument,
    current_doc_index: usize,
    hotspot: &Hotspot,
) -> usize {
    match &hotspot.target {
        HotspotTarget::Internal { offset, popup } => {
            let Some(topic) = current_document.resolve_topic_offset(*offset) else {
                return actions.push_noop(&format!("Unresolved TOPICOFFSET {}", offset.0));
            };
            if *popup {
                actions.push_open(current_doc_index, topic, "popup", None)
            } else {
                let mode = navigation_mode(current_document, topic, None);
                actions.push_open(
                    current_doc_index,
                    topic,
                    mode,
                    explicit_or_default_window(current_document, topic, None),
                )
            }
        }
        HotspotTarget::ContextHash { hash, popup } => {
            let Some(topic) = current_document
                .topic_index_for_context_hash(*hash)
                .or_else(|| current_document.resolve_topic_offset(hlp::TopicOffset(*hash)))
            else {
                return actions.push_noop(&format!(
                    "Unresolved context hash 0x{:08X}",
                    *hash as u32
                ));
            };
            if *popup {
                actions.push_open(current_doc_index, topic, "popup", None)
            } else {
                let mode = navigation_mode(current_document, topic, None);
                actions.push_open(
                    current_doc_index,
                    topic,
                    mode,
                    explicit_or_default_window(current_document, topic, None),
                )
            }
        }
        HotspotTarget::External {
            opcode,
            offset,
            window_number,
            help_file,
            window_name,
            ..
        } => {
            let Some(target_doc_index) = resolve_export_document_index(
                current_document,
                help_file.as_deref(),
                path_map,
            ) else {
                return actions.push_noop("The linked HLP was not embedded in this HTML export.");
            };
            let target_document = &documents[target_doc_index].document;
            let Some(topic) = target_document.resolve_topic_offset(*offset) else {
                return actions.push_noop(&format!(
                    "Unresolved linked TOPICOFFSET {} in {}",
                    offset.0,
                    target_document.source_path().display()
                ));
            };
            if opcode & 1 == 0 {
                return actions.push_open(target_doc_index, topic, "popup", None);
            }
            let explicit = window_name
                .as_deref()
                .and_then(|name| target_document.window_by_name(name))
                .or_else(|| window_number.and_then(|number| target_document.window_by_number(number)));
            let explicit_main = window_name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case("main"))
                || explicit.is_some_and(|window| {
                    window
                        .name
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case("main"))
                });
            if (window_name.is_some() || window_number.is_some()) && !explicit_main {
                actions.push_open(target_doc_index, topic, "secondary", explicit)
            } else if explicit_main {
                actions.push_open(target_doc_index, topic, "main", None)
            } else {
                let mode = navigation_mode(target_document, topic, None);
                actions.push_open(
                    target_doc_index,
                    topic,
                    mode,
                    explicit_or_default_window(target_document, topic, None),
                )
            }
        }
        HotspotTarget::Macro(text) => {
            let program = macro_text_to_program_js(
                current_document,
                documents,
                path_map,
                actions,
                text,
                &mut Vec::new(),
            );
            actions.push_program(program)
        }
    }
}

fn macro_program_js(
    origin_document: &HelpDocument,
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    actions: &mut ActionRegistry,
    macro_texts: &[String],
    warnings: &mut Vec<String>,
) -> String {
    let mut output = String::from("[");
    let mut first = true;
    for text in macro_texts {
        let program = macro_text_to_program_js(
            origin_document,
            documents,
            path_map,
            actions,
            text,
            warnings,
        );
        let inner = program.strip_prefix('[').and_then(|value| value.strip_suffix(']')).unwrap_or("");
        if inner.is_empty() {
            continue;
        }
        if !first {
            output.push(',');
        }
        first = false;
        output.push_str(inner);
    }
    output.push(']');
    output
}

fn macro_text_to_program_js(
    origin_document: &HelpDocument,
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    _actions: &mut ActionRegistry,
    text: &str,
    warnings: &mut Vec<String>,
) -> String {
    let Ok(program) = HelpMacroProgram::parse(text) else {
        warnings.push(format!(
            "{}: malformed macro omitted from HTML export: {text}",
            origin_document.source_path().display()
        ));
        return "[]".to_owned();
    };
    let origin_doc_index = document_index_for_path(path_map, origin_document.source_path()).unwrap_or(0);
    let mut output = String::from("[");
    let mut first = true;
    for macro_ in program.macros {
        let HelpMacro::Allowed(command) = macro_ else {
            continue;
        };
        let Some(operation) = safe_macro_operation_js(
            origin_document,
            origin_doc_index,
            documents,
            path_map,
            command,
        ) else {
            continue;
        };
        if !first {
            output.push(',');
        }
        first = false;
        output.push_str(&operation);
    }
    output.push(']');
    output
}

fn safe_macro_operation_js(
    origin_document: &HelpDocument,
    origin_doc_index: usize,
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    command: SafeHelpMacro,
) -> Option<String> {
    let simple = |kind: &str| Some(format!("{{kind:{}}}", js_string(kind)));
    match command {
        SafeHelpMacro::About => simple("about"),
        SafeHelpMacro::Back => simple("back"),
        SafeHelpMacro::BackFlush => simple("backFlush"),
        SafeHelpMacro::BookmarkDefine => simple("bookmarkAdd"),
        SafeHelpMacro::BookmarkMore => Some("{kind:\"pane\",pane:\"bookmarks\"}".to_owned()),
        SafeHelpMacro::BrowseButtons => simple("browseButtons"),
        SafeHelpMacro::Contents => simple("contents"),
        SafeHelpMacro::Finder => Some("{kind:\"pane\",pane:\"index\"}".to_owned()),
        SafeHelpMacro::History => Some("{kind:\"pane\",pane:\"history\"}".to_owned()),
        SafeHelpMacro::OpenUrl { url } => Some(format!(
            "{{kind:\"url\",url:{}}}",
            js_string(&url)
        )),
        SafeHelpMacro::Search => Some("{kind:\"pane\",pane:\"search\"}".to_owned()),
        SafeHelpMacro::FocusWindow { window } => Some(format!(
            "{{kind:\"focusWindow\",window:{}}}",
            js_string(&window)
        )),
        SafeHelpMacro::Next => Some("{kind:\"browse\",direction:1}".to_owned()),
        SafeHelpMacro::Prev => Some("{kind:\"browse\",direction:-1}".to_owned()),
        SafeHelpMacro::SetPopupColor { red, green, blue } => Some(format!(
            "{{kind:\"popupColor\",doc:{origin_doc_index},color:{}}}",
            js_string(&format!("rgb({red},{green},{blue})"))
        )),
        SafeHelpMacro::ALink { keywords } => {
            let topics = origin_document
                .keywords()
                .lookup_exact('A', &keywords)
                .into_iter()
                .filter_map(|offset| origin_document.resolve_topic_offset(offset))
                .collect::<Vec<_>>();
            let mut topics_js = String::from("[");
            for (index, topic) in topics.iter().enumerate() {
                if index != 0 {
                    topics_js.push(',');
                }
                let _ = write!(topics_js, "{{doc:{origin_doc_index},topic:{topic}}}");
            }
            topics_js.push(']');
            Some(format!(
                "{{kind:\"alink\",keywords:{},topics:{topics_js}}}",
                js_string(&keywords)
            ))
        }
        SafeHelpMacro::JumpContents { help_file, window } => resolve_macro_open(
            origin_document,
            documents,
            path_map,
            &help_file,
            (!window.trim().is_empty()).then_some(window.as_str()),
            MacroTarget::Contents,
            false,
        ),
        SafeHelpMacro::JumpContext { help_file, window, context } => resolve_macro_open(
            origin_document,
            documents,
            path_map,
            &help_file,
            (!window.trim().is_empty()).then_some(window.as_str()),
            MacroTarget::Map(context),
            false,
        ),
        SafeHelpMacro::JumpHash { help_file, window, hash } => resolve_macro_open(
            origin_document,
            documents,
            path_map,
            &help_file,
            (!window.trim().is_empty()).then_some(window.as_str()),
            MacroTarget::Hash(hash),
            false,
        ),
        SafeHelpMacro::JumpId { path_window, topic_id } => {
            let (help_file, window) = split_macro_path_window(&path_window);
            resolve_macro_open(
                origin_document,
                documents,
                path_map,
                help_file,
                window,
                MacroTarget::Id(topic_id),
                false,
            )
        }
        SafeHelpMacro::PopupContext { help_file, context } => resolve_macro_open(
            origin_document,
            documents,
            path_map,
            &help_file,
            None,
            MacroTarget::Map(context),
            true,
        ),
        SafeHelpMacro::PopupHash { help_file, hash } => resolve_macro_open(
            origin_document,
            documents,
            path_map,
            &help_file,
            None,
            MacroTarget::Hash(hash),
            true,
        ),
        SafeHelpMacro::PopupId { help_file, topic_id } => resolve_macro_open(
            origin_document,
            documents,
            path_map,
            &help_file,
            None,
            MacroTarget::Id(topic_id),
            true,
        ),
    }
}

#[derive(Debug)]
enum MacroTarget {
    Contents,
    Map(i32),
    Hash(i32),
    Id(String),
}

fn resolve_macro_open(
    origin_document: &HelpDocument,
    documents: &[ExportDocument],
    path_map: &BTreeMap<String, usize>,
    help_file: &str,
    window_name: Option<&str>,
    target: MacroTarget,
    popup: bool,
) -> Option<String> {
    let doc_index = resolve_export_document_index(origin_document, Some(help_file), path_map)?;
    let document = &documents[doc_index].document;
    let topic = match target {
        MacroTarget::Contents => document.contents_topic_index(),
        MacroTarget::Map(value) => document.topic_index_for_map_id(value),
        MacroTarget::Hash(value) => document.topic_index_for_context_hash(value),
        MacroTarget::Id(value) => document.topic_index_for_context_name(&value),
    }?;
    if popup {
        return Some(open_action_js(doc_index, topic, "popup", None));
    }
    let window = window_name.and_then(|name| document.window_by_name(name));
    let explicit_main = window_name.is_some_and(|name| name.eq_ignore_ascii_case("main"))
        || window.is_some_and(|definition| {
            definition
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case("main"))
        });
    if window_name.is_some() && !explicit_main {
        Some(open_action_js(doc_index, topic, "secondary", window))
    } else if explicit_main {
        Some(open_action_js(doc_index, topic, "main", None))
    } else {
        let mode = navigation_mode(document, topic, None);
        Some(open_action_js(
            doc_index,
            topic,
            mode,
            explicit_or_default_window(document, topic, None),
        ))
    }
}

fn resolve_export_document_index(
    origin_document: &HelpDocument,
    help_file: Option<&str>,
    path_map: &BTreeMap<String, usize>,
) -> Option<usize> {
    let path = help_file
        .filter(|value| !value.trim().is_empty())
        .map_or_else(
            || origin_document.source_path().to_path_buf(),
            |value| resolve_external_help_path(origin_document.source_path(), value),
        );
    document_index_for_path(path_map, &path)
}

fn navigation_mode<'a>(
    document: &'a HelpDocument,
    topic_index: usize,
    explicit_window_name: Option<&str>,
) -> &'static str {
    if explicit_window_name.is_some_and(|name| !name.eq_ignore_ascii_case("main")) {
        return "secondary";
    }
    if explicit_window_name.is_none()
        && document.default_window_for_topic(topic_index).is_some_and(|window| {
            !window
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case("main"))
        })
    {
        return "secondary";
    }
    "main"
}

fn explicit_or_default_window<'a>(
    document: &'a HelpDocument,
    topic_index: usize,
    explicit_window_name: Option<&str>,
) -> Option<&'a WindowDefinition> {
    if let Some(name) = explicit_window_name {
        return document.window_by_name(name).filter(|window| {
            !window
                .name
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case("main"))
        });
    }
    document.default_window_for_topic(topic_index).filter(|window| {
        !window
            .name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case("main"))
    })
}

fn open_action_js(
    doc: usize,
    topic: usize,
    mode: &str,
    window: Option<&WindowDefinition>,
) -> String {
    format!(
        "{{kind:\"open\",doc:{doc},topic:{topic},mode:{},window:{}}}",
        js_string(mode),
        window_js(window)
    )
}

fn window_js(window: Option<&WindowDefinition>) -> String {
    let Some(window) = window else {
        return "null".to_owned();
    };
    let caption = window.caption.as_deref().or(window.name.as_deref()).unwrap_or("Help");
    let fixed = window.non_scrolling_color.map(css_rgb);
    let scrolling = window.scrolling_color.map(css_rgb);
    format!(
        "{{name:{},caption:{},x:{},y:{},width:{},height:{},maximize:{},alwaysOnTop:{},fixed:{},scrolling:{}}}",
        js_optional_string(window.name.as_deref()),
        js_string(caption),
        js_optional_i16(window.x),
        js_optional_i16(window.y),
        js_optional_i16(window.width),
        js_optional_i16(window.height),
        window.maximize,
        window.always_on_top,
        fixed.as_deref().map_or_else(|| "null".to_owned(), js_string),
        scrolling.as_deref().map_or_else(|| "null".to_owned(), js_string),
    )
}

fn geometry_css(item: &LayoutBox) -> String {
    format!(
        "left:{}px;top:{}px;width:{}px;height:{}px",
        item.bounds.x,
        item.bounds.y,
        item.bounds.width.max(0),
        item.bounds.height.max(0)
    )
}

/// Converts an authored `|FONT` size in twentieths of a point to exported CSS pixels.
fn semantic_font_px(point_size_twips: i32) -> f64 {
    let reference = f64::from(point_size_twips.abs().max(20)) * f64::from(EXPORT_DPI) / 1440.0;
    let scale = EXPORT_BASE_FONT_PX / (EXPORT_BASE_POINT_SIZE * f64::from(EXPORT_DPI) / 72.0);
    (reference * scale).max(1.0)
}

/// Builds the inline CSS for one semantic text run.
///
/// Every authored `|FONT` attribute is emitted explicitly so the browser cannot fall back to the
/// shell's own font: family, size, weight, italic, underline, strikeout, and small caps.
/// `font-synthesis` is stated so a face without a real bold/italic/small-caps cut is still
/// synthesized rather than silently rendered as regular text.
///
/// Small caps follow WinHlp32 (`0x411a59..0x411a6c`), which renders the HC30 small-caps attribute
/// by reducing the authored cell height to two thirds and drawing the authored characters. That is
/// exactly right for the usual authored all-capitals key names such as `NUM LOCK`. When the run
/// still contains lower-case characters the exporter keeps the authored size and asks the browser
/// for real small-capital shaping instead, which is what the attribute means typographically and
/// avoids rendering ordinary prose two thirds too small.
fn semantic_text_style_css(style: &ResolvedTextStyle, text: &str) -> String {
    let has_lowercase = text.chars().any(char::is_lowercase);
    let authored_twips = if style.small_caps && !has_lowercase {
        style.point_size_twips.saturating_mul(2) / 3
    } else {
        style.point_size_twips
    };
    let px = semantic_font_px(authored_twips);
    let mut decoration = Vec::new();
    if style.underline {
        decoration.push("underline");
    }
    if style.strike_out {
        decoration.push("line-through");
    }
    let decoration = if decoration.is_empty() {
        "none".to_owned()
    } else {
        decoration.join(" ")
    };
    let mut css = format!(
        "font-family:{};font-size:{px:.3}px;font-weight:{};font-style:{};text-decoration:{decoration};font-synthesis:weight style small-caps;",
        html_font_family(style),
        style.weight.clamp(100, 1000),
        if style.italic { "italic" } else { "normal" },
    );
    if style.small_caps {
        css.push_str("font-variant-caps:small-caps;");
    }
    css
}

/// Font family and size that unstyled content inside a region inherits.
///
/// WinHlp32 selects descriptor 0 once per topic render and only changes it on character opcode
/// `0x80`, so descriptor 0 is the topic's initial font. Using it for the region keeps authored
/// blank paragraphs at the height the original viewer gave them instead of the browser default.
fn semantic_region_base_css(fonts: &FontTable) -> String {
    let Some(descriptor) = fonts.descriptors().first() else {
        return String::new();
    };
    let style = semantic_style_from_font(descriptor, None);
    let px = semantic_font_px(style.point_size_twips);
    format!("font-family:{};font-size:{px:.3}px", html_font_family(&style))
}

fn text_style_css(style: &ResolvedTextStyle) -> String {
    let authored_twips = if style.small_caps {
        style.point_size_twips.saturating_mul(2) / 3
    } else {
        style.point_size_twips
    };
    let px = (f64::from(authored_twips.abs().max(20)) * 96.0 / 1440.0).max(1.0);
    let family = html_font_family(style);
    let mut decoration = Vec::new();
    if style.underline {
        decoration.push("underline");
    }
    if style.strike_out {
        decoration.push("line-through");
    }
    let decoration = if decoration.is_empty() {
        "none".to_owned()
    } else {
        decoration.join(" ")
    };
    format!(
        "font-family:{family};font-size:{px:.3}px;font-weight:{};font-style:{};text-decoration:{}",
        style.weight.clamp(100, 1000),
        if style.italic { "italic" } else { "normal" },
        decoration,
    )
}

fn html_font_family(style: &ResolvedTextStyle) -> String {
    let authored = css_quoted_font(&style.face_name);
    if is_semantic_symbol_face(&style.face_name) {
        return format!("{authored},sans-serif");
    }
    if style.charset.is_some_and(|charset| !matches!(charset, 0x00 | 0x01)) {
        return format!("{authored},sans-serif");
    }
    if style.family == ResolvedFontFamily::Monospace {
        return "Consolas,'Courier New',monospace".to_owned();
    }
    // Every quoted family name is single-quoted: these lists are written into a double-quoted
    // HTML style attribute and a double quote would truncate the whole declaration list.
    match style.source_family {
        hlp::HlpFontFamily::Roman => "'Times New Roman',Times,serif".to_owned(),
        hlp::HlpFontFamily::Swiss => "'Segoe UI',Arial,sans-serif".to_owned(),
        hlp::HlpFontFamily::Script => "'Segoe Script',cursive".to_owned(),
        hlp::HlpFontFamily::Decorative => format!("{authored},serif"),
        hlp::HlpFontFamily::Modern => "Consolas,'Courier New',monospace".to_owned(),
        hlp::HlpFontFamily::Unknown(_) => "'Segoe UI',Arial,sans-serif".to_owned(),
    }
}

fn border_style_css(flags: BorderFlags, style: BorderStyle, height: i32) -> String {
    if matches!(style, BorderStyle::Reserved(_)) {
        return "border:none".to_owned();
    }
    let width = if matches!(style, BorderStyle::Thick) { 2 } else { 1 };
    let compact_separator = !flags.box_all
        && flags.top
        && flags.bottom
        && !flags.left
        && !flags.right
        && height <= 16;
    if compact_separator {
        return format!("border-top:{width}px solid #000");
    }
    let mut css = String::new();
    let all = flags.box_all;
    if all || flags.top {
        let _ = write!(css, "border-top:{width}px solid #000;");
    }
    if all || flags.left {
        let _ = write!(css, "border-left:{width}px solid #000;");
    }
    if all || flags.bottom {
        let _ = write!(css, "border-bottom:{width}px solid #000;");
    }
    if all || flags.right {
        let _ = write!(css, "border-right:{width}px solid #000;");
    }
    if matches!(style, BorderStyle::Double) {
        css.push_str("outline:1px solid #000;outline-offset:-3px;");
    } else if matches!(style, BorderStyle::Shadow) {
        css.push_str("filter:drop-shadow(1px 1px 0 #000);");
    }
    css
}

fn normalize_html_extension(path: &Path) -> PathBuf {
    if path
        .extension()
        .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("html"))
    {
        path.to_path_buf()
    } else {
        path.with_extension("html")
    }
}

fn topic_label(title: &str, topic_index: usize) -> String {
    if title.trim().is_empty() {
        format!("Topic {}", topic_index + 1)
    } else {
        title.to_owned()
    }
}

fn template_id(doc: usize, topic: usize) -> String {
    format!("hlp-topic-{doc}-{topic}")
}

fn path_identity(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

fn automatic_export_reference_allowed(target: &str) -> bool {
    let normalized = target.trim().replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') || normalized.starts_with("//") {
        return false;
    }
    let bytes = normalized.as_bytes();
    !bytes.get(1).is_some_and(|byte| *byte == b':')
}

fn split_macro_path_window(value: &str) -> (&str, Option<&str>) {
    value
        .split_once('>')
        .map_or((value.trim(), None), |(file, window)| {
            let window = window.trim();
            (file.trim(), (!window.is_empty()).then_some(window))
        })
}

fn push_unique_string(values: &mut Vec<String>, value: &str) {
    if !value.trim().is_empty() && !values.iter().any(|existing| existing.eq_ignore_ascii_case(value)) {
        values.push(value.to_owned());
    }
}

fn is_semantic_symbol_face(face_name: &str) -> bool {
    let normalized = face_name.to_ascii_lowercase();
    normalized.contains("symbol")
        || normalized.contains("wingdings")
        || normalized.contains("webdings")
        || normalized.contains("dingbats")
        || normalized == "marlett"
}

fn css_rgb(color: Rgb) -> String {
    format!("rgb({},{},{})", color.red, color.green, color.blue)
}

/// Quotes one CSS font-family name.
///
/// The result is written into a double-quoted HTML `style` attribute, so the family name is
/// deliberately single-quoted. A double-quoted family name terminated the attribute early and made
/// the browser silently discard every declaration after `font-family:` - which is why authored
/// size, weight, italic, underline, small-caps, and colour disappeared from exported topics.
fn css_quoted_font(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Escapes one complete CSS declaration list for use inside a double-quoted HTML attribute.
fn style_attribute(css: &str) -> String {
    html_escape(css)
}

fn html_escape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(character),
        }
    }
    output
}

fn js_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len().saturating_add(2));
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{000C}' => output.push_str("\\f"),
            '<' => output.push_str("\\u003C"),
            '>' => output.push_str("\\u003E"),
            '&' => output.push_str("\\u0026"),
            character if character <= '\u{001F}' => {
                let _ = write!(output, "\\u{:04X}", character as u32);
            }
            _ => output.push(character),
        }
    }
    output.push('"');
    output
}

fn js_optional_string(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), js_string)
}

fn js_optional_i16(value: Option<i16>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| value.to_string())
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3).saturating_mul(4));
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        output.push(TABLE[usize::from(a >> 2)] as char);
        output.push(TABLE[usize::from(((a & 0x03) << 4) | (b >> 4))] as char);
        if chunk.len() > 1 {
            output.push(TABLE[usize::from(((b & 0x0F) << 2) | (c >> 6))] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(TABLE[usize::from(c & 0x3F)] as char);
        } else {
            output.push('=');
        }
    }
    output
}

fn fmt_error(error: std::fmt::Error) -> String {
    error.to_string()
}

const EXPORT_SHELL: &str = r#"
<div id="hlp-app" class="hlp-app">
  <header class="toolbar" aria-label="Help navigation">
    <button id="back-button" type="button" title="Back">◀</button>
    <button id="forward-button" type="button" title="Forward">▶</button>
    <span class="toolbar-separator"></span>
    <button id="previous-button" type="button" title="Previous topic">Previous</button>
    <button id="next-button" type="button" title="Next topic">Next</button>
    <button id="browse-previous-button" type="button" title="Previous authored browse topic" hidden>Browse Prev</button>
    <button id="browse-next-button" type="button" title="Next authored browse topic" hidden>Browse Next</button>
    <span class="toolbar-separator"></span>
    <button id="navigation-toggle" type="button" title="Show or hide navigation pane">☰</button>
    <button id="zoom-out-button" type="button" title="Zoom out">−</button>
    <button id="zoom-in-button" type="button" title="Zoom in">+</button>
    <button id="print-button" type="button" title="Print this help topic">Print</button>
    <span id="toolbar-title" class="toolbar-title"></span>
  </header>
  <div class="workspace">
    <aside id="navigation-pane" class="navigation-pane">
      <div class="tabs" role="tablist">
        <button class="tab active" data-pane="contents" type="button">Contents</button>
        <button class="tab" data-pane="index" type="button">Index</button>
        <button class="tab" data-pane="search" type="button">Search</button>
        <button class="tab" data-pane="bookmarks" type="button">Bookmarks</button>
        <button class="tab" data-pane="history" type="button">History</button>
      </div>
      <section id="pane-contents" class="nav-page active">
        <div class="contents-mode"><button id="contents-hierarchy" class="active" type="button">Hierarchical view</button><button id="contents-all" type="button">Show all</button></div>
        <div id="contents-list" class="nav-list"></div>
      </section>
      <section id="pane-index" class="nav-page">
        <input id="index-query" class="query" type="search" placeholder="Filter index" autocomplete="off">
        <div id="index-list" class="nav-list"></div>
      </section>
      <section id="pane-search" class="nav-page">
        <input id="search-query" class="query" type="search" placeholder="Search help" autocomplete="off">
        <div id="search-list" class="nav-list"></div>
      </section>
      <section id="pane-bookmarks" class="nav-page">
        <div class="bookmark-tools"><button id="bookmark-add" type="button">+</button><button id="bookmark-remove" type="button">−</button></div>
        <div id="bookmarks-list" class="nav-list"></div>
      </section>
      <section id="pane-history" class="nav-page"><div id="history-list" class="nav-list"></div></section>
    </aside>
    <div id="nav-splitter" class="nav-splitter" role="separator" aria-orientation="vertical" aria-label="Resize navigation pane" tabindex="0"></div>
    <main id="content-host" class="content-host">
      <div class="page-border"><div id="main-topic" class="main-topic"></div></div>
    </main>
  </div>
  <footer id="status" class="status">Exported from Rust HLP Viewer</footer>
</div>
<div id="popup" class="popup" hidden><button class="popup-close" type="button" aria-label="Close">×</button><div id="popup-topic" class="popup-topic"></div></div>
<div id="secondary-shade" class="secondary-shade" hidden><section id="secondary" class="secondary"><header><span id="secondary-title">Help</span><button id="secondary-close" type="button" aria-label="Close">×</button></header><div id="secondary-topic" class="secondary-topic"></div></section></div>
<div id="choice-shade" class="secondary-shade" hidden><section class="choice"><header><span>Topics Found</span><button id="choice-close" type="button">×</button></header><p>Select a related topic.</p><div id="choice-list" class="nav-list"></div></section></div>
"#;

const EXPORT_CSS: &str = r#"
:root{--host:#d4d4d4;--page:#ffffe4;--info:#f9f99e;--chrome:#efefef;--border:#000;--selection:#000080;--hotspot:#008000;--hotspot-hover:#006400;--control-drop:4px;font-family:"Segoe UI",Arial,sans-serif;color:#000}
*{box-sizing:border-box}html,body{margin:0;width:100%;height:100%;overflow:hidden;background:var(--host)}button,input{font:inherit;color:inherit}.hlp-app{display:grid;grid-template-rows:auto 1fr auto;width:100vw;height:100vh;background:var(--host)}
.toolbar{display:flex;align-items:center;gap:6px;padding:8px 10px;background:var(--chrome);border-bottom:1px solid #b8b8b8;min-height:46px}.toolbar button,.contents-mode button,.bookmark-tools button{min-height:28px;border:1px solid #aaa;background:#f7f7f7;padding:3px 10px}.toolbar button:active,.contents-mode button.active,.tab.active{background:#ddd}.toolbar button:disabled{opacity:.45}.toolbar-separator{width:1px;height:26px;background:#bbb;margin:0 4px}.toolbar-title{margin-left:8px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-weight:600}
.workspace{display:grid;grid-template-columns:var(--nav-width,300px) 5px 1fr;min-height:0}.workspace.navigation-hidden{grid-template-columns:0 0 1fr}.navigation-pane{min-width:0;overflow:hidden;background:#f7f7f7;display:grid;grid-template-rows:auto 1fr}.navigation-hidden .navigation-pane{visibility:hidden}.nav-splitter{cursor:col-resize;background:#c8c8c8;border-left:1px solid #999;border-right:1px solid #aaa;touch-action:none}.nav-splitter:hover,.nav-splitter:focus{background:#b9b9b9;outline:none}.navigation-hidden .nav-splitter{visibility:hidden}.tabs{display:flex;flex-wrap:wrap;background:#e7e7e7;border-bottom:1px solid #aaa}.tab{flex:1 1 auto;border:0;border-right:1px solid #aaa;background:#eee;padding:7px 8px;white-space:nowrap}.nav-page{display:none;min-height:0;overflow:hidden}.nav-page.active{display:grid;grid-template-rows:auto 1fr}.nav-page#pane-history{grid-template-rows:1fr}.contents-mode{display:flex;gap:4px;padding:5px}.contents-mode button{font-size:12px;padding:3px 6px}.query{margin:5px;border:1px solid #999;padding:5px;background:white}.nav-list{overflow:auto;background:#fff;border-top:1px solid #ccc}.nav-row{display:block;width:100%;border:0;background:transparent;text-align:left;padding:3px 7px;line-height:1.25;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.nav-row:hover,.nav-row:focus{background:#e7eefb;outline:none}.nav-row.selected{background:var(--selection);color:#fff}.nav-row.selected:hover,.nav-row.selected:focus{background:var(--selection);color:#fff}.nav-row.book{font-weight:600}.nav-row.disabled{color:#777;cursor:default}.contents-tree{min-width:max-content;padding:1px 0}.contents-node{display:block}.contents-entry{display:flex;align-items:stretch;min-width:max-content}.contents-entry>.nav-row{width:auto;min-width:0;flex:1 1 auto;padding-left:2px}.contents-expander,.contents-spacer{flex:0 0 18px;width:18px;min-width:18px}.contents-expander{border:0;background:transparent;padding:0;font:11px/1 "Segoe UI Symbol","Segoe UI",sans-serif;text-align:center;cursor:default}.contents-expander:hover,.contents-expander:focus{background:#e7eefb;outline:none}.contents-children{margin-left:16px}.bookmark-tools{display:flex;gap:5px;padding:5px}.bookmark-tools button{width:34px}
.content-host{min-width:0;min-height:0;overflow:auto;background:var(--host);padding:12px}.page-border{width:100%;max-width:100%;min-width:0;min-height:100%;margin:0 auto;border:1px solid #000;background:var(--page)}.main-topic{width:100%;min-width:0;max-width:100%;background:var(--page)}.semantic-topic{width:auto;max-width:100%}.topic-view{position:relative;background:var(--page);color:#000}.topic-region{position:relative;overflow:visible}.fixed-region{position:sticky;top:0;z-index:5}.box{position:absolute;display:block;margin:0;padding:0}.text-box{white-space:pre;line-height:1;overflow:visible}.text-glyphs{display:inline-block;white-space:pre;line-height:1}.text-box.export-heading .text-glyphs{font-weight:700!important}.text-box.interactive .text-glyphs{color:var(--hotspot)!important;text-decoration:underline!important}.text-box.interactive:hover .text-glyphs,.text-box.interactive:focus .text-glyphs{color:var(--hotspot-hover)!important}.interactive{cursor:pointer}.interactive:focus{outline:1px dotted #000;outline-offset:1px}.picture{image-rendering:auto}.picture-hotspot{background:transparent}.placeholder{border:1px dashed #777;color:#555;font:11px "Segoe UI",sans-serif;overflow:hidden;white-space:nowrap}.embedded-control{border:1px solid #777;background:#ececec;overflow:hidden;white-space:nowrap;font:11px "Segoe UI",sans-serif}.paragraph-border{pointer-events:none}
.semantic-region{width:100%;max-width:100%;height:auto!important;min-height:0;overflow:visible}.semantic-region-empty{height:0!important;min-height:0}.semantic-region-inner{padding:12px;display:flow-root}.semantic-region.fixed-region .semantic-region-inner{padding-bottom:6px}.hlp-paragraph{display:block;position:relative;box-sizing:border-box;min-width:0;max-width:100%;padding:0;line-height:1.35;overflow-wrap:normal;word-break:normal;hyphens:none}.hlp-blank-paragraph::before{content:"\00a0"}.hlp-paragraph:after{content:"";display:block;clear:both}.hlp-run{white-space:inherit}.topic-heading .hlp-run{font-weight:700!important}.hlp-link{color:var(--hotspot)!important;text-decoration:underline!important;text-decoration-thickness:auto;text-underline-offset:auto}.hlp-link:hover,.hlp-link:focus{color:var(--hotspot-hover)!important}.hlp-hard-break{}.hlp-tabbed-paragraph{text-indent:0!important}.hlp-tab-grid{display:grid;min-width:0;align-items:start}.hlp-tab-segment{display:block;min-width:0;white-space:normal}.hlp-tabbed-paragraph[style*="white-space:nowrap"] .hlp-tab-segment{white-space:nowrap}.hlp-picture{position:relative;display:inline-block;vertical-align:bottom;line-height:0}.hlp-picture canvas{display:block;width:100%;height:100%}.hlp-picture-left{float:left;margin:0 6px 4px 0}.hlp-picture-right{float:right;margin:0 0 4px 6px}.hlp-picture .picture-hotspot{position:absolute;display:block;background:transparent}.hlp-table{display:flex;align-items:flex-start;max-width:100%;min-width:0}.hlp-table-column{flex:0 0 auto;min-width:0}.hlp-table-cell{display:flow-root;min-width:0}.semantic-control{display:inline-block;vertical-align:calc(0px - var(--control-drop,4px));min-width:30px;min-height:16px;border:1px solid #999;background:#ececec;padding:1px 4px;font:inherit}.semantic-control.empty-label{width:12px;height:12px;min-width:12px;min-height:12px;padding:0;vertical-align:calc(0px - var(--control-drop,4px))}.semantic-placeholder{position:relative;display:inline-block}.semantic-hosted{width:min(192px,100%);height:192px}.hlp-origin-reset{display:inline-block;width:0;height:0;position:relative;left:var(--hlp-origin)}
.status{min-height:24px;padding:3px 8px;background:#efefef;border-top:1px solid #aaa;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.popup{position:fixed;z-index:1000;max-width:min(560px,calc(100vw - 20px));max-height:min(440px,calc(100vh - 20px));overflow:auto;border:1px solid #000;background:var(--info);box-shadow:2px 2px 5px #555}.popup .topic-view,.popup .topic-region{background:var(--popup-background,var(--info))!important}.popup-close{position:sticky;float:right;top:2px;right:2px;z-index:20;border:1px solid #888;background:#eee;width:24px;height:24px}.popup-topic{min-width:320px}
.secondary-shade{position:fixed;inset:0;z-index:1100;background:rgba(0,0,0,.18);display:flex;align-items:center;justify-content:center;padding:24px}.secondary,.choice{display:grid;grid-template-rows:auto 1fr;min-width:420px;max-width:min(94vw,980px);min-height:180px;max-height:90vh;background:var(--host);border:1px solid #333;box-shadow:4px 4px 14px #555}.secondary>header,.choice>header{display:flex;justify-content:space-between;align-items:center;padding:5px 8px;background:#ececec;border-bottom:1px solid #999;font-weight:600}.secondary>header button,.choice>header button{border:1px solid #999;background:#f8f8f8;width:26px;height:25px}.secondary-topic{overflow:auto;background:var(--host);padding:8px}.choice{grid-template-rows:auto auto 1fr}.choice p{margin:9px}.choice .nav-list{min-height:100px}.secondary-shade[hidden],.popup[hidden]{display:none}
@media(max-width:720px){.workspace{--nav-width:240px}.content-host{padding:6px}.toolbar button{padding:3px 6px}.toolbar-title{display:none}}
@media print{html,body{overflow:visible;background:#fff}.toolbar,.navigation-pane,.nav-splitter,.status,.popup,.secondary-shade{display:none!important}.workspace{display:block}.content-host{overflow:visible;padding:0;background:#fff}.page-border{border:0;margin:0;background:#fff}.main-topic{width:auto!important;min-width:0;zoom:1!important}.fixed-region{position:relative;top:auto}}
"#;

const EXPORT_JS: &str = r#"
(() => {
  "use strict";
  const byId = id => document.getElementById(id);
  const app = byId("hlp-app");
  const workspace = app.querySelector(".workspace");
  const mainHost = byId("main-topic");
  const status = byId("status");
  const popup = byId("popup");
  const popupHost = byId("popup-topic");
  const secondaryShade = byId("secondary-shade");
  const secondary = byId("secondary");
  const secondaryHost = byId("secondary-topic");
  const choiceShade = byId("choice-shade");
  const choiceList = byId("choice-list");
  const bookmarkStorageKey = `rust-hlp-viewer:${DOCUMENTS.map(doc=>doc.name).join("|")}:bookmarks`;
  function loadStoredBookmarks(){
    try{
      const parsed=JSON.parse(localStorage.getItem(bookmarkStorageKey)||"[]");
      if(!Array.isArray(parsed)) return [];
      return parsed.filter(loc=>Number.isInteger(loc?.doc)&&Number.isInteger(loc?.topic)&&!!topic(loc.doc,loc.topic)).slice(0,500).map(loc=>({doc:loc.doc,topic:loc.topic}));
    }catch(_error){ return []; }
  }
  function storeBookmarks(bookmarks){ try{ localStorage.setItem(bookmarkStorageKey,JSON.stringify(bookmarks)); }catch(_error){} }
  const state = { current:null, back:[], forward:[], history:[], bookmarks:loadStoredBookmarks(), browseButtons:false, popupColors:new Map(), lastConfiguredDoc:null, zoom:Math.max(70,Math.min(200,INITIAL_ZOOM||100)) };
  const navSplitter = byId("nav-splitter");
  let splitterDrag = null;

  function setStatus(text){ status.textContent = text || ""; }
  function setNavigationWidth(width){
    const maxWidth=Math.max(220,Math.min(620,window.innerWidth-260));
    workspace.style.setProperty("--nav-width",`${Math.max(180,Math.min(maxWidth,width))}px`);
    fitTopicWidth();
  }
  function topic(doc, index){ return DOCUMENTS[doc] && DOCUMENTS[doc].topics[index]; }
  function locationTitle(loc){ const t=topic(loc.doc,loc.topic); return t ? t.title : `Topic ${loc.topic+1}`; }
  function sameLocation(a,b){ return !!a && !!b && a.doc===b.doc && a.topic===b.topic; }

  function applyZoom(){
    const value=`${state.zoom}%`;
    mainHost.style.zoom=value; popupHost.style.zoom=value; secondaryHost.style.zoom=value;
    byId("zoom-out-button").disabled=state.zoom<=70; byId("zoom-in-button").disabled=state.zoom>=200;
    fitTopicWidth();
  }
  function adjustZoom(delta){ state.zoom=Math.max(70,Math.min(200,state.zoom+delta)); applyZoom(); setStatus(`Text zoom: ${state.zoom}%`); }

  // Ordinary prose is wrapped by the browser, so the only thing that has to be kept in step with
  // the window, the navigation splitter, and the text zoom is the width of the surface the browser
  // wraps inside. A zoomed element scales its own pixel lengths, so the surface is sized in
  // pre-zoom pixels: available / factor rendered at factor is exactly the visible page width.
  // This deliberately measures the container only; no glyph, word, or paragraph is measured or
  // repositioned here.
  let appliedTopicWidth=0;
  function fitTopicWidth(){
    const host=byId("content-host");
    if(!host) return;
    const styles=getComputedStyle(host);
    const padding=(parseFloat(styles.paddingLeft)||0)+(parseFloat(styles.paddingRight)||0);
    const available=host.clientWidth-padding-2;
    const factor=Math.max(0.1,state.zoom/100);
    const target=Math.max(160,Math.round(available/factor));
    if(Math.abs(target-appliedTopicWidth)<1) return;
    appliedTopicWidth=target;
    mainHost.style.width=`${target}px`;
  }

  function initCanvases(root){
    root.querySelectorAll("canvas[data-rgba]").forEach(canvas => {
      try{
        const binary=atob(canvas.dataset.rgba||"");
        const bytes=new Uint8ClampedArray(binary.length);
        for(let i=0;i<binary.length;i++) bytes[i]=binary.charCodeAt(i);
        const ctx=canvas.getContext("2d");
        if(ctx && bytes.length===canvas.width*canvas.height*4) ctx.putImageData(new ImageData(bytes,canvas.width,canvas.height),0,0);
      }catch(error){ console.warn("Could not decode embedded HLP picture",error); }
    });
  }

  // Topic text is already ordinary semantic HTML. There is deliberately no post-layout glyph
  // measurement or word-position correction here: the browser owns shaping and wrapping.
  function cloneTopic(doc,index){
    const meta=topic(doc,index); if(!meta) return null;
    const template=byId(meta.template); if(!template) return null;
    const fragment=template.content.cloneNode(true);
    initCanvases(fragment);
    return fragment;
  }

  function updateToolbar(){
    const loc=state.current; const meta=loc && topic(loc.doc,loc.topic);
    byId("back-button").disabled=state.back.length===0;
    byId("forward-button").disabled=state.forward.length===0;
    byId("previous-button").disabled=!loc || loc.topic<=0;
    byId("next-button").disabled=!loc || loc.topic+1>=DOCUMENTS[loc.doc].topics.length;
    byId("browse-previous-button").hidden=!state.browseButtons;
    byId("browse-next-button").hidden=!state.browseButtons;
    byId("browse-previous-button").disabled=!meta || meta.browsePrev===null;
    byId("browse-next-button").disabled=!meta || meta.browseNext===null;
    byId("toolbar-title").textContent=loc ? `${locationTitle(loc)} — ${DOCUMENTS[loc.doc].name}` : EXPORT_TITLE;
  }

  function rememberHistory(loc){
    if(!loc) return;
    state.history=state.history.filter(item=>!sameLocation(item,loc));
    state.history.unshift({...loc});
    if(state.history.length>100) state.history.length=100;
    renderHistory();
  }

  function openMain(doc,index,{record=true,runMacros=true}={}){
    const meta=topic(doc,index); if(!meta){ setStatus("The exported topic is unavailable."); return; }
    const next={doc,topic:index};
    if(record && state.current && !sameLocation(state.current,next)){ state.back.push({...state.current}); state.forward.length=0; }
    closePopup();
    mainHost.replaceChildren();
    const fragment=cloneTopic(doc,index); if(fragment) mainHost.appendChild(fragment);
    state.current=next; rememberHistory(next); updateToolbar(); syncContentsSelection();
    document.title=`${meta.title} - ${EXPORT_TITLE}`;
    setStatus(`Topic ${index+1}/${DOCUMENTS[doc].topics.length} · ${DOCUMENTS[doc].name}`);
    if(runMacros){
      if(state.lastConfiguredDoc!==doc){ state.lastConfiguredDoc=doc; executeOps(DOCUMENTS[doc].config); }
      if(state.current && state.current.doc===doc && state.current.topic===index) executeOps(meta.macros);
    }
  }

  function openPopup(doc,index,anchor){
    const fragment=cloneTopic(doc,index); if(!fragment) return;
    popupHost.replaceChildren(fragment); popup.hidden=false;
    popup.style.setProperty("--popup-background",state.popupColors.get(doc)||"var(--info)");
    const x=Math.min((anchor?.clientX ?? 80)+8,window.innerWidth-340);
    const y=Math.min((anchor?.clientY ?? 80)+8,window.innerHeight-180);
    popup.style.left=`${Math.max(4,x)}px`; popup.style.top=`${Math.max(4,y)}px`;
    setStatus(`Popup: ${locationTitle({doc,topic:index})}`);
  }
  function closePopup(){ popup.hidden=true; popupHost.replaceChildren(); }

  function openSecondary(action){
    const fragment=cloneTopic(action.doc,action.topic); if(!fragment) return;
    const win=action.window||{};
    byId("secondary-title").textContent=win.caption||locationTitle({doc:action.doc,topic:action.topic});
    secondaryHost.replaceChildren(fragment);
    secondaryHost.style.background=win.scrolling||"var(--page)";
    const fixedRegion=secondaryHost.querySelector(".fixed-region");
    const scrollingRegion=secondaryHost.querySelector(".scrolling-region");
    if(fixedRegion&&win.fixed) fixedRegion.style.setProperty("background",win.fixed,"important");
    if(scrollingRegion&&win.scrolling) scrollingRegion.style.setProperty("background",win.scrolling,"important");
    const topicView=secondaryHost.querySelector(".topic-view");
    if(topicView&&win.scrolling) topicView.style.background=win.scrolling;
    applyZoom();
    const width=Number.isFinite(win.width)&&win.width>0?Math.max(360,win.width):760;
    const height=Number.isFinite(win.height)&&win.height>0?Math.max(200,win.height):520;
    secondary.style.width=`min(94vw,${width}px)`; secondary.style.height=`min(90vh,${height}px)`;
    secondaryShade.hidden=false;
  }
  function closeSecondary(){ secondaryShade.hidden=true; secondaryHost.replaceChildren(); }

  function activate(action,event){
    if(!action) return;
    switch(action.kind){
      case "open":
        // The native viewer intentionally uses a single topic surface (build-fix 16): popup and
        // secondary-window targets still resolve their destination but activate in the main view.
        openMain(action.doc,action.topic);
        break;
      case "url": window.open(action.url,"_blank","noopener,noreferrer"); break;
      case "program": executeOps(action.ops,event); break;
      case "noop": setStatus(action.message||"This action is unavailable in the export."); break;
    }
  }

  function executeOps(ops,event){
    if(!Array.isArray(ops)) return;
    let budget=128;
    for(const op of ops){
      if(--budget<0){ setStatus("WinHelp macro execution limit reached."); break; }
      executeOp(op,event);
    }
  }
  function executeOp(op,event){
    if(!op) return;
    if(op.kind==="open"||op.kind==="url"){ activate(op,event); return; }
    switch(op.kind){
      case "about": alert(`${EXPORT_TITLE}\n\nInteractive HTML exported by Rust HLP Viewer.`); break;
      case "back": goBack(); break;
      case "backFlush": state.back.length=0; state.forward.length=0; updateToolbar(); setStatus("WinHelp Back history cleared"); break;
      case "bookmarkAdd": addBookmark(); break;
      case "browseButtons": state.browseButtons=true; updateToolbar(); break;
      case "contents": if(ROOT_CONTENTS_ACTION!==null) activate(ACTIONS[ROOT_CONTENTS_ACTION],event); break;
      case "pane": showPane(op.pane); break;
      case "focusWindow": if(!op.window || String(op.window).toLowerCase()==="main") mainHost.focus?.(); break;
      case "browse": browse(op.direction); break;
      case "popupColor": state.popupColors.set(op.doc,op.color); break;
      case "alink": showALink(op.topics); break;
    }
  }

  function goBack(){ if(!state.back.length) return; const target=state.back.pop(); if(state.current) state.forward.push({...state.current}); openMain(target.doc,target.topic,{record:false}); }
  function goForward(){ if(!state.forward.length) return; const target=state.forward.pop(); if(state.current) state.back.push({...state.current}); openMain(target.doc,target.topic,{record:false}); }
  function physical(direction){ if(!state.current) return; const index=state.current.topic+direction; if(index>=0&&index<DOCUMENTS[state.current.doc].topics.length) openMain(state.current.doc,index); }
  function browse(direction){ if(!state.current) return; const meta=topic(state.current.doc,state.current.topic); const target=direction>0?meta.browseNext:meta.browsePrev; if(target!==null) openMain(state.current.doc,target); else setStatus(direction>0?"No authored Next topic":"No authored Previous topic"); }

  function showPane(name){
    workspace.classList.remove("navigation-hidden");
    document.querySelectorAll(".tab").forEach(tab=>tab.classList.toggle("active",tab.dataset.pane===name));
    document.querySelectorAll(".nav-page").forEach(page=>page.classList.toggle("active",page.id===`pane-${name}`));
    if(name==="history") renderHistory(); if(name==="bookmarks") renderBookmarks();
    const input=byId(`${name}-query`); if(input) setTimeout(()=>input.focus(),0);
  }

  function makeRow(label,click,className=""){
    const button=document.createElement("button"); button.type="button"; button.className=`nav-row ${className}`.trim(); button.textContent=label; button.title=label; if(click) button.addEventListener("click",click); else button.classList.add("disabled"); return button;
  }
  function setContentsNodeExpanded(node,expanded){
    const children=node.querySelector(":scope > .contents-children");
    const expander=node.querySelector(":scope > .contents-entry > .contents-expander");
    const row=node.querySelector(":scope > .contents-entry > .nav-row");
    if(!children||!expander) return;
    children.hidden=!expanded;
    expander.textContent=expanded?"▼":"▶";
    expander.setAttribute("aria-expanded",expanded?"true":"false");
    expander.title=expanded?"Collapse":"Expand";
    expander.setAttribute("aria-label",`${expanded?"Collapse":"Expand"} ${row?.textContent||"Contents branch"}`);
    row?.setAttribute("aria-expanded",expanded?"true":"false");
  }
  function syncContentsSelection(){
    if(!state.current||state.current.doc!==ROOT_DOC) return;
    const host=byId("contents-list");
    host.querySelectorAll(".nav-row.selected").forEach(row=>row.classList.remove("selected"));
    const node=host.querySelector(`.contents-node[data-doc="${state.current.doc}"][data-topic="${state.current.topic}"]`);
    if(node){
      const row=node.querySelector(":scope > .contents-entry > .nav-row");
      row?.classList.add("selected");
      let group=node.parentElement;
      while(group&&group.classList.contains("contents-children")){
        const parentNode=group.parentElement;
        if(!parentNode?.classList.contains("contents-node")) break;
        setContentsNodeExpanded(parentNode,true);
        group=parentNode.parentElement;
      }
      row?.scrollIntoView({block:"nearest"});
      return;
    }
    const flat=host.querySelector(`.nav-row[data-doc="${state.current.doc}"][data-topic="${state.current.topic}"]`);
    flat?.classList.add("selected");
  }
  function renderHierarchicalContents(host){
    if(!CONTENTS.length){
      host.append(makeRow("Hierarchical contents unavailable (.CNT/.GID data not found)",null));
      return;
    }
    const root=document.createElement("div"); root.className="contents-tree"; root.setAttribute("role","tree");
    host.append(root);
    const ancestors=[];
    CONTENTS.forEach((row,index)=>{
      while(ancestors.length&&ancestors[ancestors.length-1].level>=row.level) ancestors.pop();
      const parent=ancestors.length?ancestors[ancestors.length-1].children:root;
      const next=CONTENTS[index+1];
      const hasChildren=!!next&&next.level>row.level;
      const node=document.createElement("div"); node.className="contents-node";
      const action=row.action===null?null:ACTIONS[row.action];
      if(action?.kind==="open"){
        node.dataset.doc=String(action.doc);
        node.dataset.topic=String(action.topic);
      }
      const entry=document.createElement("div"); entry.className="contents-entry";
      let expander=null;
      if(hasChildren){
        expander=document.createElement("button"); expander.type="button"; expander.className="contents-expander"; expander.textContent="▶"; expander.title="Expand"; expander.setAttribute("aria-label",`Expand ${row.title}`); expander.setAttribute("aria-expanded","false");
        entry.append(expander);
      }else{
        const spacer=document.createElement("span"); spacer.className="contents-spacer"; spacer.setAttribute("aria-hidden","true"); entry.append(spacer);
      }
      let toggle=()=>{};
      const activateRow=row.action!==null?()=>activate(ACTIONS[row.action]):hasChildren?()=>toggle():null;
      const button=makeRow(row.title,activateRow,row.action===null?"book":"");
      button.setAttribute("role","treeitem");
      if(hasChildren) button.setAttribute("aria-expanded","false");
      entry.append(button);
      node.append(entry);
      let children=null;
      if(hasChildren){
        children=document.createElement("div"); children.className="contents-children"; children.setAttribute("role","group"); children.hidden=true; node.append(children);
        toggle=()=>setContentsNodeExpanded(node,expander.getAttribute("aria-expanded")!=="true");
        expander.addEventListener("click",event=>{event.stopPropagation();toggle();});
      }
      parent.append(node);
      if(hasChildren) ancestors.push({level:row.level,children});
    });
    syncContentsSelection();
  }
  function renderContents(hierarchical=true){
    const host=byId("contents-list"); host.replaceChildren();
    if(hierarchical){ renderHierarchicalContents(host); return; }
    if(!ALL_TOPICS.length){ host.append(makeRow("No topics",null)); return; }
    ALL_TOPICS.forEach(row=>{
      const button=makeRow(row.title,()=>activate(ACTIONS[row.action]));
      const action=ACTIONS[row.action];
      if(action?.kind==="open"){ button.dataset.doc=String(action.doc); button.dataset.topic=String(action.topic); }
      host.append(button);
    });
    syncContentsSelection();
  }
  function renderIndex(){
    const host=byId("index-list"), q=byId("index-query").value.trim().toLowerCase(); host.replaceChildren();
    INDEX_ROWS.filter(row=>!q||row.keyword.toLowerCase().includes(q)).forEach(row=>{
      const label=row.actions.length>1?`${row.keyword}  (${row.actions.length} topics)`:row.keyword;
      host.append(makeRow(label,()=>row.actions.length===1?activate(ACTIONS[row.actions[0]]):showActionChoices(row.actions)));
    });
  }
  function renderSearch(){
    const host=byId("search-list"), q=byId("search-query").value.trim().toLowerCase(); host.replaceChildren(); if(!q) return;
    const hits=SEARCH_TOPICS.map(item=>{const title=item.title.toLowerCase(),text=item.text.toLowerCase();let score=0;if(title===q)score+=100;if(title.includes(q))score+=40;let pos=text.indexOf(q);if(pos>=0)score+=10;return {...item,score};}).filter(item=>item.score>0).sort((a,b)=>b.score-a.score||a.title.localeCompare(b.title)).slice(0,200);
    hits.forEach(item=>host.append(makeRow(item.title,()=>openMain(item.doc,item.topic))));
    if(!hits.length) host.append(makeRow("No matches",null));
  }
  function renderHistory(){ const host=byId("history-list"); host.replaceChildren(); state.history.forEach(loc=>host.append(makeRow(`${locationTitle(loc)} — ${DOCUMENTS[loc.doc].name}`,()=>openMain(loc.doc,loc.topic)))); }
  function renderBookmarks(){ const host=byId("bookmarks-list"); host.replaceChildren(); state.bookmarks.forEach((loc,index)=>host.append(makeRow(`${locationTitle(loc)} — ${DOCUMENTS[loc.doc].name}`,()=>openMain(loc.doc,loc.topic)))); if(!state.bookmarks.length)host.append(makeRow("No bookmarks",null)); }
  function addBookmark(){ if(!state.current)return; if(!state.bookmarks.some(item=>sameLocation(item,state.current))){state.bookmarks.push({...state.current});storeBookmarks(state.bookmarks);} renderBookmarks(); setStatus("Bookmark added"); }
  function removeBookmark(){ if(!state.bookmarks.length)return; state.bookmarks=state.bookmarks.filter(item=>!sameLocation(item,state.current)); storeBookmarks(state.bookmarks); renderBookmarks(); }

  function showActionChoices(actionIds){
    choiceList.replaceChildren(); actionIds.forEach(id=>{const action=ACTIONS[id]; if(action?.kind!=="open")return; choiceList.append(makeRow(locationTitle({doc:action.doc,topic:action.topic}),()=>{choiceShade.hidden=true;activate(action);}));}); choiceShade.hidden=false;
  }
  function showALink(topics){
    if(!Array.isArray(topics)||!topics.length){setStatus("No related topics found");return;}
    if(topics.length===1){openMain(topics[0].doc,topics[0].topic);return;}
    choiceList.replaceChildren(); topics.forEach(loc=>choiceList.append(makeRow(locationTitle(loc),()=>{choiceShade.hidden=true;openMain(loc.doc,loc.topic);}))); choiceShade.hidden=false;
  }

  document.addEventListener("click",event=>{
    const target=event.target.closest?.("[data-action]"); if(target){event.preventDefault(); activate(ACTIONS[Number(target.dataset.action)],event);}
    if(!popup.hidden && !popup.contains(event.target) && !target) closePopup();
  });
  document.addEventListener("keydown",event=>{
    const target=event.target.closest?.("[data-action]");
    if(target&&(event.key==="Enter"||event.key===" ")){event.preventDefault();activate(ACTIONS[Number(target.dataset.action)],event);return;}
    if(event.key==="Escape"){closePopup();closeSecondary();choiceShade.hidden=true;return;}
    if(event.altKey&&!event.ctrlKey&&!event.shiftKey&&event.key==="ArrowLeft"){event.preventDefault();goBack();return;}
    if(event.altKey&&!event.ctrlKey&&!event.shiftKey&&event.key==="ArrowRight"){event.preventDefault();goForward();return;}
    if(!event.altKey&&!event.ctrlKey&&!event.shiftKey&&event.key==="ArrowLeft"&&!event.target.matches?.("input,textarea")){event.preventDefault();physical(-1);return;}
    if(!event.altKey&&!event.ctrlKey&&!event.shiftKey&&event.key==="ArrowRight"&&!event.target.matches?.("input,textarea")){event.preventDefault();physical(1);return;}
    if(event.key==="F9"){event.preventDefault();workspace.classList.toggle("navigation-hidden");}
  });

  byId("back-button").addEventListener("click",goBack); byId("forward-button").addEventListener("click",goForward);
  byId("previous-button").addEventListener("click",()=>physical(-1)); byId("next-button").addEventListener("click",()=>physical(1));
  byId("browse-previous-button").addEventListener("click",()=>browse(-1)); byId("browse-next-button").addEventListener("click",()=>browse(1));
  byId("navigation-toggle").addEventListener("click",()=>{workspace.classList.toggle("navigation-hidden");fitTopicWidth();});
  navSplitter.addEventListener("pointerdown",event=>{
    if(workspace.classList.contains("navigation-hidden")) return;
    const current=parseFloat(getComputedStyle(workspace).getPropertyValue("--nav-width"))||byId("navigation-pane").getBoundingClientRect().width||300;
    splitterDrag={pointer:event.pointerId,startX:event.clientX,startWidth:current};
    navSplitter.setPointerCapture?.(event.pointerId); event.preventDefault();
  });
  navSplitter.addEventListener("pointermove",event=>{ if(splitterDrag&&splitterDrag.pointer===event.pointerId)setNavigationWidth(splitterDrag.startWidth+event.clientX-splitterDrag.startX); });
  const stopSplitter=event=>{ if(splitterDrag&&(!event||splitterDrag.pointer===event.pointerId))splitterDrag=null; };
  navSplitter.addEventListener("pointerup",stopSplitter); navSplitter.addEventListener("pointercancel",stopSplitter);
  navSplitter.addEventListener("keydown",event=>{ if(event.key==="ArrowLeft"||event.key==="ArrowRight"){ const current=byId("navigation-pane").getBoundingClientRect().width; setNavigationWidth(current+(event.key==="ArrowRight"?20:-20)); event.preventDefault(); } });
  byId("zoom-out-button").addEventListener("click",()=>adjustZoom(-10)); byId("zoom-in-button").addEventListener("click",()=>adjustZoom(10));
  byId("print-button").addEventListener("click",()=>window.print());
  document.querySelectorAll(".tab").forEach(tab=>tab.addEventListener("click",()=>showPane(tab.dataset.pane)));
  byId("contents-hierarchy").addEventListener("click",()=>{byId("contents-hierarchy").classList.add("active");byId("contents-all").classList.remove("active");renderContents(true);});
  byId("contents-all").addEventListener("click",()=>{byId("contents-all").classList.add("active");byId("contents-hierarchy").classList.remove("active");renderContents(false);});
  byId("index-query").addEventListener("input",renderIndex); byId("search-query").addEventListener("input",renderSearch);
  byId("bookmark-add").addEventListener("click",addBookmark); byId("bookmark-remove").addEventListener("click",removeBookmark);
  byId("popup").querySelector(".popup-close").addEventListener("click",closePopup); byId("secondary-close").addEventListener("click",closeSecondary); secondaryShade.addEventListener("click",event=>{if(event.target===secondaryShade)closeSecondary();});
  byId("choice-close").addEventListener("click",()=>choiceShade.hidden=true); choiceShade.addEventListener("click",event=>{if(event.target===choiceShade)choiceShade.hidden=true;});

  window.addEventListener("resize",fitTopicWidth);
  if(typeof ResizeObserver==="function"){
    try{ new ResizeObserver(()=>fitTopicWidth()).observe(byId("content-host")); }catch(_error){}
  }

  renderContents(true); renderIndex(); renderBookmarks(); renderHistory(); applyZoom();
  activate(ACTIONS[START_ACTION]);
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html_and_script_breakout_characters() {
        assert_eq!(html_escape("<&\"'>"), "&lt;&amp;&quot;&#39;&gt;");
        assert_eq!(js_string("</script>&"), "\"\\u003C/script\\u003E\\u0026\"");
    }

    #[test]
    fn base64_handles_partial_groups() {
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"Man"), "TWFu");
    }

    #[test]
    fn default_output_replaces_source_extension_with_html() {
        assert_eq!(
            default_output_path(std::path::Path::new("manual.hlp")),
            std::path::PathBuf::from("manual.html")
        );
        assert_eq!(
            default_output_path(std::path::Path::new("help/manual")),
            std::path::PathBuf::from("help/manual.html")
        );
    }

    #[test]
    fn export_reference_policy_blocks_absolute_paths() {
        assert!(automatic_export_reference_allowed("COMMON.HLP"));
        assert!(automatic_export_reference_allowed("sub/COMMON.HLP"));
        assert!(!automatic_export_reference_allowed("C:/HELP/COMMON.HLP"));
        assert!(!automatic_export_reference_allowed("//server/share/COMMON.HLP"));
        assert!(!automatic_export_reference_allowed("/tmp/COMMON.HLP"));
    }

    #[test]
    fn html_semantic_topics_do_not_use_retained_word_reflow() {
        assert!(EXPORT_CSS.contains(".semantic-region-inner"));
        assert!(EXPORT_CSS.contains(".hlp-paragraph"));
        assert!(EXPORT_CSS.contains("overflow-wrap:normal"));
        assert!(!EXPORT_JS.contains("scaleX("));
        assert!(!EXPORT_JS.contains("letterSpacing="));
        assert!(!EXPORT_JS.contains("function placeNaturalText"));
        assert!(!EXPORT_JS.contains("retainedBoxNumber"));
    }

    #[test]
    fn html_export_marks_decoded_topic_title_bold() {
        assert!(EXPORT_CSS.contains(".topic-heading .hlp-run{font-weight:700!important}"));
        assert!(EXPORT_CSS.contains(".topic-heading .hlp-run"));
    }

    #[test]
    fn html_hotspots_are_real_green_underlined_anchors() {
        assert!(EXPORT_CSS.contains("--hotspot:#008000"));
        assert!(EXPORT_CSS.contains(".hlp-link{color:var(--hotspot)!important;text-decoration:underline!important"));
    }

    #[test]
    fn html_related_topics_alink_dispatch_remains_live() {
        assert!(EXPORT_JS.contains("case \"alink\": showALink(op.topics)"));
        assert!(EXPORT_JS.contains("function showALink(topics)"));
        assert!(EXPORT_SHELL.contains("Select a related topic."));
    }
    #[test]
    fn semantic_metric_conversion_matches_reference_96_dpi_rule() {
        assert_eq!(semantic_raw_metric(72), 48);
        assert_eq!(semantic_raw_metric(-72), -48);
        assert_eq!(semantic_raw_metric(1), 0);
    }

    #[test]
    fn semantic_tabs_use_winhelp_half_inch_default() {
        let format = ParagraphFormat::default();
        let targets = semantic_tab_targets(&format, 2);
        assert_eq!(targets[0].0, 48);
        assert_eq!(targets[1].0, 96);
    }

    #[test]
    fn semantic_tabs_select_first_custom_stop_strictly_to_the_right() {
        let mut format = ParagraphFormat::default();
        format.tabs = vec![
            hlp::TabStop { position: 36, alignment: TabAlignment::Left },
            hlp::TabStop { position: 144, alignment: TabAlignment::Center },
            hlp::TabStop { position: 216, alignment: TabAlignment::Right },
        ];
        let targets = semantic_tab_targets(&format, 3);
        assert_eq!(targets[0], (24, TabAlignment::Left));
        assert_eq!(targets[1], (96, TabAlignment::Center));
        assert_eq!(targets[2], (144, TabAlignment::Right));
    }

    fn test_style() -> ResolvedTextStyle {
        ResolvedTextStyle {
            face_name: "Segoe UI".to_owned(),
            family: ResolvedFontFamily::Proportional,
            source_family: hlp::HlpFontFamily::Swiss,
            point_size: 12,
            point_size_twips: 240,
            weight: 400,
            italic: false,
            underline: false,
            strike_out: false,
            small_caps: false,
            foreground: Rgb { red: 0, green: 0, blue: 0 },
            foreground_inherits: true,
            background: Rgb { red: 255, green: 255, blue: 255 },
            background_inherits: true,
            charset: None,
        }
    }

    #[test]
    fn font_declarations_survive_the_html_style_attribute() {
        // A double-quoted family name terminated the attribute early and silently discarded every
        // later declaration, which is what dropped authored bold/italic/underline/size/colour.
        let family = html_font_family(&test_style());
        assert!(!family.contains('"'), "family {family} would close the style attribute");
        assert!(family.contains("'Segoe UI'"));
        let escaped = style_attribute(&semantic_text_style_css(&test_style(), "Dicas"));
        assert!(!escaped.contains('"'));
        assert!(escaped.contains("font-size:16.800px"));
    }

    #[test]
    fn exported_type_scale_puts_a_ten_point_font_on_the_base_size() {
        // 10 pt is 13.333 px under the 96 DPI reference rule; the export scales every authored size
        // by one factor so that size lands on EXPORT_BASE_FONT_PX with all ratios preserved.
        assert!((semantic_font_px(200) - EXPORT_BASE_FONT_PX).abs() < 0.001);
        assert!((semantic_font_px(400) - EXPORT_BASE_FONT_PX * 2.0).abs() < 0.001);
        assert!((semantic_font_px(100) - EXPORT_BASE_FONT_PX / 2.0).abs() < 0.001);
        // Paragraph geometry deliberately keeps the unscaled reference conversion.
        assert_eq!(semantic_raw_metric(72), 48);
    }

    #[test]
    fn semantic_runs_emit_authored_bold_italic_and_underline() {
        let mut style = test_style();
        style.weight = 700;
        style.italic = true;
        style.underline = true;
        let css = semantic_text_style_css(&style, "Dicas");
        assert!(css.contains("font-weight:700"));
        assert!(css.contains("font-style:italic"));
        assert!(css.contains("text-decoration:underline"));
        // Faces without a real bold/italic cut must still be synthesized rather than drawn plain.
        assert!(css.contains("font-synthesis:weight style small-caps"));

        let mut struck = test_style();
        struck.strike_out = true;
        struck.underline = true;
        assert!(semantic_text_style_css(&struck, "x").contains("text-decoration:underline line-through"));
        assert!(semantic_text_style_css(&test_style(), "x").contains("text-decoration:none"));
    }

    #[test]
    fn small_caps_scales_authored_capitals_and_shapes_lower_case() {
        let mut style = test_style();
        style.small_caps = true;
        // Authored capitals follow WinHlp32's 2/3 cell height: 240 twips -> 160 twips -> 11.2px.
        let capitals = semantic_text_style_css(&style, "NUM LOCK");
        assert!(capitals.contains("font-size:11.200px"), "{capitals}");
        assert!(capitals.contains("font-variant-caps:small-caps"));
        // Mixed-case runs keep the authored size and use real small-capital shaping instead.
        let mixed = semantic_text_style_css(&style, "Num Lock");
        assert!(mixed.contains("font-size:16.800px"), "{mixed}");
        assert!(mixed.contains("font-variant-caps:small-caps"));
        assert!(!semantic_text_style_css(&test_style(), "Num Lock").contains("font-variant-caps"));
    }

    #[test]
    fn paragraph_spacing_adds_instead_of_collapsing() {
        let mut flow = SemanticFlow::new();
        // WinHelp adds space-below to the next paragraph's space-above; sibling CSS margins would
        // instead collapse to max(12, 8) = 12 and pull the paragraphs together.
        assert_eq!(flow.advance(4, 12), 4);
        assert_eq!(flow.advance(8, 6), 20);
        assert_eq!(flow.pending_space_above, 6);
    }

    #[test]
    fn blank_authored_paragraphs_keep_a_line_box() {
        assert!(EXPORT_CSS.contains(".hlp-blank-paragraph::before{content:\"\\00a0\"}"));
    }

    #[test]
    fn standard_buttons_sit_below_the_text_baseline() {
        // Related Topics / ALink buttons are dropped by one tunable amount so the control lines up
        // with the rule its authoring paragraph draws beside it.
        assert!(EXPORT_CSS.contains("--control-drop:4px"));
        assert!(EXPORT_CSS.contains(".semantic-control{display:inline-block;vertical-align:calc(0px - var(--control-drop,4px))"));
        assert!(!EXPORT_CSS.contains(".semantic-control{display:inline-block;vertical-align:baseline"));
    }

    fn test_button() -> Inline {
        Inline::EmbeddedWindow(EmbeddedWindowReference {
            record_type: hlp::TopicRecordType::EmbeddedWindow,
            raw_prefix: [0; 6],
            descriptor: "!,ALink(topics)".to_owned(),
            payload_size: 0,
        })
    }

    fn test_text(text: &str) -> Inline {
        Inline::Text(TextRun { text: text.to_owned(), font_index: 0, hotspot: None })
    }

    fn test_picture() -> Inline {
        Inline::Picture(PictureReference {
            command: 0,
            position: PicturePosition::Inline,
            picture_type: 0x22,
            encoded_size: 0,
            hotspot_count: None,
            source: hlp::PictureSource::Unsupported(Vec::new()),
            image: None,
            hotspots: Vec::new(),
            decode_warning: None,
        })
    }

    #[test]
    fn hosted_controls_do_not_hang_in_the_paragraph_margin() {
        let paragraph = |inlines: Vec<Inline>| Paragraph {
            format: ParagraphFormat::default(),
            inlines,
        };
        // A leading control, with or without a preceding glyphless 0x85 marker.
        assert!(semantic_control_leads_paragraph(&paragraph(vec![
            test_button(),
            test_text(" Topicos relacionados"),
        ])));
        assert!(semantic_control_leads_paragraph(&paragraph(vec![
            Inline::Control85(0),
            test_button(),
        ])));
        // Authored bullet and list bitmaps remain real hanging markers.
        assert!(!semantic_control_leads_paragraph(&paragraph(vec![
            test_picture(),
            test_text(" bullet item"),
        ])));
        assert!(!semantic_control_leads_paragraph(&paragraph(vec![
            test_text("1"),
            Inline::Tab,
            test_text("numbered item"),
        ])));
        assert!(!semantic_control_leads_paragraph(&paragraph(Vec::new())));
    }

    #[test]
    fn exported_topic_surface_fits_the_visible_page() {
        // No frozen export-time width may survive in the shell, otherwise a resized window or a
        // changed text zoom clips the topic instead of re-wrapping it.
        assert!(!EXPORT_CSS.contains("min-width:var(--topic-width"));
        assert!(!EXPORT_CSS.contains("width:max-content;min-width:min(100%"));
        assert!(EXPORT_CSS.contains(".main-topic{width:100%;min-width:0;max-width:100%"));
        assert!(EXPORT_JS.contains("function fitTopicWidth()"));
        assert!(EXPORT_JS.contains("window.addEventListener(\"resize\",fitTopicWidth)"));
    }
}
