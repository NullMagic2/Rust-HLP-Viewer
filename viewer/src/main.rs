#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

//! Native wxDragon front end and integrated diagnostic entry point for Rust HLP Viewer 0.7.1.
//!
//! The process chooses its launch mode before wxWidgets is initialized. Diagnostic dumps and
//! `--export-html` therefore exercise the same Rust parser/exporter as the GUI without constructing
//! wxDragon or wxWidgets objects.

mod html_export;
mod support;

use support::{bookmarks as bookmark_store, cli, console, dump, recent};

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, CString};
use std::io::{self, Write};
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use hlp::{
    ContentsTarget, HelpDocument, HelpMacro, HelpMacroProgram, NavigationHistory, NavigationLocation,
    SafeHelpMacro, SearchMatchKind, resolve_external_help_path,
};
use hlp::{Hotspot, HotspotTarget, Rgb, TopicOffset, WindowDefinition};
use hlp::{
    LayoutBox, LayoutEngine, LayoutKind, Point as LayoutPoint, Rect as LayoutRect, RegionLayout,
    ResolvedFontFamily, ResolvedTextStyle, TextMetrics, TopicLayout,
};
use wxdragon::bitmap::Bitmap;
use wxdragon::dialogs::single_choice_dialog::SingleChoiceDialog;
use wxdragon::dialogs::text_entry_dialog::TextEntryDialog;
use wxdragon::event::{ButtonEvents, TextEvents, TreeEvents};
use wxdragon::dc::{BrushStyle, DeviceContext, PaintDC, PenStyle};
use wxdragon::font::{Font, FontFamily, FontStyle, FontWeight};
use wxdragon::geometry::Point as WxPoint;
use wxdragon::menus::menuitem::ItemKind;
use wxdragon::prelude::*;
use wxdragon::widgets::frame::FrameStyle;
use wxdragon::widgets::item_data::HasItemData;
use wxdragon::widgets::treectrl::{TreeCtrlStyle, TreeItemId};
use wxdragon::widgets::scrolled_window::ScrollBarConfig;
use wxdragon::widgets::splitter_window::{SplitterWindow, SplitterWindowStyle};
use wxdragon::widgets::toolbar::{ToolBar, ToolBarStyle};

#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::{HANDLE, HWND, POINT, RECT, SIZE};
#[cfg(target_os = "windows")]
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, GetDC, GetDeviceCaps, GetTextExtentPoint32W,
    GetTextMetricsW, InvalidateRect, ReleaseDC, SelectObject, SetBkMode, SetTextColor, TextOutW,
    DEFAULT_CHARSET, HDC, HGDIOBJ, LOGFONTW, LOGPIXELSX, LOGPIXELSY, TEXTMETRICW, TRANSPARENT,
};
#[cfg(target_os = "windows")]
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(target_os = "windows")]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, FindWindowExW, GetParent, GetSystemMetrics,
    GetWindowThreadProcessId, LoadImageW, SendMessageW, ICON_BIG, ICON_SMALL, IMAGE_ICON, SM_CXICON,
    SM_CXSMICON, SM_CYICON, SM_CYSMICON, WM_SETICON,
};

const ID_OPEN: i32 = 1001;
const ID_EXIT: i32 = 1002;
const ID_ABOUT: i32 = 1003;
const ID_PRINT: i32 = 1004;
const ID_COPY: i32 = 1005;
const ID_PASTE: i32 = 1006;
const ID_SELECT_ALL: i32 = 1007;
const ID_EXPORT_HTML: i32 = 1008;
const ID_RECENT_DOCUMENT_BASE: i32 = 1010;
const ID_BACK: i32 = 1101;
const ID_FORWARD: i32 = 1102;
const ID_CONTENTS: i32 = 1103;
const ID_PREVIOUS_TOPIC: i32 = 1104;
const ID_NEXT_TOPIC: i32 = 1105;
const ID_BROWSE_PREVIOUS: i32 = 1106;
const ID_BROWSE_NEXT: i32 = 1107;
const ID_ZOOM_OUT: i32 = 1108;
const ID_ZOOM_IN: i32 = 1109;
const ID_TOGGLE_NAVIGATION: i32 = 1201;
const ID_MACRO_DIAGNOSTICS: i32 = 1202;
const SCROLL_UNIT: i32 = 8;
const DEFAULT_TEXT_ZOOM_PERCENT: i32 = 110;
const TEXT_ZOOM_STEP_PERCENT: i32 = 10;
const MIN_TEXT_ZOOM_PERCENT: i32 = 70;
const MAX_TEXT_ZOOM_PERCENT: i32 = 200;
const HELP_BACKGROUND: Rgb = Rgb { red: 255, green: 255, blue: 228 };
const PRINT_PAGE_BACKGROUND: Rgb = Rgb { red: 255, green: 255, blue: 255 };
const TEXT_SELECTION_BACKGROUND: Rgb = Rgb { red: 0, green: 0, blue: 128 };
const TEXT_SELECTION_FOREGROUND: Rgb = Rgb { red: 255, green: 255, blue: 255 };
// WinHlp's information surface must remain visibly darker/yellower than the help page itself.
// Use the user-sampled WinHlp tooltip colour RGB(249,249,158) (#F9F99E). Keep one shared
// constant so native hover tips and legacy popup-note rendering cannot drift apart again.
const WINHELP_INFO_BACKGROUND: Rgb = Rgb { red: 249, green: 249, blue: 158 };
const WINHELP_TOOLTIP_BACKGROUND: Rgb = WINHELP_INFO_BACKGROUND;
const WINHELP_TOOLTIP_TEXT: Rgb = Rgb { red: 0, green: 0, blue: 0 };
const CONTENT_HOST_BACKGROUND: Rgb = Rgb { red: 212, green: 212, blue: 212 };
const NAVIGATION_PANE_WIDTH: i32 = 300;
const NAVIGATION_PANE_MIN_WIDTH: i32 = 180;
const POPUP_BACKGROUND: Rgb = WINHELP_INFO_BACKGROUND;
const CONTENT_FRAME_MARGIN: i32 = 12;
const CONTENT_FRAME_BORDER_THICKNESS: i32 = 1;
const MIN_LAYOUT_WIDTH: i32 = 320;
const POPUP_DEFAULT_WIDTH: i32 = 520;
const POPUP_MIN_HEIGHT: i32 = 100;
const POPUP_MAX_HEIGHT: i32 = 440;
const MAX_RELATED_HELP_FILES: usize = 32;
const MAX_MACRO_EXECUTION_STEPS: usize = 128;
const MAX_MACRO_DIAGNOSTICS: usize = 512;
const MAX_MACRO_DIAGNOSTIC_CHARS: usize = 2_048;
const VK_ESCAPE: i32 = 27;
// wxWidgets wxKeyCode values (not Win32 VK_* values).
const WXK_LEFT: i32 = 314;
const WXK_RIGHT: i32 = 316;

const WELCOME_TEXT: &str = "Rust HLP Viewer 0.7.1\n\nOpen a classic Windows .HLP file with File > Open (Ctrl+O). File > Export to HTML creates a self-contained interactive browser copy that preserves the retained formatting/navigation model. File > Print (Ctrl+P) prints the current topic, selected topic ranges, or all topics while retaining topic formatting.\n\nThe browsing strip provides Previous/Next topic navigation, authored browse buttons when present, the Navigation pane toggle, and text zoom. Back/Forward remain available from the Navigate menu and Alt+Left/Alt+Right. View > Navigation Pane (F9) shows or hides the Contents / Index / Search / Bookmarks / History side panel. Drag the divider beside the navigation pane to resize it.\n\nDrag across topic text to select it, then use Edit > Copy (Ctrl+C). Paste (Ctrl+V) inserts clipboard text into the focused Index or Search field.";

/// Native controls shared by the main-window event closures.
struct ViewerUi {
    frame: Frame,
    toolbar: ToolBar,
    body_splitter: SplitterWindow,
    navigation_column: Panel,
    content_column: Panel,
    browse_bar: Panel,
    browse_previous: Button,
    browse_next: Button,
    browse_prev_seq: Button,
    browse_next_seq: Button,
    browse_toggle_navigation: Button,
    browse_zoom_out: Button,
    browse_zoom_in: Button,
    navigation: Notebook,
    contents_hierarchical: Button,
    contents_show_all: Button,
    contents_tree: TreeCtrl,
    index_query: TextCtrl,
    index_list: ListBox,
    search_query: TextCtrl,
    search_list: ListBox,
    bookmark_add: Button,
    bookmark_remove: Button,
    bookmarks_list: ListBox,
    history_list: ListBox,
    content_host: Panel,
    page_border: Panel,
    page_inner: Panel,
    fixed_canvas: Panel,
    scrolled: ScrolledWindow,
    scrolling_canvas: Panel,
    status_bar: StatusBar,
}

#[derive(Debug, Clone)]
struct BookmarkEntry {
    label: String,
    location: NavigationLocation,
}

/// One keyword row merged across the current HLP and one-hop Contents-linked HLPs.
#[derive(Debug, Clone)]
struct PaneKeyword {
    keyword: String,
    locations: Vec<NavigationLocation>,
}

/// One ranked full-text result with a file-qualified destination.
#[derive(Debug, Clone)]
struct PaneSearchHit {
    location: NavigationLocation,
    title: String,
    score: u32,
    match_kind: SearchMatchKind,
}

/// Presentation mode for the Contents tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentsViewMode {
    /// Preserve the hierarchy authored in `.CNT` or recovered from a compiled `.GID` cache.
    Hierarchical,
    /// Deliberately flatten every decoded topic in physical topic order.
    AllTopics,
}

/// Retained topic region that currently owns a text selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TopicRegionKind {
    Fixed,
    Scrolling,
}

impl TopicRegionKind {
    fn from_fixed_region(fixed_region: bool) -> Self {
        if fixed_region { Self::Fixed } else { Self::Scrolling }
    }
}

/// Character-boundary position inside one retained text box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TopicTextPosition {
    box_index: usize,
    byte_offset: usize,
}

/// Mouse/keyboard text selection for one retained topic region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TopicTextSelection {
    region: TopicRegionKind,
    anchor: TopicTextPosition,
    focus: TopicTextPosition,
}

impl TopicTextSelection {
    fn ordered(self) -> (TopicTextPosition, TopicTextPosition) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    fn is_empty(self) -> bool {
        self.anchor == self.focus
    }
}

/// The surface that Edit menu commands should act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditTarget {
    Topic(TopicRegionKind),
    IndexQuery,
    SearchQuery,
}

/// Per-canvas mouse gesture state used to distinguish selection drags from hyperlink clicks.
#[derive(Debug, Clone, Default)]
struct TopicPointerState {
    anchor: Option<TopicTextPosition>,
    dragged: bool,
    pressed_hotspot: Option<(LayoutRect, Hotspot)>,
}

/// Mutable viewer state shared by wxDragon event closures on the GUI thread.
struct ViewerState {
    /// Document currently supplying the topic painted in the main window. Cross-document jumps may
    /// replace this without changing the navigation catalog anchored by `navigation_document`.
    document: Option<HelpDocument>,
    /// Document explicitly opened by the user (or at startup). Contents/Index/Search stay anchored
    /// to this root while cross-document topics are displayed in the main window.
    navigation_document: Option<HelpDocument>,
    topic_index: usize,
    layout: Option<TopicLayout>,
    layout_width: i32,
    history: NavigationHistory,
    related_documents: Vec<HelpDocument>,
    navigation_warnings: Vec<String>,
    contents_view_mode: ContentsViewMode,
    topic_selection: Option<TopicTextSelection>,
    edit_target: EditTarget,
    index_query: String,
    index_visible: Vec<PaneKeyword>,
    search_query: String,
    search_visible: Vec<PaneSearchHit>,
    bookmarks: Vec<BookmarkEntry>,
    history_visible: Vec<NavigationLocation>,
    macro_diagnostics: Vec<String>,
    macro_execution_depth: usize,
    macro_execution_budget: usize,
    popup_colors: BTreeMap<String, Rgb>,
    /// Currently open transient WinHelp popup owned by the main viewer, if any.
    /// wxDragon widgets use invalidating native handles, so retaining this lightweight handle is
    /// safe even when the native window is closed asynchronously.
    active_popup: Option<Frame>,
    /// Invalidates per-canvas hover-tooltip caches whenever a click/navigation dismisses them.
    tooltip_generation: u64,
    macro_browse_tools_added: bool,
    text_zoom_percent: i32,
    recent_documents: Vec<PathBuf>,
    navigation_pane_width: i32,
    navigation_pane_visible: bool,
}

impl Default for ViewerState {
    fn default() -> Self {
        Self {
            document: None,
            navigation_document: None,
            topic_index: 0,
            layout: None,
            layout_width: 0,
            history: NavigationHistory::default(),
            related_documents: Vec::new(),
            navigation_warnings: Vec::new(),
            contents_view_mode: ContentsViewMode::Hierarchical,
            topic_selection: None,
            edit_target: EditTarget::Topic(TopicRegionKind::Scrolling),
            index_query: String::new(),
            index_visible: Vec::new(),
            search_query: String::new(),
            search_visible: Vec::new(),
            bookmarks: Vec::new(),
            history_visible: Vec::new(),
            macro_diagnostics: Vec::new(),
            macro_execution_depth: 0,
            macro_execution_budget: MAX_MACRO_EXECUTION_STEPS,
            popup_colors: BTreeMap::new(),
            active_popup: None,
            tooltip_generation: 0,
            macro_browse_tools_added: false,
            text_zoom_percent: DEFAULT_TEXT_ZOOM_PERCENT,
            recent_documents: Vec::new(),
            navigation_pane_width: NAVIGATION_PANE_WIDTH,
            navigation_pane_visible: true,
        }
    }
}

/// Native auxiliary-window behavior. Popups are transient; secondary windows are persistent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuxiliaryKind {
    Popup,
    Secondary,
}

/// Native widgets owned by one popup or secondary help surface.
#[derive(Clone, Copy)]
struct AuxiliaryUi {
    frame: Frame,
    host: Panel,
    fixed_canvas: Panel,
    scrolled: ScrolledWindow,
    scrolling_canvas: Panel,
}

/// Mutable document/navigation state for a popup or secondary help surface.
struct AuxiliaryState {
    document: HelpDocument,
    topic_index: usize,
    definition: Option<WindowDefinition>,
    kind: AuxiliaryKind,
    layout: Option<TopicLayout>,
    layout_width: i32,
    activated_once: bool,
    /// Invalidates the native hover-tooltip cache after clicks or auxiliary navigation.
    tooltip_generation: u64,
    popup_color_override: Option<Rgb>,
    text_zoom_percent: i32,
    macro_navigation_generation: u64,
}

/// Selects diagnostic/GUI mode before wxDragon can create a native application object.
fn main() {
    match cli::parse() {
        Ok(cli::LaunchMode::Dump { file, verbose }) => {
            console::attach_parent_console();
            let stdout = io::stdout();
            let mut output = io::BufWriter::new(stdout.lock());
            if let Err(error) = dump::inspect(&file, verbose, &mut output) {
                let _ = writeln!(io::stderr().lock(), "{}: {error}", file.display());
                std::process::exit(1);
            }
            let _ = output.flush();
        }
        Ok(cli::LaunchMode::ExportHtml { source, target }) => {
            console::attach_parent_console();
            let target = target
                .unwrap_or_else(|| html_export::default_output_path(&source));
            match export_html_headless(&source, &target) {
                Ok(report) => {
                    println!("{}", report.output_path.display());
                    if report.warning_count != 0 {
                        eprintln!(
                            "warning: HTML export completed with {} unresolved or unavailable linked item(s)",
                            report.warning_count
                        );
                    }
                    let _ = io::stdout().flush();
                    let _ = io::stderr().flush();
                }
                Err(error) => {
                    eprintln!("{}: {error}", source.display());
                    std::process::exit(1);
                }
            }
        }
        Ok(cli::LaunchMode::Help) => {
            console::attach_parent_console();
            print!("{}", cli::usage());
            let _ = io::stdout().flush();
        }
        Ok(cli::LaunchMode::Version) => {
            console::attach_parent_console();
            println!("hlp-viewer {}", env!("CARGO_PKG_VERSION"));
            let _ = io::stdout().flush();
        }
        Ok(cli::LaunchMode::Gui { initial_file }) => run_gui(initial_file),
        Err(error) => {
            console::attach_parent_console();
            eprintln!("error: {error}\n\n{}", cli::usage());
            std::process::exit(2);
        }
    }
}

/// Starts wxWidgets through wxDragon only for interactive viewer mode.
fn run_gui(initial_file: Option<PathBuf>) {
    SystemOptions::set_option_by_int("msw.no-manifest-check", 1);
    let _ = wxdragon::main(move |_app| build_main_window(initial_file.clone()));
}

/// Builds the native frame, menu bar, status bar, fixed topic region, and scrolling topic body.
fn install_main_menu_bar(frame: &Frame, recent_documents: &[PathBuf]) {
    let file_menu = Menu::builder()
        .append_item(ID_OPEN, "&Open...\tCtrl+O", "Open a Windows Help file")
        .build();

    let mut recent_builder = Menu::builder();
    for (index, path) in recent_documents
        .iter()
        .take(recent::MAX_RECENT_DOCUMENTS)
        .enumerate()
    {
        let label = format!("&{} {}", index + 1, recent_menu_path_label(path));
        recent_builder = recent_builder.append_item(
            ID_RECENT_DOCUMENT_BASE + i32::try_from(index).unwrap_or(0),
            &label,
            "Open this recent Windows Help file",
        );
    }
    let recent_menu = recent_builder.build();
    let _ = file_menu.append_submenu(
        recent_menu,
        "Recent &Documents",
        "Open a recently viewed Windows Help file",
    );
    let _ = file_menu.append_separator();
    let _ = file_menu.append(
        ID_EXPORT_HTML,
        "Export to &HTML...",
        "Export this help system as a self-contained interactive HTML file",
        ItemKind::Normal,
    );
    let _ = file_menu.append(
        ID_PRINT,
        "&Print...\tCtrl+P",
        "Print the current topic, a topic range, or all topics",
        ItemKind::Normal,
    );
    let _ = file_menu.append_separator();
    let _ = file_menu.append(
        ID_EXIT,
        "E&xit\tAlt+F4",
        "Close Rust HLP Viewer",
        ItemKind::Normal,
    );

    let edit_menu = Menu::builder()
        .append_item(ID_COPY, "&Copy\tCtrl+C", "Copy the selected topic or query text")
        .append_item(ID_PASTE, "&Paste\tCtrl+V", "Paste clipboard text into the focused Index or Search field")
        .append_separator()
        .append_item(ID_SELECT_ALL, "Select &All\tCtrl+A", "Select all text in the active topic region or query field")
        .build();

    let navigate_menu = Menu::builder()
        .append_item(ID_BACK, "&Back\tAlt+Left", "Return to the previous navigation location")
        .append_item(ID_FORWARD, "&Forward\tAlt+Right", "Return to the next navigation location")
        .append_separator()
        .append_item(ID_CONTENTS, "&Contents\tCtrl+Home", "Open the help file's contents topic")
        .append_separator()
        .append_item(ID_PREVIOUS_TOPIC, "Previous &Topic\tLeft", "Open the physically previous decoded topic")
        .append_item(ID_NEXT_TOPIC, "&Next Topic\tRight", "Open the physically next decoded topic")
        .build();
    let view_menu = Menu::builder()
        .append_item(
            ID_TOGGLE_NAVIGATION,
            "&Navigation Pane\tF9",
            "Show or hide the Contents / Index / Search / Bookmarks / History side panel",
        )
        .append_separator()
        .append_item(
            ID_MACRO_DIAGNOSTICS,
            "WinHelp &Macro Diagnostics...",
            "Show allowed, blocked, and malformed WinHelp macro activity",
        )
        .build();
    let help_menu = Menu::builder()
        .append_item(ID_ABOUT, "&About", "About Rust HLP Viewer")
        .build();
    let menu_bar = MenuBar::builder()
        .append(file_menu, "&File")
        .append(edit_menu, "&Edit")
        .append(navigate_menu, "&Navigate")
        .append(view_menu, "&View")
        .append(help_menu, "&Help")
        .build();
    frame.set_menu_bar(menu_bar);
}

fn recent_menu_path_label(path: &Path) -> String {
    path.to_string_lossy()
        .replace('&', "&&")
        .replace('\r', " ")
        .replace('\n', " ")
        .replace('\t', " ")
}

fn recent_document_index(id: i32) -> Option<usize> {
    let offset = id.checked_sub(ID_RECENT_DOCUMENT_BASE)?;
    let index = usize::try_from(offset).ok()?;
    (index < recent::MAX_RECENT_DOCUMENTS).then_some(index)
}

fn open_recent_document(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    index: usize,
) {
    let path = state.borrow().recent_documents.get(index).cloned();
    if let Some(path) = path {
        load_document(ui, state, &path, true);
    }
}

fn remember_recent_document(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    path: &Path,
) {
    let path = recent::absolute_path(path);
    let recent_documents = {
        let mut state = state.borrow_mut();
        recent::record(&mut state.recent_documents, path);
        state.recent_documents.clone()
    };
    install_main_menu_bar(&ui.frame, &recent_documents);
    if let Err(error) = recent::save(&recent_documents) {
        ui.status_bar.set_status_text(
            &format!("HLP opened; recent-document config could not be saved: {error}"),
            0,
        );
    }
}


/// Assigns the embedded application icon to the native top-level frame on Windows.
///
/// Build-fix 38 embedded `viewer/assets/hlp.ico` into the executable as Win32 icon resource 1,
/// which gives Explorer the correct application icon. wxWidgets still creates its own top-level
/// window class, so Windows can show the toolkit's generic frame icon unless the HWND receives
/// explicit `WM_SETICON` messages. Load both system large/small sizes from the same embedded icon
/// group so the taskbar/Alt-Tab and title-bar caption use the project artwork without requiring an
/// external icon file beside the executable. The two icon handles intentionally remain alive for
/// the lifetime of this single top-level frame; the operating system reclaims them at process exit.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn set_frame_application_icon(frame: &Frame) {
    const ICON_RESOURCE_ID: usize = 1;

    let hwnd = frame.get_handle() as HWND;
    if hwnd.is_null() {
        return;
    }

    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    if module.is_null() {
        return;
    }

    let resource_name = ICON_RESOURCE_ID as *const u16;
    let large_width = unsafe { GetSystemMetrics(SM_CXICON) };
    let large_height = unsafe { GetSystemMetrics(SM_CYICON) };
    let small_width = unsafe { GetSystemMetrics(SM_CXSMICON) };
    let small_height = unsafe { GetSystemMetrics(SM_CYSMICON) };

    let large_icon = unsafe {
        LoadImageW(
            module,
            resource_name,
            IMAGE_ICON,
            large_width,
            large_height,
            0,
        )
    };
    let small_icon = unsafe {
        LoadImageW(
            module,
            resource_name,
            IMAGE_ICON,
            small_width,
            small_height,
            0,
        )
    };

    if !large_icon.is_null() {
        unsafe {
            SendMessageW(
                hwnd,
                WM_SETICON,
                ICON_BIG as usize,
                large_icon as isize,
            );
        }
    }
    if !small_icon.is_null() {
        unsafe {
            SendMessageW(
                hwnd,
                WM_SETICON,
                ICON_SMALL as usize,
                small_icon as isize,
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn set_frame_application_icon(_frame: &Frame) {}

#[cfg(target_os = "windows")]
const TOOLTIP_CLASS: [u16; 17] = [
    b't' as u16, b'o' as u16, b'o' as u16, b'l' as u16, b't' as u16, b'i' as u16,
    b'p' as u16, b's' as u16, b'_' as u16, b'c' as u16, b'l' as u16, b'a' as u16,
    b's' as u16, b's' as u16, b'3' as u16, b'2' as u16, 0,
];

/// Applies the requested WinHelp information palette to wxWidgets' already-created native tooltip
/// controls. This remains useful for ordinary toolbar/list tooltips; HLP hotspot previews use the
/// dedicated pre-created control below so their first visible frame is already the requested colour.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn apply_windows_tooltip_palette(owner_handle: *mut std::ffi::c_void) -> bool {
    const TTM_SETTIPBKCOLOR: u32 = 0x0400 + 19;
    const TTM_SETTIPTEXTCOLOR: u32 = 0x0400 + 20;
    const TTM_SETWINDOWTHEME: u32 = 0x2000 + 11;
    const EMPTY_THEME: [u16; 1] = [0];

    let owner = owner_handle as HWND;
    if owner.is_null() {
        return false;
    }
    let owner_thread = unsafe { GetWindowThreadProcessId(owner, std::ptr::null_mut()) };
    if owner_thread == 0 {
        return false;
    }

    let mut applied = false;
    let mut after: HWND = std::ptr::null_mut();
    loop {
        let tooltip = unsafe {
            FindWindowExW(
                std::ptr::null_mut(),
                after,
                TOOLTIP_CLASS.as_ptr(),
                std::ptr::null(),
            )
        };
        if tooltip.is_null() {
            break;
        }
        after = tooltip;
        if unsafe { GetWindowThreadProcessId(tooltip, std::ptr::null_mut()) } != owner_thread {
            continue;
        }
        unsafe {
            SendMessageW(
                tooltip,
                TTM_SETWINDOWTHEME,
                0,
                EMPTY_THEME.as_ptr() as isize,
            );
            SendMessageW(
                tooltip,
                TTM_SETTIPBKCOLOR,
                colorref_from_rgb(WINHELP_TOOLTIP_BACKGROUND) as usize,
                0,
            );
            SendMessageW(
                tooltip,
                TTM_SETTIPTEXTCOLOR,
                colorref_from_rgb(WINHELP_TOOLTIP_TEXT) as usize,
                0,
            );
        }
        applied = true;
    }
    applied
}

#[cfg(not(target_os = "windows"))]
fn apply_windows_tooltip_palette(_owner_handle: *mut std::ffi::c_void) -> bool {
    false
}

/// Minimal TOOLINFOW-compatible layout used by the dedicated Windows hotspot tooltip. Keeping the
/// definition local avoids making the portable wxDragon front end depend on the broader common-
/// controls bindings solely for one control structure.
#[cfg(target_os = "windows")]
#[repr(C)]
struct NativeToolInfoW {
    cb_size: u32,
    u_flags: u32,
    hwnd: HWND,
    u_id: usize,
    rect: RECT,
    hinst: *mut std::ffi::c_void,
    lpsz_text: *mut u16,
    l_param: isize,
    lp_reserved: *mut std::ffi::c_void,
}

/// One pre-created classic tooltip control attached to a topic canvas.
///
/// Unlike wxToolTip's lazy HWND creation, this control is constructed and coloured while hidden,
/// before the pointer can ever trigger it. Therefore Windows never paints a default InfoWindow-
/// coloured frame that must be recoloured a few milliseconds later.
#[cfg(target_os = "windows")]
struct NativeHotspotTooltip {
    hwnd: HWND,
    owner_hwnd: HWND,
    tool_hwnd: HWND,
    hinst: *mut std::ffi::c_void,
    text: Vec<u16>,
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
impl NativeHotspotTooltip {
    fn new(tool_handle: *mut std::ffi::c_void) -> Option<Self> {
        const WS_EX_TOPMOST: u32 = 0x0000_0008;
        const WS_POPUP: u32 = 0x8000_0000;
        const TTS_ALWAYSTIP: u32 = 0x01;
        const TTS_NOPREFIX: u32 = 0x02;
        const TTF_IDISHWND: u32 = 0x0001;
        const TTF_SUBCLASS: u32 = 0x0010;
        const TTM_ADDTOOLW: u32 = 0x0400 + 50;
        const TTM_SETMAXTIPWIDTH: u32 = 0x0400 + 24;
        const TTM_SETTIPBKCOLOR: u32 = 0x0400 + 19;
        const TTM_SETTIPTEXTCOLOR: u32 = 0x0400 + 20;
        const TTM_SETWINDOWTHEME: u32 = 0x2000 + 11;
        const EMPTY_THEME: [u16; 1] = [0];

        let tool_hwnd = tool_handle as HWND;
        if tool_hwnd.is_null() {
            return None;
        }
        let parent = unsafe { GetParent(tool_hwnd) };
        let owner_hwnd = if parent.is_null() { tool_hwnd } else { parent };
        let module = unsafe { GetModuleHandleW(std::ptr::null()) };
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST,
                TOOLTIP_CLASS.as_ptr(),
                std::ptr::null(),
                WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
                0,
                0,
                0,
                0,
                owner_hwnd,
                std::ptr::null_mut(),
                module,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return None;
        }

        // Style the invisible HWND before registering its tool. There is no default-colour frame to
        // flash because the common control has not yet been given any hover target or text.
        unsafe {
            SendMessageW(
                hwnd,
                TTM_SETWINDOWTHEME,
                0,
                EMPTY_THEME.as_ptr() as isize,
            );
            SendMessageW(
                hwnd,
                TTM_SETTIPBKCOLOR,
                colorref_from_rgb(WINHELP_TOOLTIP_BACKGROUND) as usize,
                0,
            );
            SendMessageW(
                hwnd,
                TTM_SETTIPTEXTCOLOR,
                colorref_from_rgb(WINHELP_TOOLTIP_TEXT) as usize,
                0,
            );
            SendMessageW(hwnd, TTM_SETMAXTIPWIDTH, 0, 720);
        }

        let mut tooltip = Self {
            hwnd,
            owner_hwnd,
            tool_hwnd,
            hinst: module,
            text: vec![0],
        };
        let mut info = tooltip.tool_info(TTF_IDISHWND | TTF_SUBCLASS);
        if unsafe { SendMessageW(hwnd, TTM_ADDTOOLW, 0, (&mut info as *mut NativeToolInfoW) as isize) } == 0 {
            unsafe {
                DestroyWindow(hwnd);
            }
            return None;
        }
        Some(tooltip)
    }

    fn tool_info(&mut self, flags: u32) -> NativeToolInfoW {
        NativeToolInfoW {
            cb_size: std::mem::size_of::<NativeToolInfoW>() as u32,
            u_flags: flags,
            hwnd: self.owner_hwnd,
            u_id: self.tool_hwnd as usize,
            rect: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            hinst: self.hinst,
            lpsz_text: self.text.as_mut_ptr(),
            l_param: 0,
            lp_reserved: std::ptr::null_mut(),
        }
    }

    fn set_text(&mut self, text: Option<&str>) {
        const TTF_IDISHWND: u32 = 0x0001;
        const TTF_SUBCLASS: u32 = 0x0010;
        const TTM_POP: u32 = 0x0400 + 28;
        const TTM_UPDATETIPTEXTW: u32 = 0x0400 + 57;

        // Hide any old target first so moving directly between two links never leaves stale text on
        // screen. The replacement buffer remains owned by this object for as long as the native
        // tooltip may reference it.
        unsafe {
            SendMessageW(self.hwnd, TTM_POP, 0, 0);
        }
        self.text = text
            .filter(|value| !value.is_empty())
            .map_or_else(|| vec![0], |value| value.encode_utf16().chain(std::iter::once(0)).collect());
        let mut info = self.tool_info(TTF_IDISHWND | TTF_SUBCLASS);
        unsafe {
            SendMessageW(
                self.hwnd,
                TTM_UPDATETIPTEXTW,
                0,
                (&mut info as *mut NativeToolInfoW) as isize,
            );
        }
    }
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
impl Drop for NativeHotspotTooltip {
    fn drop(&mut self) {
        if !self.hwnd.is_null() {
            unsafe {
                DestroyWindow(self.hwnd);
            }
            self.hwnd = std::ptr::null_mut();
        }
    }
}


/// A viewer-owned overflow popup shared by the navigation item widgets on Windows.
///
/// This is intentionally not `wxToolTip` and not `tooltips_class32`. The previous common-control
/// implementations could reveal the complete label, but Windows still owned their final placement;
/// attempts to move that native tooltip after the fact remained timing-sensitive. This lightweight
/// popup is painted by the viewer itself, so its text origin can be placed exactly on top of the
/// cropped ListBox/TreeCtrl text origin supplied by the shared navigation binders.
#[cfg(target_os = "windows")]
const INLINE_OVERFLOW_TOOLTIP_CLASS: [u16; 23] = [
    'R' as u16,
    'u' as u16,
    's' as u16,
    't' as u16,
    'H' as u16,
    'l' as u16,
    'p' as u16,
    'O' as u16,
    'v' as u16,
    'e' as u16,
    'r' as u16,
    'f' as u16,
    'l' as u16,
    'o' as u16,
    'w' as u16,
    'T' as u16,
    'o' as u16,
    'o' as u16,
    'l' as u16,
    't' as u16,
    'i' as u16,
    'p' as u16,
    0,
];

#[cfg(target_os = "windows")]
const INLINE_OVERFLOW_TOOLTIP_PROP: [u16; 19] = [
    'R' as u16,
    'u' as u16,
    's' as u16,
    't' as u16,
    'H' as u16,
    'l' as u16,
    'p' as u16,
    'O' as u16,
    'v' as u16,
    'e' as u16,
    'r' as u16,
    'f' as u16,
    'l' as u16,
    'o' as u16,
    'w' as u16,
    'T' as u16,
    'i' as u16,
    'p' as u16,
    0,
];

#[cfg(target_os = "windows")]
const INLINE_OVERFLOW_SHOW_TIMER_ID: usize = 0x484C_5054;
#[cfg(target_os = "windows")]
const INLINE_OVERFLOW_INITIAL_DELAY_MS: u32 = 350;
#[cfg(target_os = "windows")]
const INLINE_OVERFLOW_BORDER: i32 = 1;
#[cfg(target_os = "windows")]
const INLINE_OVERFLOW_PADDING_X: i32 = 2;
#[cfg(target_os = "windows")]
const INLINE_OVERFLOW_PADDING_Y: i32 = 1;

#[cfg(target_os = "windows")]
#[repr(C)]
struct NativeWindowClassW {
    style: u32,
    wnd_proc: Option<unsafe extern "system" fn(HWND, u32, usize, isize) -> isize>,
    class_extra: i32,
    window_extra: i32,
    instance: *mut std::ffi::c_void,
    icon: *mut std::ffi::c_void,
    cursor: *mut std::ffi::c_void,
    background: *mut std::ffi::c_void,
    menu_name: *const u16,
    class_name: *const u16,
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct NativePaintStruct {
    hdc: HDC,
    erase: i32,
    paint: RECT,
    restore: i32,
    incremental_update: i32,
    reserved: [u8; 32],
}

#[cfg(target_os = "windows")]
struct NativeOverflowTooltipPaintState {
    text: Vec<u16>,
    font: HGDIOBJ,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    visible: bool,
    pending: bool,
}

#[cfg(target_os = "windows")]
static INLINE_OVERFLOW_TOOLTIP_CLASS_REGISTERED: std::sync::OnceLock<bool> =
    std::sync::OnceLock::new();

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn ensure_inline_overflow_tooltip_class() -> bool {
    *INLINE_OVERFLOW_TOOLTIP_CLASS_REGISTERED.get_or_init(|| {
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        if instance.is_null() {
            return false;
        }
        let class = NativeWindowClassW {
            style: 0,
            wnd_proc: Some(inline_overflow_tooltip_window_proc),
            class_extra: 0,
            window_extra: 0,
            instance,
            icon: std::ptr::null_mut(),
            cursor: std::ptr::null_mut(),
            background: std::ptr::null_mut(),
            menu_name: std::ptr::null(),
            class_name: INLINE_OVERFLOW_TOOLTIP_CLASS.as_ptr(),
        };
        unsafe { RegisterClassW(&class) != 0 }
    })
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
unsafe extern "system" fn inline_overflow_tooltip_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    const WM_PAINT: u32 = 0x000F;
    const WM_ERASEBKGND: u32 = 0x0014;
    const WM_MOUSEACTIVATE: u32 = 0x0021;
    const WM_NCHITTEST: u32 = 0x0084;
    const HTTRANSPARENT: isize = -1;
    const MA_NOACTIVATE: isize = 3;
    const HGDI_ERROR_VALUE: isize = -1;

    match message {
        // The popup visually covers the clipped row but must never become the mouse target. Returning
        // HTTRANSPARENT keeps hover/mouse-leave ownership with the underlying native navigation
        // control and removes the synthetic-leave problem that affected row-aligned tooltips.
        WM_NCHITTEST => return HTTRANSPARENT,
        WM_MOUSEACTIVATE => return MA_NOACTIVATE,
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            let mut paint = NativePaintStruct {
                hdc: std::ptr::null_mut(),
                erase: 0,
                paint: RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                restore: 0,
                incremental_update: 0,
                reserved: [0; 32],
            };
            let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
            if !hdc.is_null() {
                let mut client = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                unsafe {
                    GetClientRect(hwnd, &mut client);
                }

                let background = unsafe { CreateSolidBrush(colorref_from_rgb(WINHELP_TOOLTIP_BACKGROUND)) };
                if !background.is_null() {
                    unsafe {
                        FillRect(hdc, &client, background);
                        DeleteObject(background as HGDIOBJ);
                    }
                }

                let border = unsafe { CreateSolidBrush(colorref_from_rgb(WINHELP_TOOLTIP_TEXT)) };
                if !border.is_null() && client.right > client.left && client.bottom > client.top {
                    let top = RECT {
                        left: client.left,
                        top: client.top,
                        right: client.right,
                        bottom: client.top.saturating_add(INLINE_OVERFLOW_BORDER),
                    };
                    let bottom = RECT {
                        left: client.left,
                        top: client.bottom.saturating_sub(INLINE_OVERFLOW_BORDER),
                        right: client.right,
                        bottom: client.bottom,
                    };
                    let left = RECT {
                        left: client.left,
                        top: client.top,
                        right: client.left.saturating_add(INLINE_OVERFLOW_BORDER),
                        bottom: client.bottom,
                    };
                    let right = RECT {
                        left: client.right.saturating_sub(INLINE_OVERFLOW_BORDER),
                        top: client.top,
                        right: client.right,
                        bottom: client.bottom,
                    };
                    unsafe {
                        FillRect(hdc, &top, border);
                        FillRect(hdc, &bottom, border);
                        FillRect(hdc, &left, border);
                        FillRect(hdc, &right, border);
                        DeleteObject(border as HGDIOBJ);
                    }
                } else if !border.is_null() {
                    unsafe {
                        DeleteObject(border as HGDIOBJ);
                    }
                }

                let state_ptr = unsafe {
                    GetPropW(hwnd, INLINE_OVERFLOW_TOOLTIP_PROP.as_ptr())
                        as *const NativeOverflowTooltipPaintState
                };
                if !state_ptr.is_null() {
                    let state = unsafe { &*state_ptr };
                    if !state.text.is_empty() {
                        let old_font = if state.font.is_null() {
                            std::ptr::null_mut()
                        } else {
                            unsafe { SelectObject(hdc, state.font) }
                        };
                        unsafe {
                            SetBkMode(hdc, TRANSPARENT as i32);
                            SetTextColor(hdc, colorref_from_rgb(WINHELP_TOOLTIP_TEXT));
                        }
                        let count = i32::try_from(state.text.len()).unwrap_or(i32::MAX);
                        unsafe {
                            TextOutW(
                                hdc,
                                INLINE_OVERFLOW_BORDER.saturating_add(INLINE_OVERFLOW_PADDING_X),
                                INLINE_OVERFLOW_BORDER.saturating_add(INLINE_OVERFLOW_PADDING_Y),
                                state.text.as_ptr(),
                                count,
                            );
                        }
                        if !old_font.is_null() && old_font as isize != HGDI_ERROR_VALUE {
                            unsafe {
                                SelectObject(hdc, old_font);
                            }
                        }
                    }
                }
            }
            unsafe {
                EndPaint(hwnd, &paint);
            }
            return 0;
        }
        _ => {}
    }

    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
unsafe extern "system" fn inline_overflow_show_timer_proc(
    hwnd: HWND,
    _message: u32,
    timer_id: usize,
    _time: u32,
) {
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_SHOWWINDOW: u32 = 0x0040;

    unsafe {
        KillTimer(hwnd, timer_id);
    }
    let state_ptr = unsafe {
        GetPropW(hwnd, INLINE_OVERFLOW_TOOLTIP_PROP.as_ptr()) as *mut NativeOverflowTooltipPaintState
    };
    if state_ptr.is_null() {
        return;
    }
    let state = unsafe { &mut *state_ptr };
    state.pending = false;
    if state.text.is_empty() {
        return;
    }
    state.visible = true;
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            state.x,
            state.y,
            state.width,
            state.height,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        InvalidateRect(hwnd, std::ptr::null(), 0);
        UpdateWindow(hwnd);
    }
}

#[cfg(target_os = "windows")]
struct NativeInlineOverflowTooltip {
    hwnd: HWND,
    tool_hwnd: HWND,
    state: Box<NativeOverflowTooltipPaintState>,
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
impl NativeInlineOverflowTooltip {
    fn new(tool_handle: *mut std::ffi::c_void) -> Option<Self> {
        const WS_EX_TOPMOST: u32 = 0x0000_0008;
        const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
        const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
        const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
        const WS_POPUP: u32 = 0x8000_0000;
        const WM_GETFONT: u32 = 0x0031;

        if !ensure_inline_overflow_tooltip_class() {
            return None;
        }
        let tool_hwnd = tool_handle as HWND;
        if tool_hwnd.is_null() {
            return None;
        }
        let parent = unsafe { GetParent(tool_hwnd) };
        let owner_hwnd = if parent.is_null() { tool_hwnd } else { parent };
        let module = unsafe { GetModuleHandleW(std::ptr::null()) };
        let font = unsafe { SendMessageW(tool_hwnd, WM_GETFONT, 0, 0) } as HGDIOBJ;
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                INLINE_OVERFLOW_TOOLTIP_CLASS.as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                owner_hwnd,
                std::ptr::null_mut(),
                module,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return None;
        }

        let mut state = Box::new(NativeOverflowTooltipPaintState {
            text: Vec::new(),
            font,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            visible: false,
            pending: false,
        });
        let state_ptr = (&mut *state as *mut NativeOverflowTooltipPaintState) as *mut std::ffi::c_void;
        if unsafe { SetPropW(hwnd, INLINE_OVERFLOW_TOOLTIP_PROP.as_ptr(), state_ptr) } == 0 {
            unsafe {
                DestroyWindow(hwnd);
            }
            return None;
        }

        Some(Self {
            hwnd,
            tool_hwnd,
            state,
        })
    }

    fn hide(&mut self) {
        const SWP_NOSIZE: u32 = 0x0001;
        const SWP_NOMOVE: u32 = 0x0002;
        const SWP_NOZORDER: u32 = 0x0004;
        const SWP_NOACTIVATE: u32 = 0x0010;
        const SWP_HIDEWINDOW: u32 = 0x0080;

        self.state.text.clear();
        self.state.visible = false;
        self.state.pending = false;
        unsafe {
            KillTimer(self.hwnd, INLINE_OVERFLOW_SHOW_TIMER_ID);
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_HIDEWINDOW,
            );
        }
    }

    fn show_at(&mut self, text: &str, text_origin: WxPoint) {
        const SWP_NOZORDER: u32 = 0x0004;
        const SWP_NOACTIVATE: u32 = 0x0010;
        const SWP_SHOWWINDOW: u32 = 0x0040;

        if text.is_empty() {
            self.hide();
            return;
        }

        let next_text = text.encode_utf16().collect::<Vec<_>>();
        let text_changed = self.state.text != next_text;
        self.state.text = next_text;
        let extent = native_control_text_extent(self.tool_hwnd as *mut std::ffi::c_void, text)
            .unwrap_or_else(|| SIZE {
                cx: i32::try_from(text.encode_utf16().count())
                    .unwrap_or(i32::MAX / 8)
                    .saturating_mul(8)
                    .max(1),
                cy: 16,
            });
        let text_offset_x = INLINE_OVERFLOW_BORDER.saturating_add(INLINE_OVERFLOW_PADDING_X);
        let text_offset_y = INLINE_OVERFLOW_BORDER.saturating_add(INLINE_OVERFLOW_PADDING_Y);
        self.state.x = text_origin.x.saturating_sub(text_offset_x);
        self.state.y = text_origin.y.saturating_sub(text_offset_y);
        self.state.width = extent
            .cx
            .max(1)
            .saturating_add(text_offset_x.saturating_mul(2));
        self.state.height = extent
            .cy
            .max(1)
            .saturating_add(text_offset_y.saturating_mul(2));

        if self.state.visible {
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    std::ptr::null_mut(),
                    self.state.x,
                    self.state.y,
                    self.state.width,
                    self.state.height,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
                InvalidateRect(self.hwnd, std::ptr::null(), 0);
                UpdateWindow(self.hwnd);
            }
            return;
        }

        // Keep the familiar native hover delay, but unlike the old common-control approach the timer
        // only controls *when* the popup becomes visible. Its position and painting are fully ours.
        if self.state.pending && !text_changed {
            return;
        }
        unsafe {
            KillTimer(self.hwnd, INLINE_OVERFLOW_SHOW_TIMER_ID);
        }
        self.state.pending = true;
        let timer = unsafe {
            SetTimer(
                self.hwnd,
                INLINE_OVERFLOW_SHOW_TIMER_ID,
                INLINE_OVERFLOW_INITIAL_DELAY_MS,
                Some(inline_overflow_show_timer_proc),
            )
        };
        if timer == 0 {
            self.state.pending = false;
            self.state.visible = true;
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    std::ptr::null_mut(),
                    self.state.x,
                    self.state.y,
                    self.state.width,
                    self.state.height,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
                InvalidateRect(self.hwnd, std::ptr::null(), 0);
                UpdateWindow(self.hwnd);
            }
        }
    }
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
impl Drop for NativeInlineOverflowTooltip {
    fn drop(&mut self) {
        if !self.hwnd.is_null() {
            unsafe {
                KillTimer(self.hwnd, INLINE_OVERFLOW_SHOW_TIMER_ID);
                RemovePropW(self.hwnd, INLINE_OVERFLOW_TOOLTIP_PROP.as_ptr());
                DestroyWindow(self.hwnd);
            }
            self.hwnd = std::ptr::null_mut();
        }
    }
}

/// Shared state driver for Contents, Index, Search, Bookmarks, and History overflow reveals.
///
/// It caches both the current text and its requested screen-space origin so moving between rows or
/// scrolling a row under the pointer also moves the reveal. Windows uses the viewer-owned custom
/// popup above; if creating that popup fails, the same driver falls back to wxWindow::set_tooltip.
struct OverflowTooltip {
    current_text: Option<String>,
    current_anchor: Option<WxPoint>,
    fallback: Box<dyn Fn(&str)>,
    #[cfg(target_os = "windows")]
    native: Option<NativeInlineOverflowTooltip>,
    #[cfg(target_os = "windows")]
    fallback_owner: *mut std::ffi::c_void,
}

impl OverflowTooltip {
    fn new<F>(tool_handle: *mut std::ffi::c_void, fallback: F) -> Self
    where
        F: Fn(&str) + 'static,
    {
        #[cfg(not(target_os = "windows"))]
        let _ = tool_handle;

        Self {
            current_text: None,
            current_anchor: None,
            fallback: Box::new(fallback),
            #[cfg(target_os = "windows")]
            native: NativeInlineOverflowTooltip::new(tool_handle),
            #[cfg(target_os = "windows")]
            fallback_owner: tool_handle,
        }
    }

    fn set_reveal(&mut self, reveal: Option<(String, WxPoint)>) {
        let (text, anchor) = reveal
            .filter(|(value, _)| !value.is_empty())
            .map_or((None, None), |(value, anchor)| (Some(value), Some(anchor)));
        let text_changed = self.current_text.as_deref() != text.as_deref();
        let anchor_changed = match (self.current_anchor, anchor) {
            (Some(current), Some(next)) => current.x != next.x || current.y != next.y,
            (None, None) => false,
            _ => true,
        };
        if !text_changed && !anchor_changed {
            return;
        }
        self.current_text = text;
        self.current_anchor = anchor;

        #[cfg(target_os = "windows")]
        if let Some(native) = self.native.as_mut() {
            match (self.current_text.as_deref(), self.current_anchor) {
                (Some(text), Some(anchor)) => native.show_at(text, anchor),
                _ => native.hide(),
            }
            return;
        }

        if text_changed {
            (self.fallback)(self.current_text.as_deref().unwrap_or(""));
            #[cfg(target_os = "windows")]
            if self.current_text.is_some() {
                // The custom popup is the normal Windows path. Keep the existing wxToolTip palette
                // fix only for the rare native-window-creation fallback.
                apply_windows_tooltip_palette(self.fallback_owner);
            }
        }
    }
}

/// Measures one native control label with the font actually selected by that control.
///
/// wxDragon's CString bridge can under-measure non-ASCII navigation labels. The native path avoids
/// that bridge by asking the control for WM_GETFONT and measuring UTF-16 text directly with GDI.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn native_control_text_extent(handle: *mut std::ffi::c_void, text: &str) -> Option<SIZE> {
    const WM_GETFONT: u32 = 0x0031;
    const HGDI_ERROR_VALUE: isize = -1;

    if text.is_empty() {
        return Some(SIZE { cx: 0, cy: 0 });
    }
    let hwnd = handle as HWND;
    if hwnd.is_null() {
        return None;
    }
    let wide = text.encode_utf16().collect::<Vec<_>>();
    let count = i32::try_from(wide.len()).ok()?;
    let font: HGDIOBJ = unsafe { SendMessageW(hwnd, WM_GETFONT, 0, 0) } as HGDIOBJ;
    if font.is_null() {
        return None;
    }
    let hdc = unsafe { GetDC(hwnd) };
    if hdc.is_null() {
        return None;
    }

    let old_font: HGDIOBJ = unsafe { SelectObject(hdc, font) };
    if old_font.is_null() || old_font as isize == HGDI_ERROR_VALUE {
        unsafe {
            ReleaseDC(hwnd, hdc);
        }
        return None;
    }

    let mut size = SIZE { cx: 0, cy: 0 };
    let measured = unsafe { GetTextExtentPoint32W(hdc, wide.as_ptr(), count, &mut size) != 0 };
    unsafe {
        SelectObject(hdc, old_font);
        ReleaseDC(hwnd, hdc);
    }
    measured.then_some(SIZE {
        cx: size.cx.max(0),
        cy: size.cy.max(0),
    })
}

#[cfg(target_os = "windows")]
fn native_control_text_width(handle: *mut std::ffi::c_void, text: &str) -> Option<i32> {
    native_control_text_extent(handle, text).map(|size| size.cx)
}

/// Uses native GDI measurement when available and wxDragon's extent as a portable fallback.
/// Returning None for a non-empty label means it could not be measured reliably; callers treat that
/// conservatively as clipped so a reveal is shown rather than accidentally suppressed.
fn control_text_width<F>(
    handle: *mut std::ffi::c_void,
    text: &str,
    wx_measure: F,
) -> Option<i32>
where
    F: FnOnce() -> i32,
{
    #[cfg(target_os = "windows")]
    if let Some(width) = native_control_text_width(handle, text) {
        return Some(width);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = handle;

    let width = wx_measure();
    if text.is_empty() || width > 0 {
        Some(width.max(0))
    } else {
        None
    }
}

fn build_main_window(initial_file: Option<PathBuf>) {
    let frame = Frame::builder()
        .with_title("Rust HLP Viewer")
        .with_size(Size::new(1000, 720))
        .build();
    set_frame_application_icon(&frame);

    let toolbar = frame
        .create_tool_bar(
            Some(ToolBarStyle::Flat | ToolBarStyle::Text | ToolBarStyle::NoIcons),
            -1,
        )
        .expect("wxWidgets failed to create the main browsing toolbar");
    let empty_bitmap = Bitmap::null_bitmap();
    let _ = toolbar.add_tool(
        ID_PREVIOUS_TOPIC,
        "Previous",
        &empty_bitmap,
        "Open the physically previous decoded topic",
    );
    let _ = toolbar.add_tool(
        ID_NEXT_TOPIC,
        "Next",
        &empty_bitmap,
        "Open the physically next decoded topic",
    );
    toolbar.add_separator();
    let _ = toolbar.add_check_tool(
        ID_TOGGLE_NAVIGATION,
        "Navigation",
        &empty_bitmap,
        "Show or hide the navigation side panel",
    );
    toolbar.toggle_tool(ID_TOGGLE_NAVIGATION, true);
    toolbar.add_separator();
    let _ = toolbar.add_tool(
        ID_ZOOM_OUT,
        "-",
        &empty_bitmap,
        "Zoom help text out",
    );
    let _ = toolbar.add_tool(
        ID_ZOOM_IN,
        "+",
        &empty_bitmap,
        "Zoom help text in",
    );
    let _ = toolbar.realize();
    // Keep a real row reserved for the frame-owned browser toolbar even when the platform
    // computes an unexpectedly small best size for a text-only toolbar with null bitmaps.
    toolbar.set_min_size(Size::new(-1, 1));
    toolbar.show(false);

    // Keep the navigation and document chrome in a native splitter so the left discovery pane can
    // be resized by dragging its sash. Each side owns a complete vertical column: the left column
    // retains the 40 px alignment band above the tabs, while the right column owns the browse bar
    // and document surface. This keeps the browse controls centred over the document at every sash
    // position without manually mirroring a fixed spacer width.
    let body_splitter = SplitterWindow::builder(&frame)
        .with_style(SplitterWindowStyle::Default | SplitterWindowStyle::LiveUpdate)
        .build();
    body_splitter.set_minimum_pane_size(NAVIGATION_PANE_MIN_WIDTH);
    let navigation_column = Panel::builder(&body_splitter).build();
    let content_column = Panel::builder(&body_splitter).build();

    let navigation_top_spacer = Panel::builder(&navigation_column).build();
    navigation_top_spacer.set_min_size(Size::new(-1, 40));

    let browse_bar = Panel::builder(&content_column).build();
    let browse_previous = Button::builder(&browse_bar).with_label("◀").build();
    let browse_next = Button::builder(&browse_bar).with_label("▶").build();
    let browse_prev_seq = Button::builder(&browse_bar).with_label("⇤").build();
    let browse_next_seq = Button::builder(&browse_bar).with_label("⇥").build();
    let browse_toggle_navigation = Button::builder(&browse_bar).with_label("☰").build();
    let browse_zoom_out = Button::builder(&browse_bar).with_label("−").build();
    let browse_zoom_in = Button::builder(&browse_bar).with_label("+").build();

    for button in [
        browse_previous,
        browse_next,
        browse_prev_seq,
        browse_next_seq,
        browse_toggle_navigation,
        browse_zoom_out,
        browse_zoom_in,
    ] {
        button.set_min_size(Size::new(40, 30));
    }
    // The custom browse bar is the visible WinHelp-style toolbox. Give it a deliberate
    // 5 px top/bottom inset and regular inter-button/group spacing instead of relying on
    // platform-dependent best-size padding.
    browse_bar.set_min_size(Size::new(-1, 40));

    browse_previous.set_tooltip("Previous physical topic (Left)");
    browse_next.set_tooltip("Next physical topic (Right)");
    browse_prev_seq.set_tooltip("Previous authored browse topic");
    browse_next_seq.set_tooltip("Next authored browse topic");
    browse_toggle_navigation.set_tooltip("Show or hide the navigation pane (F9)");
    browse_zoom_out.set_tooltip("Zoom help text out");
    browse_zoom_in.set_tooltip("Zoom help text in");
    apply_windows_tooltip_palette(frame.get_handle());
    browse_prev_seq.show(false);
    browse_next_seq.show(false);

    let browse_row = BoxSizer::builder(Orientation::Horizontal).build();
    // Keep the whole toolbox centred while making the rhythm explicit: 4 px within a
    // logical pair, 10 px between groups. This keeps the remaining navigation controls
    // inconsistent platform padding visible in the previous build-fix.
    browse_row.add_stretch_spacer(1);
    browse_row.add(&browse_previous, 0, SizerFlag::Expand, 0);
    browse_row.add_spacer(4);
    browse_row.add(&browse_next, 0, SizerFlag::Expand, 0);
    browse_row.add_spacer(10);
    browse_row.add(&browse_prev_seq, 0, SizerFlag::Expand, 0);
    browse_row.add_spacer(4);
    browse_row.add(&browse_next_seq, 0, SizerFlag::Expand, 0);
    browse_row.add_spacer(10);
    browse_row.add(&browse_toggle_navigation, 0, SizerFlag::Expand, 0);
    browse_row.add_spacer(10);
    browse_row.add(&browse_zoom_out, 0, SizerFlag::Expand, 0);
    browse_row.add_spacer(4);
    browse_row.add(&browse_zoom_in, 0, SizerFlag::Expand, 0);
    browse_row.add_stretch_spacer(1);

    let browse_outer = BoxSizer::builder(Orientation::Vertical).build();
    browse_outer.add_spacer(5);
    browse_outer.add_sizer(&browse_row, 0, SizerFlag::Expand, 0);
    browse_outer.add_spacer(5);
    browse_bar.set_sizer(browse_outer, true);

    let navigation = Notebook::builder(&navigation_column).build();
    navigation.set_min_size(Size::new(NAVIGATION_PANE_MIN_WIDTH, -1));

    let contents_panel = Panel::builder(&navigation).build();
    let contents_controls = Panel::builder(&contents_panel).build();
    let contents_hierarchical = Button::builder(&contents_controls)
        .with_label("Hierarchical view")
        .build();
    contents_hierarchical.set_tooltip("Show the hierarchy authored by .CNT or recovered from a WinHelp .GID cache");
    let contents_show_all = Button::builder(&contents_controls).with_label("Show all").build();
    contents_show_all.set_tooltip("Show every decoded HLP topic as one flat list");
    let contents_controls_sizer = BoxSizer::builder(Orientation::Horizontal).build();
    contents_controls_sizer.add(&contents_hierarchical, 1, SizerFlag::Expand, 0);
    contents_controls_sizer.add_spacer(4);
    contents_controls_sizer.add(&contents_show_all, 1, SizerFlag::Expand, 0);
    contents_controls.set_sizer(contents_controls_sizer, true);

    // HideRoot keeps the help-file title out of the row list, matching WinHelp's Contents tab.
    // It also stops MSW's TVM_ENSUREVISIBLE from horizontally scrolling the tree by one indent
    // level when a child is selected, which used to shear the first characters off the top row.
    // LinesAtRoot is required alongside it: MSW only sets TVS_LINESATROOT when asked, and the
    // common control draws expand buttons on top-level items only when that style is present.
    // Authored/cached Contents books are top-level rows once the root is hidden, so without it they would
    // have no visible expander. wxWidgets places no restriction on combining the two.
    let contents_tree = TreeCtrl::builder(&contents_panel)
        .with_style(
            TreeCtrlStyle::HasButtons | TreeCtrlStyle::LinesAtRoot | TreeCtrlStyle::HideRoot,
        )
        .build();
    let contents_sizer = BoxSizer::builder(Orientation::Vertical).build();
    contents_sizer.add(&contents_controls, 0, SizerFlag::Expand, 0);
    contents_sizer.add_spacer(4);
    contents_sizer.add(&contents_tree, 1, SizerFlag::Expand, 0);
    contents_panel.set_sizer(contents_sizer, true);

    let index_panel = Panel::builder(&navigation).build();
    let index_query = TextCtrl::builder(&index_panel).build();
    index_query.set_tooltip("Filter the authored WinHelp keyword index");
    let index_list = ListBox::builder(&index_panel).build();
    let index_sizer = BoxSizer::builder(Orientation::Vertical).build();
    index_sizer.add(&index_query, 0, SizerFlag::Expand, 0);
    index_sizer.add(&index_list, 1, SizerFlag::Expand, 0);
    index_panel.set_sizer(index_sizer, true);

    let search_panel = Panel::builder(&navigation).build();
    let search_query = TextCtrl::builder(&search_panel).build();
    search_query.set_tooltip("Search titles, authored keywords, and decoded topic text");
    let search_list = ListBox::builder(&search_panel).build();
    let search_sizer = BoxSizer::builder(Orientation::Vertical).build();
    search_sizer.add(&search_query, 0, SizerFlag::Expand, 0);
    search_sizer.add(&search_list, 1, SizerFlag::Expand, 0);
    search_panel.set_sizer(search_sizer, true);

    let bookmarks_panel = Panel::builder(&navigation).build();
    let bookmark_controls = Panel::builder(&bookmarks_panel).build();
    let bookmark_add = Button::builder(&bookmark_controls).with_label("+").build();
    bookmark_add.set_tooltip("Add the current topic to bookmarks");
    bookmark_add.set_min_size(Size::new(42, -1));
    let bookmark_remove = Button::builder(&bookmark_controls).with_label("-").build();
    bookmark_remove.set_tooltip("Remove the selected bookmark");
    bookmark_remove.set_min_size(Size::new(42, -1));
    let bookmark_controls_sizer = BoxSizer::builder(Orientation::Horizontal).build();
    bookmark_controls_sizer.add(&bookmark_add, 0, SizerFlag::Expand, 0);
    bookmark_controls_sizer.add_spacer(4);
    bookmark_controls_sizer.add(&bookmark_remove, 0, SizerFlag::Expand, 0);
    bookmark_controls_sizer.add_stretch_spacer(1);
    bookmark_controls.set_sizer(bookmark_controls_sizer, true);
    let bookmarks_list = ListBox::builder(&bookmarks_panel).build();
    let bookmarks_sizer = BoxSizer::builder(Orientation::Vertical).build();
    bookmarks_sizer.add(&bookmark_controls, 0, SizerFlag::Expand, 0);
    bookmarks_sizer.add(&bookmarks_list, 1, SizerFlag::Expand, 0);
    bookmarks_panel.set_sizer(bookmarks_sizer, true);

    let history_panel = Panel::builder(&navigation).build();
    let history_list = ListBox::builder(&history_panel).build();
    let history_sizer = BoxSizer::builder(Orientation::Vertical).build();
    history_sizer.add(&history_list, 1, SizerFlag::Expand, 0);
    history_panel.set_sizer(history_sizer, true);

    navigation.add_page(&contents_panel, "Contents", true, None);
    navigation.add_page(&index_panel, "Index", false, None);
    navigation.add_page(&search_panel, "Search", false, None);
    navigation.add_page(&bookmarks_panel, "Bookmarks", false, None);
    navigation.add_page(&history_panel, "History", false, None);

    let content_host = Panel::builder(&content_column).build();
    content_host.set_background_color(colour_from_rgb(CONTENT_HOST_BACKGROUND));
    let page_border = Panel::builder(&content_host).build();
    page_border.set_background_color(colour_from_rgb(Rgb { red: 0, green: 0, blue: 0 }));
    let page_inner = Panel::builder(&page_border).build();
    page_inner.set_background_color(colour_from_rgb(HELP_BACKGROUND));

    let fixed_canvas = Panel::builder(&page_inner).build();
    fixed_canvas.set_background_color(colour_from_rgb(HELP_BACKGROUND));
    fixed_canvas.set_background_style(BackgroundStyle::Paint);
    fixed_canvas.set_can_focus(true);
    fixed_canvas.show(false);

    let scrolled = ScrolledWindow::builder(&page_inner).build();
    scrolled.enable_scrolling(false, true);
    scrolled.set_scroll_rate(0, SCROLL_UNIT);
    scrolled.set_background_color(colour_from_rgb(HELP_BACKGROUND));

    // Painting a child panel lets wxScrolledWindow move the child natively. The paint and mouse
    // coordinates therefore remain in document coordinates rather than virtual-scroll units.
    let scrolling_canvas = Panel::builder(&scrolled).build();
    scrolling_canvas.set_background_color(colour_from_rgb(HELP_BACKGROUND));
    scrolling_canvas.set_background_style(BackgroundStyle::Paint);
    scrolling_canvas.set_can_focus(true);

    let content_layout = BoxSizer::builder(Orientation::Vertical).build();
    content_layout.add(&fixed_canvas, 0, SizerFlag::Expand, 0);
    content_layout.add(&scrolled, 1, SizerFlag::Expand, 0);
    page_inner.set_sizer(content_layout, true);

    let navigation_column_layout = BoxSizer::builder(Orientation::Vertical).build();
    navigation_column_layout.add(&navigation_top_spacer, 0, SizerFlag::Expand, 0);
    navigation_column_layout.add(&navigation, 1, SizerFlag::Expand, 0);
    navigation_column.set_sizer(navigation_column_layout, true);

    let content_column_layout = BoxSizer::builder(Orientation::Vertical).build();
    content_column_layout.add(&browse_bar, 0, SizerFlag::Expand, 0);
    content_column_layout.add(&content_host, 1, SizerFlag::Expand, 0);
    content_column.set_sizer(content_column_layout, true);

    let _ = body_splitter.split_vertically(&navigation_column, &content_column, NAVIGATION_PANE_WIDTH);

    let root_layout = BoxSizer::builder(Orientation::Vertical).build();
    root_layout.add(&body_splitter, 1, SizerFlag::Expand, 0);
    frame.set_sizer(root_layout, true);

    let status_bar = StatusBar::builder(&frame)
        .with_fields_count(2)
        .with_status_widths(vec![-1, 300])
        .add_initial_text(0, "Ready")
        .add_initial_text(1, "No HLP loaded")
        .build();
    frame.set_existing_status_bar(Some(&status_bar));

    let recent_documents = recent::load().unwrap_or_default();
    let (saved_bookmarks, bookmark_load_error) = match bookmark_store::load() {
        Ok(bookmarks) => (bookmarks, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    let bookmarks = saved_bookmarks
        .into_iter()
        .map(bookmark_entry_from_stored)
        .collect();
    install_main_menu_bar(&frame, &recent_documents);

    let ui = Rc::new(ViewerUi {
        frame,
        toolbar,
        body_splitter,
        navigation_column,
        content_column,
        browse_bar,
        browse_previous,
        browse_next,
        browse_prev_seq,
        browse_next_seq,
        browse_toggle_navigation,
        browse_zoom_out,
        browse_zoom_in,
        navigation,
        contents_hierarchical,
        contents_show_all,
        contents_tree,
        index_query,
        index_list,
        search_query,
        search_list,
        bookmark_add,
        bookmark_remove,
        bookmarks_list,
        history_list,
        content_host,
        page_border,
        page_inner,
        fixed_canvas,
        scrolled,
        scrolling_canvas,
        status_bar,
    });
    let state = Rc::new(RefCell::new(ViewerState {
        bookmarks,
        recent_documents,
        ..ViewerState::default()
    }));
    // All static wxToolTip instances have been attached by this point. Reapply the palette after
    // constructing the complete UI so the shared native tooltip window cannot retain the host
    // theme's white background merely because it was created after the browse strip.
    apply_windows_tooltip_palette(ui.frame.get_handle());
    if let Some(error) = bookmark_load_error {
        ui.status_bar
            .set_status_text(&format!("Could not load bookmarks: {error}"), 0);
    }

    bind_navigation_pane(Rc::clone(&ui), Rc::clone(&state));
    bind_navigation_overflow_tooltips(&ui);
    bind_edit_focus_tracking(&ui, &state);
    refresh_navigation_pane(&ui, &state);

    bind_paint_handler(fixed_canvas, Rc::clone(&state), true);
    bind_paint_handler(scrolling_canvas, Rc::clone(&state), false);
    bind_hotspot_handler(Rc::clone(&ui), Rc::clone(&state), true);
    bind_hotspot_handler(Rc::clone(&ui), Rc::clone(&state), false);
    bind_main_hotspot_tooltip(ui.fixed_canvas, Rc::clone(&state), true);
    bind_main_hotspot_tooltip(ui.scrolling_canvas, Rc::clone(&state), false);
    bind_main_transient_dismissal_surfaces(&ui, &state);

    // Handle arrow navigation as native key events instead of depending on menu-accelerator
    // parsing. Bind the same handler to every surface that may own keyboard focus.
    bind_main_navigation_keys(frame, Rc::clone(&ui), Rc::clone(&state));
    bind_main_navigation_keys(fixed_canvas, Rc::clone(&ui), Rc::clone(&state));
    bind_main_navigation_keys(scrolled, Rc::clone(&ui), Rc::clone(&state));
    bind_main_navigation_keys(scrolling_canvas, Rc::clone(&ui), Rc::clone(&state));

    let ui_for_menu = Rc::clone(&ui);
    let state_for_menu = Rc::clone(&state);
    frame.on_menu_selected(move |event: MenuEventData| {
        dismiss_main_transients(&ui_for_menu, &state_for_menu);
        match event.get_id() {
        ID_OPEN => open_document_dialog(&ui_for_menu, &state_for_menu),
        ID_EXPORT_HTML => export_html_dialog(&ui_for_menu, &state_for_menu),
        ID_PRINT => print_topics(&ui_for_menu, &state_for_menu),
        ID_COPY => copy_edit_selection(&ui_for_menu, &state_for_menu),
        ID_PASTE => paste_edit_selection(&ui_for_menu, &state_for_menu),
        ID_SELECT_ALL => select_all_edit_target(&ui_for_menu, &state_for_menu),
        ID_BACK => navigate_history(&ui_for_menu, &state_for_menu, true),
        ID_FORWARD => navigate_history(&ui_for_menu, &state_for_menu, false),
        ID_CONTENTS => navigate_contents(&ui_for_menu, &state_for_menu),
        ID_PREVIOUS_TOPIC => navigate_adjacent_topic(&ui_for_menu, &state_for_menu, false),
        ID_NEXT_TOPIC => navigate_adjacent_topic(&ui_for_menu, &state_for_menu, true),
        ID_BROWSE_PREVIOUS => macro_browse_main(&ui_for_menu, &state_for_menu, false),
        ID_BROWSE_NEXT => macro_browse_main(&ui_for_menu, &state_for_menu, true),
        ID_ZOOM_OUT => adjust_text_zoom(&ui_for_menu, &state_for_menu, -TEXT_ZOOM_STEP_PERCENT),
        ID_ZOOM_IN => adjust_text_zoom(&ui_for_menu, &state_for_menu, TEXT_ZOOM_STEP_PERCENT),
        ID_TOGGLE_NAVIGATION => toggle_navigation_pane(&ui_for_menu, &state_for_menu),
        ID_MACRO_DIAGNOSTICS => show_macro_diagnostics(&ui_for_menu, &state_for_menu),
        ID_EXIT => ui_for_menu.frame.close(true),
        ID_ABOUT => show_about(&ui_for_menu.frame),
        other => {
            if let Some(index) = recent_document_index(other) {
                open_recent_document(&ui_for_menu, &state_for_menu, index);
            }
        }
        }
    });

    layout_main_content_chrome(&ui);

    let ui_for_previous_button = Rc::clone(&ui);
    let state_for_previous_button = Rc::clone(&state);
    ui.browse_previous.on_click(move |_| navigate_adjacent_topic(&ui_for_previous_button, &state_for_previous_button, false));
    let ui_for_next_button = Rc::clone(&ui);
    let state_for_next_button = Rc::clone(&state);
    ui.browse_next.on_click(move |_| navigate_adjacent_topic(&ui_for_next_button, &state_for_next_button, true));
    let ui_for_prev_seq_button = Rc::clone(&ui);
    let state_for_prev_seq_button = Rc::clone(&state);
    ui.browse_prev_seq.on_click(move |_| macro_browse_main(&ui_for_prev_seq_button, &state_for_prev_seq_button, false));
    let ui_for_next_seq_button = Rc::clone(&ui);
    let state_for_next_seq_button = Rc::clone(&state);
    ui.browse_next_seq.on_click(move |_| macro_browse_main(&ui_for_next_seq_button, &state_for_next_seq_button, true));
    let ui_for_nav_button = Rc::clone(&ui);
    let state_for_nav_button = Rc::clone(&state);
    ui.browse_toggle_navigation.on_click(move |_| toggle_navigation_pane(&ui_for_nav_button, &state_for_nav_button));
    let ui_for_zoom_out_button = Rc::clone(&ui);
    let state_for_zoom_out_button = Rc::clone(&state);
    ui.browse_zoom_out.on_click(move |_| adjust_text_zoom(&ui_for_zoom_out_button, &state_for_zoom_out_button, -TEXT_ZOOM_STEP_PERCENT));
    let ui_for_zoom_in_button = Rc::clone(&ui);
    let state_for_zoom_in_button = Rc::clone(&state);
    ui.browse_zoom_in.on_click(move |_| adjust_text_zoom(&ui_for_zoom_in_button, &state_for_zoom_in_button, TEXT_ZOOM_STEP_PERCENT));

    // Reflow from the content host's own size event, not the frame's. During a frame restore the
    // frame EVT_SIZE can run before wxWidgets has propagated the new sizer geometry to the help
    // page; measuring `scrolled` there retained the old maximized width, so text and the cream
    // background were clipped after returning to the smaller window. The host event fires after
    // that propagation and therefore exposes the authoritative restored viewport dimensions.
    let ui_for_size = Rc::clone(&ui);
    let state_for_size = Rc::clone(&state);
    content_host.on_size(move |event: WindowEventData| {
        event.skip(true);
        layout_main_content_chrome(&ui_for_size);
        let target_width = usable_layout_width(ui_for_size.scrolled);
        let needs_reflow = {
            let state = state_for_size.borrow();
            state.document.is_some() && state.layout_width != target_width
        };
        if needs_reflow {
            refresh_topic_layout(&ui_for_size, &state_for_size);
        } else {
            // Force newly exposed native panel area to erase with the configured background even
            // when wrapping width happened to remain unchanged.
            ui_for_size.content_host.refresh(true, None);
            ui_for_size.page_border.refresh(true, None);
            ui_for_size.page_inner.refresh(true, None);
            ui_for_size.fixed_canvas.refresh(true, None);
            ui_for_size.scrolling_canvas.refresh(true, None);
        }
    });

    ui.browse_bar.show(true);
    frame.layout();
    frame.centre();
    frame.show(true);

    if let Some(path) = initial_file {
        load_document(&ui, &state, &path, true);
    }
    ui.scrolling_canvas.set_focus();
}

/// Places the cream help page inside a black border with a gray outer host, matching the
/// classic framed WinHelp document look without changing the native shell/navigation UI.
fn layout_main_content_chrome(ui: &ViewerUi) {
    let host_size = ui.content_host.get_client_size();
    let host_width = host_size.width.max(0);
    let host_height = host_size.height.max(0);
    let horizontal_margin = CONTENT_FRAME_MARGIN.min((host_width / 2).max(0));
    let vertical_margin = CONTENT_FRAME_MARGIN.min((host_height / 2).max(0));
    let border_width = (host_width - horizontal_margin * 2).max(0);
    let border_height = (host_height - vertical_margin * 2).max(0);
    ui.page_border.set_size_with_pos(horizontal_margin, vertical_margin, border_width, border_height);

    let inner_width = (border_width - CONTENT_FRAME_BORDER_THICKNESS * 2).max(0);
    let inner_height = (border_height - CONTENT_FRAME_BORDER_THICKNESS * 2).max(0);
    ui.page_inner.set_size_with_pos(
        CONTENT_FRAME_BORDER_THICKNESS,
        CONTENT_FRAME_BORDER_THICKNESS,
        inner_width,
        inner_height,
    );
    ui.page_inner.layout();
}

/// Binds direct topic arrows plus browser-style Alt+Left / Alt+Right history navigation.
fn bind_main_navigation_keys(
    widget: impl WxWidget + WindowEvents + Copy + 'static,
    ui: Rc<ViewerUi>,
    state: Rc<RefCell<ViewerState>>,
) {
    widget.on_key_down(move |event: WindowEventData| {
        let WindowEventData::Keyboard(keyboard) = &event else {
            event.skip(true);
            return;
        };

        let key_code = keyboard.get_key_code();
        let alt_down = keyboard.alt_down();
        let control_down = keyboard.control_down();
        let shift_down = keyboard.shift_down();

        if alt_down && !control_down && !shift_down {
            match key_code {
                Some(WXK_LEFT) => {
                    navigate_history(&ui, &state, true);
                    return;
                }
                Some(WXK_RIGHT) => {
                    navigate_history(&ui, &state, false);
                    return;
                }
                _ => {}
            }
        } else if !alt_down && !control_down && !shift_down {
            match key_code {
                Some(WXK_LEFT) => {
                    navigate_adjacent_topic(&ui, &state, false);
                    return;
                }
                Some(WXK_RIGHT) => {
                    navigate_adjacent_topic(&ui, &state, true);
                    return;
                }
                _ => {}
            }
        }

        // Only unhandled keys are passed back to wxWidgets. Consuming Left/Right here prevents the
        // scrolled window from interpreting them as scrolling/navigation after we change topics.
        event.skip(true);
    });
}

#[derive(Debug, Clone)]
enum ContentsAction {
    Authored(usize),
    Topic(usize),
}

/// Connects the native navigation pane to the same document/history machinery as topic hotspots.
fn bind_navigation_pane(ui: Rc<ViewerUi>, state: Rc<RefCell<ViewerState>>) {
    let ui_for_hierarchical = Rc::clone(&ui);
    let state_for_hierarchical = Rc::clone(&state);
    ui.contents_hierarchical.on_click(move |_| {
        state_for_hierarchical.borrow_mut().contents_view_mode = ContentsViewMode::Hierarchical;
        refresh_contents_tree(&ui_for_hierarchical, &state_for_hierarchical);
    });

    let ui_for_show_all = Rc::clone(&ui);
    let state_for_show_all = Rc::clone(&state);
    ui.contents_show_all.on_click(move |_| {
        state_for_show_all.borrow_mut().contents_view_mode = ContentsViewMode::AllTopics;
        refresh_contents_tree(&ui_for_show_all, &state_for_show_all);
    });

    let ui_for_contents = Rc::clone(&ui);
    let state_for_contents = Rc::clone(&state);
    ui.contents_tree.on_item_activated(move |_| {
        activate_contents_selection(&ui_for_contents, &state_for_contents);
    });
    let ui_for_contents_single = Rc::clone(&ui);
    let state_for_contents_single = Rc::clone(&state);
    ui.contents_tree.on_selection_changed(move |_| {
        activate_contents_selection(&ui_for_contents_single, &state_for_contents_single);
    });

    let ui_for_index_query = Rc::clone(&ui);
    let state_for_index_query = Rc::clone(&state);
    ui.index_query.on_text_updated(move |event| {
        state_for_index_query.borrow_mut().index_query = event.get_string().unwrap_or_default();
        refresh_index_list(&ui_for_index_query, &state_for_index_query);
    });

    let ui_for_index = Rc::clone(&ui);
    let state_for_index = Rc::clone(&state);
    ui.index_list.on_item_double_clicked(move |_| {
        activate_index_selection(&ui_for_index, &state_for_index);
    });

    let ui_for_search_query = Rc::clone(&ui);
    let state_for_search_query = Rc::clone(&state);
    ui.search_query.on_text_updated(move |event| {
        state_for_search_query.borrow_mut().search_query = event.get_string().unwrap_or_default();
        refresh_search_list(&ui_for_search_query, &state_for_search_query);
    });

    let ui_for_search = Rc::clone(&ui);
    let state_for_search = Rc::clone(&state);
    ui.search_list.on_item_double_clicked(move |_| {
        activate_search_selection(&ui_for_search, &state_for_search);
    });

    let ui_for_bookmark_add = Rc::clone(&ui);
    let state_for_bookmark_add = Rc::clone(&state);
    ui.bookmark_add.on_click(move |_| {
        add_current_bookmark(&ui_for_bookmark_add, &state_for_bookmark_add);
    });

    let ui_for_bookmark_remove = Rc::clone(&ui);
    let state_for_bookmark_remove = Rc::clone(&state);
    ui.bookmark_remove.on_click(move |_| {
        remove_selected_bookmark(&ui_for_bookmark_remove, &state_for_bookmark_remove);
    });

    let ui_for_bookmarks = Rc::clone(&ui);
    let state_for_bookmarks = Rc::clone(&state);
    ui.bookmarks_list.on_item_double_clicked(move |_| {
        activate_bookmark_selection(&ui_for_bookmarks, &state_for_bookmarks);
    });

    let ui_for_history = Rc::clone(&ui);
    let state_for_history = Rc::clone(&state);
    ui.history_list.on_item_double_clicked(move |_| {
        activate_visible_history_selection(&ui_for_history, &state_for_history);
    });
}

/// Rebuilds document-dependent navigation controls after an HLP is installed.
fn refresh_navigation_pane(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    refresh_contents_tree(ui, state);
    refresh_index_list(ui, state);
    refresh_search_list(ui, state);
    refresh_bookmark_list(ui, state);
    refresh_history_list(ui, state);
}

fn refresh_contents_tree(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    ui.contents_tree.delete_all_items();
    let (document, view_mode) = {
        let state = state.borrow();
        (state.navigation_document.clone(), state.contents_view_mode)
    };
    ui.contents_hierarchical
        .enable(view_mode != ContentsViewMode::Hierarchical);
    ui.contents_show_all
        .enable(view_mode != ContentsViewMode::AllTopics);

    let Some(document) = document else {
        // The root is hidden, so its own label never paints. The placeholder has to be a real
        // child row to be visible at all.
        if let Some(root) = ui.contents_tree.add_root("Contents", None, None) {
            let _ = ui.contents_tree.append_item(&root, "Open an HLP file", None, None);
        }
        return;
    };

    // Retained so the pane still labels itself if the hidden-root style is ever removed;
    // wxWidgets discards the root's text while wxTR_HIDE_ROOT is set.
    let root_title = document
        .contents_file()
        .and_then(|contents| contents.title.as_deref())
        .or(document.system().title.as_deref())
        .unwrap_or("Contents");
    let Some(root) = ui.contents_tree.add_root(root_title, None, None) else {
        return;
    };

    match view_mode {
        ContentsViewMode::Hierarchical => {
            if let Some(contents) = document.contents_file() {
                let mut ancestors: Vec<(u16, TreeItemId)> = Vec::new();
                for (index, entry) in contents.items.iter().enumerate() {
                    while ancestors
                        .last()
                        .is_some_and(|(level, _)| *level >= entry.level)
                    {
                        ancestors.pop();
                    }
                    let parent = ancestors
                        .last()
                        .map(|(_, item)| item)
                        .unwrap_or(&root);
                    let item = if entry.target.is_some() {
                        ui.contents_tree.append_item_with_data(
                            parent,
                            &entry.title,
                            ContentsAction::Authored(index),
                            None,
                            None,
                        )
                    } else {
                        ui.contents_tree.append_item(parent, &entry.title, None, None)
                    };
                    if let Some(item) = item {
                        ancestors.push((entry.level, item));
                    }
                }
            } else {
                // WinHelp 4.x can retain a CNT-authored tree in a compiled GID cache. Do not label
                // physical TOPIC order as "Contents": that was the misleading pre-build-fix-45
                // fallback. The explicit Show all mode remains available for inspection.
                let _ = ui.contents_tree.append_item(
                    &root,
                    "Hierarchical contents unavailable (.CNT/.GID data not found)",
                    None,
                    None,
                );
                let _ = ui.contents_tree.append_item(
                    &root,
                    "Use Show all to list every decoded topic",
                    None,
                    None,
                );
            }
        }
        ContentsViewMode::AllTopics => {
            for (index, topic) in document.presentations().iter().enumerate() {
                let title = topic_label(topic.title.as_str(), index);
                let _ = ui.contents_tree.append_item_with_data(
                    &root,
                    &title,
                    ContentsAction::Topic(index),
                    None,
                    None,
                );
            }
        }
    }

    // No expand call here: the root is hidden, so its children are already the visible top
    // level, and wxWidgets asserts on DoExpand() for a hidden root. Nested authored books stay
    // collapsed until the user opens them, which is what WinHelp's Contents tab does too.
    sync_contents_selection(ui, state);
}

/// Selects the authored/fallback Contents row that resolves to the active topic when possible.
///
/// Cross-document topics deliberately leave the original document's selection alone. This keeps
/// the navigation tree stable instead of making an unrelated external HLP appear to own the pane.
fn sync_contents_selection(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    let state_ref = state.borrow();
    let Some(document) = state_ref.navigation_document.as_ref() else {
        return;
    };
    let same_navigation_file = state_ref.document.as_ref().is_some_and(|active| {
        path_identity(active.source_path()) == path_identity(document.source_path())
    });
    if !same_navigation_file {
        return;
    }
    let topic_index = state_ref.topic_index;
    let Some(root) = ui.contents_tree.get_root_item() else {
        return;
    };
    ui.contents_tree.unselect_all();

    let mut stack = vec![root];
    while let Some(item) = stack.pop() {
        if let Some(data) = ui.contents_tree.get_custom_data(&item) {
            if let Some(action) = data.downcast_ref::<ContentsAction>() {
                if contents_action_topic_index(document, action) == Some(topic_index) {
                    reveal_contents_item(&ui.contents_tree, &item);
                    ui.contents_tree.select_item(&item);
                    return;
                }
            }
        }

        if let Some((first_child, mut cookie)) = ui.contents_tree.get_first_child(&item) {
            let mut child = Some(first_child);
            while let Some(current) = child {
                stack.push(current);
                child = ui.contents_tree.get_next_child(&item, &mut cookie);
            }
        }
    }
}

/// Makes a Contents item visible without ever asking wxWidgets to expand the hidden root.
///
/// `TreeCtrl::ensure_visible()` recursively expands ancestors. With `HideRoot`, that eventually
/// reaches the invisible root item and wxMSW asserts in `DoExpand()` (treectrl.cpp) because a
/// hidden root is not a legal expand/collapse target. Expand only real authored book ancestors,
/// stopping when their parent is the hidden root, then scroll directly to the selected item.
fn reveal_contents_item(tree: &TreeCtrl, item: &TreeItemId) {
    let mut ancestors = Vec::new();
    let mut parent = tree.get_item_parent(item);
    while let Some(candidate) = parent {
        let grandparent = tree.get_item_parent(&candidate);
        if grandparent.is_none() {
            // `candidate` is the hidden root itself. Never pass it to Expand/Collapse.
            break;
        }
        parent = grandparent;
        ancestors.push(candidate);
    }

    for ancestor in ancestors.iter().rev() {
        if !tree.is_expanded(ancestor) {
            tree.expand(ancestor);
        }
    }
    tree.scroll_to(item);
}

fn contents_action_topic_index(
    document: &HelpDocument,
    action: &ContentsAction,
) -> Option<usize> {
    match action {
        ContentsAction::Topic(topic_index) => Some(*topic_index),
        ContentsAction::Authored(entry_index) => {
            let contents = document.contents_file()?;
            let target = contents.items.get(*entry_index)?.target.as_ref()?;
            let help_file = target.help_file.as_deref().or_else(|| {
                contents.base.as_ref().map(|base| base.help_file.as_str())
            });
            if let Some(help_file) = help_file {
                let target_path = resolve_external_help_path(document.source_path(), help_file);
                if path_identity(&target_path) != path_identity(document.source_path()) {
                    return None;
                }
            }
            document.topic_index_for_reference(&target.context)
        }
    }
}

fn activate_contents_selection(ui: &Rc<ViewerUi>, state: &Rc<RefCell<ViewerState>>) {
    let Some(item) = ui.contents_tree.get_selection() else {
        return;
    };
    let Some(data) = ui.contents_tree.get_custom_data(&item) else {
        return;
    };
    let Some(action) = data.downcast_ref::<ContentsAction>().cloned() else {
        return;
    };
    match action {
        ContentsAction::Topic(topic_index) => {
            let document = state.borrow().navigation_document.clone();
            let Some(document) = document else {
                return;
            };
            navigate_main_to_document(ui, state, document, topic_index, None);
        }
        ContentsAction::Authored(entry_index) => {
            let document = state.borrow().navigation_document.clone();
            let Some(document) = document else {
                return;
            };
            let Some(contents) = document.contents_file() else {
                return;
            };
            let Some(target) = contents
                .items
                .get(entry_index)
                .and_then(|entry| entry.target.as_ref())
                .cloned()
            else {
                return;
            };
            navigate_contents_target(ui, state, &document, contents, &target);
        }
    }
}

fn navigate_contents_target(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    current_document: &HelpDocument,
    contents: &hlp::ContentsFile,
    target: &ContentsTarget,
) {
    let help_file = target.help_file.as_deref().or_else(|| {
        contents
            .base
            .as_ref()
            .map(|base| base.help_file.as_str())
    });
    let window_name = target.window_name.as_deref().or_else(|| {
        contents
            .base
            .as_ref()
            .and_then(|base| base.window_name.as_deref())
    });
    let target_document = match load_linked_document(current_document, help_file) {
        Ok(document) => document,
        Err((path, error)) => {
            show_open_error(ui, &path, &error);
            return;
        }
    };
    let Some(topic_index) = target_document.topic_index_for_reference(&target.context) else {
        ui.status_bar.set_status_text(
            &format!("Unresolved contents target '{}'", target.context),
            0,
        );
        return;
    };

    let explicit_window = window_name
        .and_then(|name| target_document.window_by_name(name))
        .cloned();
    if window_name.is_some() && !is_explicit_main_window(window_name, explicit_window.as_ref()) {
        show_topic_window(
            ui,
            state,
            ui.frame,
            &target_document,
            topic_index,
            explicit_window.as_ref(),
            AuxiliaryKind::Secondary,
            None,
        );
    } else {
        route_to_main_or_default_window(ui, state, target_document, topic_index, None);
    }
}

fn refresh_index_list(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    let visible = {
        let state_ref = state.borrow();
        let query = state_ref.index_query.trim().to_lowercase();
        let mut merged: BTreeMap<String, PaneKeyword> = BTreeMap::new();
        for document in state_ref
            .navigation_document
            .iter()
            .chain(state_ref.related_documents.iter())
        {
            for keyword in document.resolved_keywords() {
                if !query.is_empty() && !keyword.keyword.to_lowercase().contains(&query) {
                    continue;
                }
                let key = keyword.keyword.to_lowercase();
                let row = merged.entry(key).or_insert_with(|| PaneKeyword {
                    keyword: keyword.keyword.clone(),
                    locations: Vec::new(),
                });
                for topic_index in &keyword.topic_indices {
                    let location = NavigationLocation {
                        source_path: document.source_path().to_path_buf(),
                        topic_index: *topic_index,
                        topic_offset: document.topic_start_offset(*topic_index),
                        window_name: None,
                    };
                    if !row.locations.contains(&location) {
                        row.locations.push(location);
                    }
                }
            }
        }
        merged.into_values().collect::<Vec<_>>()
    };

    ui.index_list.clear();
    for keyword in &visible {
        let label = if keyword.locations.len() > 1 {
            format!("{}  ({} topics)", keyword.keyword, keyword.locations.len())
        } else {
            keyword.keyword.clone()
        };
        ui.index_list.append(&label);
    }
    state.borrow_mut().index_visible = visible;
}

fn activate_index_selection(ui: &Rc<ViewerUi>, state: &Rc<RefCell<ViewerState>>) {
    let Some(row) = ui.index_list.get_selection().map(|value| value as usize) else {
        return;
    };
    let keyword = state.borrow().index_visible.get(row).cloned();
    let Some(keyword) = keyword else {
        return;
    };
    let destination = if keyword.locations.len() == 1 {
        keyword.locations.first().cloned()
    } else {
        choose_keyword_topic(ui, state, &keyword)
    };
    if let Some(destination) = destination {
        navigate_to_saved_location(ui, state, destination);
    }
}

fn choose_keyword_topic(
    ui: &ViewerUi,
    state: &Rc<RefCell<ViewerState>>,
    keyword: &PaneKeyword,
) -> Option<NavigationLocation> {
    let choices = {
        let state_ref = state.borrow();
        keyword
            .locations
            .iter()
            .map(|location| navigation_location_label(&state_ref, location))
            .collect::<Vec<_>>()
    };
    if choices.is_empty() {
        return None;
    }
    let refs = choices.iter().map(String::as_str).collect::<Vec<_>>();
    let dialog = SingleChoiceDialog::builder(
        &ui.frame,
        &format!("The keyword '{}' matches several topics.", keyword.keyword),
        "Topics Found",
        &refs,
    )
    .build();
    if dialog.show_modal() != wxdragon::id::ID_OK {
        return None;
    }
    usize::try_from(dialog.get_selection())
        .ok()
        .and_then(|selection| keyword.locations.get(selection).cloned())
}

fn refresh_search_list(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    let hits = {
        let state_ref = state.borrow();
        let query = state_ref.search_query.clone();
        if query.trim().is_empty() {
            Vec::new()
        } else {
            let mut hits = Vec::new();
            for document in state_ref
                .navigation_document
                .iter()
                .chain(state_ref.related_documents.iter())
            {
                for hit in document.search(&query, 200) {
                    hits.push(PaneSearchHit {
                        location: NavigationLocation {
                            source_path: document.source_path().to_path_buf(),
                            topic_index: hit.topic_index,
                            topic_offset: document.topic_start_offset(hit.topic_index),
                            window_name: None,
                        },
                        title: hit.title,
                        score: hit.score,
                        match_kind: hit.match_kind,
                    });
                }
            }
            hits.sort_by(|left, right| {
                right
                    .score
                    .cmp(&left.score)
                    .then_with(|| left.location.source_path.cmp(&right.location.source_path))
                    .then_with(|| left.location.topic_index.cmp(&right.location.topic_index))
            });
            hits.truncate(200);
            hits
        }
    };

    ui.search_list.clear();
    for hit in &hits {
        let kind = match hit.match_kind {
            SearchMatchKind::Title => "Title",
            SearchMatchKind::Keyword => "Keyword",
            SearchMatchKind::Body => "Text",
        };
        let file = hit
            .location
            .source_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("help file");
        ui.search_list.append(&format!(
            "[{kind}] {} — {file}",
            topic_label(&hit.title, hit.location.topic_index)
        ));
    }
    state.borrow_mut().search_visible = hits;
}

fn activate_search_selection(ui: &Rc<ViewerUi>, state: &Rc<RefCell<ViewerState>>) {
    let Some(row) = ui.search_list.get_selection().map(|value| value as usize) else {
        return;
    };
    let destination = state
        .borrow()
        .search_visible
        .get(row)
        .map(|hit| hit.location.clone());
    if let Some(destination) = destination {
        navigate_to_saved_location(ui, state, destination);
    }
}

fn add_current_bookmark(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    let candidate = {
        let state_ref = state.borrow();
        current_location(&state_ref).and_then(|location| {
            let document = state_ref.document.as_ref()?;
            let title = document
                .presentations()
                .get(state_ref.topic_index)
                .map(|topic| topic_label(&topic.title, state_ref.topic_index))?;
            let file = document
                .source_path()
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("help file");
            Some((location, format!("{title} — {file}")))
        })
    };
    let Some((location, label)) = candidate else {
        return;
    };

    let mut state_mut = state.borrow_mut();
    if state_mut
        .bookmarks
        .iter()
        .any(|bookmark| bookmark.location == location)
    {
        ui.status_bar.set_status_text("This topic is already bookmarked", 0);
        return;
    }
    state_mut.bookmarks.push(BookmarkEntry { label, location });
    drop(state_mut);
    refresh_bookmark_list(ui, state);
    match save_bookmarks(state) {
        Ok(()) => ui.status_bar.set_status_text("Bookmark added", 0),
        Err(error) => ui
            .status_bar
            .set_status_text(&format!("Bookmark added, but could not save: {error}"), 0),
    }
}

fn remove_selected_bookmark(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    let Some(row) = ui.bookmarks_list.get_selection().map(|value| value as usize) else {
        ui.status_bar
            .set_status_text("Select a bookmark to remove", 0);
        return;
    };

    let removed = {
        let mut state_mut = state.borrow_mut();
        if row >= state_mut.bookmarks.len() {
            false
        } else {
            state_mut.bookmarks.remove(row);
            true
        }
    };
    if !removed {
        return;
    }

    refresh_bookmark_list(ui, state);
    match save_bookmarks(state) {
        Ok(()) => ui.status_bar.set_status_text("Bookmark removed", 0),
        Err(error) => ui
            .status_bar
            .set_status_text(&format!("Bookmark removed, but could not save: {error}"), 0),
    }
}

fn bookmark_entry_from_stored(stored: bookmark_store::StoredBookmark) -> BookmarkEntry {
    BookmarkEntry {
        label: stored.label,
        location: NavigationLocation {
            source_path: stored.source_path,
            topic_index: stored.topic_index,
            topic_offset: stored.topic_offset.map(TopicOffset),
            window_name: stored.window_name,
        },
    }
}

fn save_bookmarks(state: &Rc<RefCell<ViewerState>>) -> io::Result<()> {
    let stored = state
        .borrow()
        .bookmarks
        .iter()
        .map(|bookmark| bookmark_store::StoredBookmark {
            label: bookmark.label.clone(),
            source_path: bookmark.location.source_path.clone(),
            topic_index: bookmark.location.topic_index,
            topic_offset: bookmark.location.topic_offset.map(|offset| offset.0),
            window_name: bookmark.location.window_name.clone(),
        })
        .collect::<Vec<_>>();
    bookmark_store::save(&stored)
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn list_box_row_at_point(list: ListBox, point: WxPoint) -> Option<u32> {
    // LB_ITEMFROMPOINT returns the zero-based row in LOWORD and sets HIWORD when the pointer is
    // outside the client area. The native hit test remains correct after vertical scrolling.
    const LB_ITEMFROMPOINT: u32 = 0x01A9;
    let hwnd = list.get_handle() as HWND;
    if hwnd.is_null() {
        return None;
    }
    let x = point.x.clamp(i16::MIN as i32, i16::MAX as i32) as i16 as u16 as u32;
    let y = point.y.clamp(i16::MIN as i32, i16::MAX as i32) as i16 as u16 as u32;
    let packed = (x | (y << 16)) as isize;
    let result = unsafe { SendMessageW(hwnd, LB_ITEMFROMPOINT, 0, packed) } as u32;
    let outside = (result >> 16) != 0;
    let row = result & 0xFFFF;
    (!outside && row < list.get_count()).then_some(row)
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn list_box_row_rect(list: ListBox, row: u32) -> Option<RECT> {
    const LB_GETITEMRECT: u32 = 0x0198;
    const LB_ERR: isize = -1;
    let hwnd = list.get_handle() as HWND;
    if hwnd.is_null() {
        return None;
    }
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let result = unsafe {
        SendMessageW(
            hwnd,
            LB_GETITEMRECT,
            row as usize,
            (&mut rect as *mut RECT) as isize,
        )
    };
    (result != LB_ERR).then_some(rect)
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn cursor_is_inside_window_client(handle: *mut std::ffi::c_void) -> bool {
    let hwnd = handle as HWND;
    if hwnd.is_null() {
        return false;
    }

    let mut point = POINT { x: 0, y: 0 };
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe {
        if GetCursorPos(&mut point) == 0
            || ScreenToClient(hwnd, &mut point) == 0
            || GetClientRect(hwnd, &mut rect) == 0
        {
            return false;
        }
    }

    point.x >= rect.left
        && point.x < rect.right
        && point.y >= rect.top
        && point.y < rect.bottom
}

#[cfg(not(target_os = "windows"))]
fn list_box_row_at_point(list: ListBox, _point: WxPoint) -> Option<u32> {
    // wxDragon does not expose wxListBox::HitTest on every backend. Preserve the existing useful
    // fallback outside Windows by using the selected row while the pointer is inside the list.
    list.get_selection()
}

/// Shows a complete label only while the ListBox row under the pointer is actually clipped.
///
/// The same OverflowTooltip driver is used on every platform. Windows prefers the viewer-owned
/// row-aligned popup; other platforms, and Windows if custom popup creation fails, use wxToolTip.
fn bind_list_box_overflow_tooltip(list: ListBox) {
    let fallback_list = list;
    let tooltip = Rc::new(RefCell::new(OverflowTooltip::new(
        list.get_handle(),
        move |text| fallback_list.set_tooltip(text),
    )));

    let tooltip_for_motion = Rc::clone(&tooltip);
    list.on_mouse_motion(move |event: WindowEventData| {
        let WindowEventData::MouseMotion(mouse) = &event else {
            event.skip(true);
            return;
        };
        let Some(point) = mouse.get_position() else {
            event.skip(true);
            return;
        };

        let reveal = list_box_row_at_point(list, point).and_then(|row| {
            let text = list.get_string(row)?;
            let available_width = list.get_client_size().width.saturating_sub(10);
            let text_width = control_text_width(list.get_handle(), &text, || {
                list.get_text_extent(&text).width
            });
            // Measurement failure must never suppress a reveal. This matters in particular for
            // accented labels that can be under-measured by wxDragon's narrow CString bridge.
            if !text_width.map_or(true, |width| width > available_width) {
                return None;
            }

            #[cfg(target_os = "windows")]
            let anchor = {
                let rect = list_box_row_rect(list, row)?;
                // LB_GETITEMRECT describes the whole row, while a standard native ListBox draws
                // its label slightly inset and vertically centred inside that row. Feed the label's
                // text origin directly to the custom popup so its painted text sits on the clipped line.
                let row_height = rect.bottom.saturating_sub(rect.top).max(1);
                let font_height = native_control_text_extent(list.get_handle(), "Mg")
                    .map_or_else(|| list.get_text_extent("Mg").height, |size| size.cy)
                    .max(1);
                let text_top = rect
                    .top
                    .saturating_add(row_height.saturating_sub(font_height) / 2);
                list.client_to_screen(WxPoint {
                    x: rect.left.saturating_add(2),
                    y: text_top,
                })
            };
            #[cfg(not(target_os = "windows"))]
            let anchor = list.client_to_screen(WxPoint { x: 0, y: point.y });

            Some((text, anchor))
        });
        tooltip_for_motion.borrow_mut().set_reveal(reveal);
        event.skip(true);
    });

    let tooltip_for_leave = Rc::clone(&tooltip);
    list.on_mouse_leave(move |_event: WindowEventData| {
        // The popup is hit-test transparent, but retain the physical-cursor guard as a defensive
        // fallback for Windows versions that still synthesize a leave while a top-level popup appears.
        #[cfg(target_os = "windows")]
        if cursor_is_inside_window_client(list.get_handle()) {
            return;
        }
        tooltip_for_leave.borrow_mut().set_reveal(None);
    });
}

/// Shows the same complete-label reveal for clipped Contents rows.
///
/// The native tree view's automatic label tooltip is detached on Windows so there is exactly one
/// tooltip mechanism. Clipping itself is determined from wxTreeCtrl's text-only bounding rectangle,
/// which already includes the item's real indentation and font metrics.
#[cfg_attr(target_os = "windows", allow(unsafe_code))]
fn bind_tree_overflow_tooltip(tree: TreeCtrl) {
    #[cfg(target_os = "windows")]
    {
        const TV_FIRST: u32 = 0x1100;
        const TVM_SETTOOLTIPS: u32 = TV_FIRST + 24;
        let tree_hwnd = tree.get_handle() as HWND;
        if !tree_hwnd.is_null() {
            unsafe {
                SendMessageW(tree_hwnd, TVM_SETTOOLTIPS, 0, 0);
            }
        }
    }

    let fallback_tree = tree;
    let tooltip = Rc::new(RefCell::new(OverflowTooltip::new(
        tree.get_handle(),
        move |text| fallback_tree.set_tooltip(text),
    )));

    let tooltip_for_motion = Rc::clone(&tooltip);
    tree.on_mouse_motion(move |event: WindowEventData| {
        let WindowEventData::MouseMotion(mouse) = &event else {
            event.skip(true);
            return;
        };
        let Some(point) = mouse.get_position() else {
            event.skip(true);
            return;
        };

        let reveal = tree.hit_test(point).0.and_then(|item| {
            let text = tree.get_item_text(&item)?;
            let bounds = tree.get_bounding_rect(&item, true)?;
            let client_width = tree.get_client_size().width;
            let right = bounds.x.saturating_add(bounds.width);
            if bounds.x >= 0 && right <= client_width.saturating_sub(4) {
                return None;
            }
            // textOnly=true already gives the TreeCtrl label rectangle, so its upper-left corner is
            // the exact text origin that the custom popup should reproduce.
            let anchor = tree.client_to_screen(WxPoint {
                x: bounds.x,
                y: bounds.y,
            });
            Some((text, anchor))
        });
        tooltip_for_motion.borrow_mut().set_reveal(reveal);
        event.skip(true);
    });

    let tooltip_for_leave = Rc::clone(&tooltip);
    tree.on_mouse_leave(move |_event: WindowEventData| {
        // The aligned popup overlays the TreeCtrl label, so any synthetic leave must not be
        // mistaken for leaving the Contents widget. Hide only after the cursor really exits it.
        #[cfg(target_os = "windows")]
        if cursor_is_inside_window_client(tree.get_handle()) {
            return;
        }
        tooltip_for_leave.borrow_mut().set_reveal(None);
    });
}

fn bind_navigation_overflow_tooltips(ui: &ViewerUi) {
    bind_tree_overflow_tooltip(ui.contents_tree);
    bind_list_box_overflow_tooltip(ui.index_list);
    bind_list_box_overflow_tooltip(ui.search_list);
    bind_list_box_overflow_tooltip(ui.bookmarks_list);
    bind_list_box_overflow_tooltip(ui.history_list);
}

fn refresh_bookmark_list(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    ui.bookmarks_list.clear();
    for bookmark in &state.borrow().bookmarks {
        ui.bookmarks_list.append(&bookmark.label);
    }
}

fn activate_bookmark_selection(ui: &Rc<ViewerUi>, state: &Rc<RefCell<ViewerState>>) {
    let Some(row) = ui.bookmarks_list.get_selection().map(|value| value as usize) else {
        return;
    };
    let destination = state
        .borrow()
        .bookmarks
        .get(row)
        .map(|bookmark| bookmark.location.clone());
    if let Some(destination) = destination {
        navigate_to_saved_location(ui, state, destination);
    }
}

fn refresh_history_list(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    let (locations, labels) = {
        let state_ref = state.borrow();
        match current_location(&state_ref) {
            None => (Vec::new(), Vec::new()),
            Some(current) => {
                let mut locations = Vec::new();
                let mut labels = Vec::new();
                for location in state_ref.history.back_locations() {
                    locations.push(location.clone());
                    labels.push(format!("← {}", navigation_location_label(&state_ref, location)));
                }
                locations.push(current.clone());
                labels.push(format!("• {}", navigation_location_label(&state_ref, &current)));
                for location in state_ref.history.forward_locations().iter().rev() {
                    locations.push(location.clone());
                    labels.push(format!("→ {}", navigation_location_label(&state_ref, location)));
                }
                (locations, labels)
            }
        }
    };
    ui.history_list.clear();
    for label in labels {
        ui.history_list.append(&label);
    }
    state.borrow_mut().history_visible = locations;
    refresh_browsing_toolbar(ui, state);
}

/// Synchronizes the remaining browse-strip controls with the active document and topic state.
fn refresh_browsing_toolbar(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    let (
        has_document,
        can_previous,
        can_next,
        macro_browse_tools_added,
        can_browse_previous,
        can_browse_next,
        navigation_pane_visible,
    ) = {
        let state = state.borrow();
        let topic_count = state
            .document
            .as_ref()
            .map_or(0, |document| document.presentations().len());
        let can_browse_previous = state
            .document
            .as_ref()
            .and_then(|document| document.browse_previous_index(state.topic_index))
            .is_some();
        let can_browse_next = state
            .document
            .as_ref()
            .and_then(|document| document.browse_next_index(state.topic_index))
            .is_some();
        (
            state.document.is_some(),
            state.document.is_some() && state.topic_index > 0,
            state.document.is_some() && state.topic_index.saturating_add(1) < topic_count,
            state.macro_browse_tools_added,
            can_browse_previous,
            can_browse_next,
            state.navigation_pane_visible,
        )
    };

    ui.toolbar.enable_tool(ID_PREVIOUS_TOPIC, can_previous);
    ui.toolbar.enable_tool(ID_NEXT_TOPIC, can_next);
    if macro_browse_tools_added {
        ui.toolbar.enable_tool(ID_BROWSE_PREVIOUS, can_browse_previous);
        ui.toolbar.enable_tool(ID_BROWSE_NEXT, can_browse_next);
    }
    ui.toolbar
        .toggle_tool(ID_TOGGLE_NAVIGATION, navigation_pane_visible);
    ui.browse_previous.enable(can_previous);
    ui.browse_next.enable(can_next);
    ui.browse_prev_seq.show(macro_browse_tools_added);
    ui.browse_next_seq.show(macro_browse_tools_added);
    ui.browse_prev_seq.enable(macro_browse_tools_added && can_browse_previous);
    ui.browse_next_seq.enable(macro_browse_tools_added && can_browse_next);
    ui.browse_toggle_navigation.enable(true);
    ui.browse_bar.layout();
    let text_zoom_percent = state.borrow().text_zoom_percent;
    let can_zoom_out = has_document && text_zoom_percent > MIN_TEXT_ZOOM_PERCENT;
    let can_zoom_in = has_document && text_zoom_percent < MAX_TEXT_ZOOM_PERCENT;
    ui.toolbar.enable_tool(ID_ZOOM_OUT, can_zoom_out);
    ui.toolbar.enable_tool(ID_ZOOM_IN, can_zoom_in);
    ui.browse_zoom_out.enable(can_zoom_out);
    ui.browse_zoom_in.enable(can_zoom_in);
}

/// Changes text zoom, then fully reflows so painting, wrapping, and hotspot hitboxes stay aligned.
fn adjust_text_zoom(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>, delta_percent: i32) {
    let changed = {
        let mut state = state.borrow_mut();
        if state.document.is_none() {
            return;
        }
        let next = state
            .text_zoom_percent
            .saturating_add(delta_percent)
            .clamp(MIN_TEXT_ZOOM_PERCENT, MAX_TEXT_ZOOM_PERCENT);
        if next == state.text_zoom_percent {
            false
        } else {
            state.text_zoom_percent = next;
            true
        }
    };

    if changed {
        refresh_topic_layout(ui, state);
        refresh_browsing_toolbar(ui, state);
        ui.scrolling_canvas.set_focus();
    }
}

/// Shows or hides the complete discovery/navigation side panel and reflows the topic body.
fn set_navigation_pane_visible(
    ui: &ViewerUi,
    state: &Rc<RefCell<ViewerState>>,
    visible: bool,
) {
    let currently_visible = state.borrow().navigation_pane_visible;
    if currently_visible == visible {
        return;
    }

    if visible {
        let width = state
            .borrow()
            .navigation_pane_width
            .max(NAVIGATION_PANE_MIN_WIDTH);
        let _ = ui
            .body_splitter
            .split_vertically(&ui.navigation_column, &ui.content_column, width);
    } else {
        let sash = ui.body_splitter.sash_position();
        if sash >= NAVIGATION_PANE_MIN_WIDTH {
            state.borrow_mut().navigation_pane_width = sash;
        }
        let _ = ui.body_splitter.unsplit(Some(&ui.navigation_column));
    }
    state.borrow_mut().navigation_pane_visible = visible;
    ui.toolbar.toggle_tool(ID_TOGGLE_NAVIGATION, visible);
    ui.frame.layout();
    layout_main_content_chrome(ui);

    if state.borrow().document.is_some() {
        refresh_topic_layout(ui, state);
    }
}

/// Shows or hides the complete discovery/navigation side panel and reflows the topic body.
fn toggle_navigation_pane(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    let visible = !state.borrow().navigation_pane_visible;
    set_navigation_pane_visible(ui, state, visible);
    ui.status_bar.set_status_text(
        if visible { "Navigation pane shown" } else { "Navigation pane hidden" },
        0,
    );
    ui.scrolling_canvas.set_focus();
}

fn navigation_location_label(state: &ViewerState, location: &NavigationLocation) -> String {
    let file = location
        .source_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("help file");
    let title = state
        .document
        .iter()
        .chain(state.navigation_document.iter())
        .chain(state.related_documents.iter())
        .find(|document| document.source_path() == location.source_path.as_path())
        .and_then(|document| document.presentations().get(location.topic_index))
        .map(|topic| topic_label(&topic.title, location.topic_index))
        .unwrap_or_else(|| format!("Topic {}", location.topic_index + 1));
    format!("{title} — {file}")
}

fn activate_visible_history_selection(ui: &Rc<ViewerUi>, state: &Rc<RefCell<ViewerState>>) {
    let Some(row) = ui.history_list.get_selection().map(|value| value as usize) else {
        return;
    };
    let destination = state.borrow().history_visible.get(row).cloned();
    if let Some(destination) = destination {
        navigate_to_saved_location(ui, state, destination);
    }
}

fn navigate_to_saved_location(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    destination: NavigationLocation,
) {
    let history_before = state.borrow().history.clone();
    let current = {
        let state_ref = state.borrow();
        current_location(&state_ref)
    };
    if let Some(current) = current {
        state.borrow_mut().history.visit(current, &destination);
    }
    if let Err(error) = restore_location(ui, state, &destination) {
        state.borrow_mut().history = history_before;
        MessageDialog::builder(&ui.frame, &error, "Navigation failed")
            .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
            .build()
            .show_modal();
    }
    refresh_history_list(ui, state);
}

fn topic_label(title: &str, topic_index: usize) -> String {
    let title = title.trim();
    if title.is_empty() {
        format!("Topic {}", topic_index + 1)
    } else {
        title.to_owned()
    }
}

/// Binds paint dispatch for either the non-scrolling or scrolling retained region.
fn bind_paint_handler(canvas: Panel, state: Rc<RefCell<ViewerState>>, fixed_region: bool) {
    canvas.on_paint(move |_event: WindowEventData| {
        // Must run before PaintDC::new(): BeginPaint validates the update region, so afterwards
        // the region can no longer be widened. See invalidate_whole_canvas.
        invalidate_whole_canvas(canvas);
        let dc = PaintDC::new(&canvas);
        dc.set_background(colour_from_rgb(HELP_BACKGROUND));
        dc.clear();
        dc.set_background_mode(wxdragon::dc::BackgroundMode::Transparent);

        let state = state.borrow();
        let Some(layout) = &state.layout else {
            if !fixed_region {
                paint_welcome(&dc);
            }
            return;
        };
        let region = if fixed_region { &layout.fixed } else { &layout.scrolling };
        let region_kind = TopicRegionKind::from_fixed_region(fixed_region);
        let selection = state
            .topic_selection
            .filter(|selection| selection.region == region_kind && !selection.is_empty());
        paint_region_with_selection(
            canvas,
            &dc,
            region,
            state.text_zoom_percent,
            HELP_BACKGROUND,
            selection,
        );
    });
}

/// Returns one retained region from a topic layout.
fn topic_region(layout: &TopicLayout, region: TopicRegionKind) -> &RegionLayout {
    match region {
        TopicRegionKind::Fixed => &layout.fixed,
        TopicRegionKind::Scrolling => &layout.scrolling,
    }
}

fn measure_text_width(
    canvas: Panel,
    native_text: &NativeTextContext,
    style: &ResolvedTextStyle,
    text: &str,
    text_zoom_percent: i32,
) -> i32 {
    native_text
        .measure(style, text, text_zoom_percent)
        .unwrap_or_else(|| wx_text_metrics(canvas, style, text, text_zoom_percent))
        .width
}

/// Maps a mouse point to the nearest UTF-8 boundary in a retained text run.
fn topic_text_position_at_point(
    canvas: Panel,
    region: &RegionLayout,
    point: LayoutPoint,
    text_zoom_percent: i32,
    allow_nearest_line: bool,
) -> Option<TopicTextPosition> {
    let mut candidates = region
        .boxes
        .iter()
        .enumerate()
        .filter_map(|(box_index, item)| match &item.kind {
            LayoutKind::Text { text, style, .. } if !text.is_empty() => {
                Some((box_index, item, text.as_str(), style))
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    if candidates.is_empty() {
        return None;
    }

    let vertical_distance = |bounds: LayoutRect| -> i32 {
        if point.y < bounds.y {
            bounds.y.saturating_sub(point.y)
        } else if point.y >= bounds.y.saturating_add(bounds.height) {
            point.y.saturating_sub(bounds.y.saturating_add(bounds.height).saturating_sub(1))
        } else {
            0
        }
    };

    candidates.sort_by_key(|(_, item, _, _)| {
        let vertical = vertical_distance(item.bounds);
        let horizontal = if point.x < item.bounds.x {
            item.bounds.x.saturating_sub(point.x)
        } else if point.x > item.bounds.x.saturating_add(item.bounds.width) {
            point.x.saturating_sub(item.bounds.x.saturating_add(item.bounds.width))
        } else {
            0
        };
        (vertical, horizontal)
    });

    let (box_index, item, text, style) = candidates[0];
    if vertical_distance(item.bounds) != 0 && !allow_nearest_line {
        return None;
    }

    if point.x <= item.bounds.x {
        return Some(TopicTextPosition { box_index, byte_offset: 0 });
    }
    if point.x >= item.bounds.x.saturating_add(item.bounds.width) {
        return Some(TopicTextPosition { box_index, byte_offset: text.len() });
    }

    let native_text = NativeTextContext::new(canvas);
    let target_x = point.x.saturating_sub(item.bounds.x);
    let mut previous_offset = 0usize;
    let mut previous_width = 0i32;
    for (offset, _) in text.char_indices().skip(1) {
        let width = measure_text_width(canvas, &native_text, style, &text[..offset], text_zoom_percent);
        let midpoint = previous_width.saturating_add(width).saturating_div(2);
        if target_x < midpoint {
            return Some(TopicTextPosition { box_index, byte_offset: previous_offset });
        }
        previous_offset = offset;
        previous_width = width;
    }

    let total_width = measure_text_width(canvas, &native_text, style, text, text_zoom_percent);
    let midpoint = previous_width.saturating_add(total_width).saturating_div(2);
    Some(TopicTextPosition {
        box_index,
        byte_offset: if target_x < midpoint { previous_offset } else { text.len() },
    })
}

fn selection_byte_range_for_box(
    text_len: usize,
    box_index: usize,
    start: TopicTextPosition,
    end: TopicTextPosition,
) -> Option<(usize, usize)> {
    if box_index < start.box_index || box_index > end.box_index {
        return None;
    }
    let from = if box_index == start.box_index { start.byte_offset.min(text_len) } else { 0 };
    let to = if box_index == end.box_index { end.byte_offset.min(text_len) } else { text_len };
    (from < to).then_some((from, to))
}

fn selected_topic_text(region: &RegionLayout, selection: TopicTextSelection) -> String {
    let (start, end) = selection.ordered();
    let mut output = String::new();
    let mut previous_y = None;
    for (box_index, item) in region.boxes.iter().enumerate() {
        let LayoutKind::Text { text, .. } = &item.kind else {
            continue;
        };
        let Some((from, to)) = selection_byte_range_for_box(text.len(), box_index, start, end) else {
            continue;
        };
        if !text.is_char_boundary(from) || !text.is_char_boundary(to) {
            continue;
        }
        if let Some(y) = previous_y {
            if y != item.bounds.y {
                output.push_str("\r\n");
            }
        }
        output.push_str(&text[from..to]);
        previous_y = Some(item.bounds.y);
    }
    output
}

fn select_all_topic_region(state: &mut ViewerState, region_kind: TopicRegionKind) -> bool {
    let Some(layout) = &state.layout else {
        return false;
    };
    let region = topic_region(layout, region_kind);
    let first = region.boxes.iter().enumerate().find_map(|(box_index, item)| match &item.kind {
        LayoutKind::Text { text, .. } if !text.is_empty() => Some(TopicTextPosition { box_index, byte_offset: 0 }),
        _ => None,
    });
    let last = region.boxes.iter().enumerate().rev().find_map(|(box_index, item)| match &item.kind {
        LayoutKind::Text { text, .. } if !text.is_empty() => Some(TopicTextPosition { box_index, byte_offset: text.len() }),
        _ => None,
    });
    match (first, last) {
        (Some(anchor), Some(focus)) => {
            state.topic_selection = Some(TopicTextSelection { region: region_kind, anchor, focus });
            true
        }
        _ => false,
    }
}

fn refresh_topic_region(ui: &ViewerUi, region: TopicRegionKind) {
    match region {
        TopicRegionKind::Fixed => ui.fixed_canvas.refresh(true, None),
        TopicRegionKind::Scrolling => ui.scrolling_canvas.refresh(true, None),
    }
}

/// Binds click hit-testing, mouse-drag text selection, and hyperlink activation.
/// Hotspots activate on mouse-up only when the gesture did not become a text-selection drag.
fn bind_hotspot_handler(ui: Rc<ViewerUi>, state: Rc<RefCell<ViewerState>>, fixed_region: bool) {
    let canvas = if fixed_region { ui.fixed_canvas } else { ui.scrolling_canvas };
    let region_kind = TopicRegionKind::from_fixed_region(fixed_region);
    let pointer = Rc::new(RefCell::new(TopicPointerState::default()));

    let pointer_for_down = Rc::clone(&pointer);
    let ui_for_down = Rc::clone(&ui);
    let state_for_down = Rc::clone(&state);
    canvas.on_mouse_left_down(move |event: WindowEventData| {
        dismiss_main_transients(&ui_for_down, &state_for_down);
        canvas.set_focus();
        let WindowEventData::MouseButton(mouse) = event else {
            return;
        };
        let Some(position) = mouse.get_position() else {
            return;
        };
        let point = LayoutPoint { x: position.x, y: position.y };
        let (text_position, hotspot) = {
            let state = state_for_down.borrow();
            let Some(layout) = &state.layout else {
                return;
            };
            let region = topic_region(layout, region_kind);
            let text_position = topic_text_position_at_point(
                canvas,
                region,
                point,
                state.text_zoom_percent,
                false,
            );
            let hotspot = region
                .hit_test_box(point)
                .and_then(|item| item.hotspot().cloned().map(|hotspot| (item.bounds, hotspot)));
            (text_position, hotspot)
        };

        {
            let mut state = state_for_down.borrow_mut();
            state.edit_target = EditTarget::Topic(region_kind);
            state.topic_selection = text_position.map(|position| TopicTextSelection {
                region: region_kind,
                anchor: position,
                focus: position,
            });
        }
        {
            let mut pointer = pointer_for_down.borrow_mut();
            pointer.anchor = text_position;
            pointer.dragged = false;
            pointer.pressed_hotspot = hotspot;
        }
        if text_position.is_some() || pointer_for_down.borrow().pressed_hotspot.is_some() {
            canvas.capture_mouse();
        }
        canvas.refresh(true, None);
    });

    let pointer_for_motion = Rc::clone(&pointer);
    let state_for_motion = Rc::clone(&state);
    canvas.on_mouse_motion(move |event: WindowEventData| {
        let WindowEventData::MouseMotion(mouse) = &event else {
            event.skip(true);
            return;
        };
        let anchor = pointer_for_motion.borrow().anchor;
        if let (Some(anchor), Some(position)) = (anchor, mouse.get_position()) {
            let focus = {
                let state = state_for_motion.borrow();
                state.layout.as_ref().and_then(|layout| {
                    topic_text_position_at_point(
                        canvas,
                        topic_region(layout, region_kind),
                        LayoutPoint { x: position.x, y: position.y },
                        state.text_zoom_percent,
                        true,
                    )
                })
            };
            if let Some(focus) = focus {
                {
                    let mut pointer = pointer_for_motion.borrow_mut();
                    if focus != anchor {
                        pointer.dragged = true;
                    }
                }
                state_for_motion.borrow_mut().topic_selection = Some(TopicTextSelection {
                    region: region_kind,
                    anchor,
                    focus,
                });
                canvas.refresh(true, None);
            }
        }
        // The independent hover-preview handler also listens for motion on this canvas.
        event.skip(true);
    });

    let pointer_for_up = Rc::clone(&pointer);
    let ui_for_up = Rc::clone(&ui);
    let state_for_up = Rc::clone(&state);
    canvas.on_mouse_left_up(move |event: WindowEventData| {
        if canvas.has_capture() {
            canvas.release_mouse();
        }
        let release_point = match &event {
            WindowEventData::MouseButton(mouse) => mouse
                .get_position()
                .map(|position| LayoutPoint { x: position.x, y: position.y }),
            _ => None,
        };
        let (dragged, pressed_hotspot) = {
            let pointer = pointer_for_up.borrow();
            (pointer.dragged, pointer.pressed_hotspot.clone())
        };

        if !dragged {
            if let (Some(point), Some((bounds, hotspot))) = (release_point, pressed_hotspot) {
                if bounds.contains(point) {
                    state_for_up.borrow_mut().topic_selection = None;
                    ui_for_up.status_bar.set_status_text(&describe_hotspot(&hotspot), 0);
                    let anchor = canvas.client_to_screen(WxPoint {
                        x: bounds.x.saturating_add(12),
                        y: bounds.y.saturating_add(bounds.height).saturating_add(4),
                    });
                    activate_hotspot(&ui_for_up, &state_for_up, &hotspot, Some(anchor));
                }
            }
        }

        *pointer_for_up.borrow_mut() = TopicPointerState::default();
        canvas.refresh(true, None);
    });
}

/// Tracks which editable/read-only surface owns Edit menu operations.
fn bind_edit_focus_tracking(ui: &Rc<ViewerUi>, state: &Rc<RefCell<ViewerState>>) {
    let state_for_fixed = Rc::clone(state);
    ui.fixed_canvas.on_set_focus(move |event: WindowEventData| {
        state_for_fixed.borrow_mut().edit_target = EditTarget::Topic(TopicRegionKind::Fixed);
        event.skip(true);
    });

    let state_for_scrolling = Rc::clone(state);
    ui.scrolling_canvas.on_set_focus(move |event: WindowEventData| {
        state_for_scrolling.borrow_mut().edit_target = EditTarget::Topic(TopicRegionKind::Scrolling);
        event.skip(true);
    });

    let state_for_index = Rc::clone(state);
    ui.index_query.on_set_focus(move |event: WindowEventData| {
        state_for_index.borrow_mut().edit_target = EditTarget::IndexQuery;
        event.skip(true);
    });

    let state_for_search = Rc::clone(state);
    ui.search_query.on_set_focus(move |event: WindowEventData| {
        state_for_search.borrow_mut().edit_target = EditTarget::SearchQuery;
        event.skip(true);
    });
}

#[allow(unsafe_code)]
fn clipboard_set_text(text: &str) -> Result<(), String> {
    let text = CString::new(text).map_err(|_| "Selected text contains an embedded NUL character".to_owned())?;
    unsafe {
        let clipboard = wxdragon::ffi::wxd_Clipboard_Get();
        if clipboard.is_null() {
            return Err("System clipboard is unavailable".to_owned());
        }
        let was_open = wxdragon::ffi::wxd_Clipboard_IsOpened(clipboard);
        if !was_open && !wxdragon::ffi::wxd_Clipboard_Open(clipboard) {
            return Err("Could not open the system clipboard".to_owned());
        }
        let success = wxdragon::ffi::wxd_Clipboard_SetText(clipboard, text.as_ptr());
        if !was_open {
            wxdragon::ffi::wxd_Clipboard_Close(clipboard);
        }
        if success {
            Ok(())
        } else {
            Err("Could not write text to the system clipboard".to_owned())
        }
    }
}

#[allow(unsafe_code)]
fn clipboard_get_text() -> Result<Option<String>, String> {
    unsafe {
        let clipboard = wxdragon::ffi::wxd_Clipboard_Get();
        if clipboard.is_null() {
            return Err("System clipboard is unavailable".to_owned());
        }
        let was_open = wxdragon::ffi::wxd_Clipboard_IsOpened(clipboard);
        if !was_open && !wxdragon::ffi::wxd_Clipboard_Open(clipboard) {
            return Err("Could not open the system clipboard".to_owned());
        }

        let required = wxdragon::ffi::wxd_Clipboard_GetText(clipboard, std::ptr::null_mut(), 0);
        if required < 0 {
            if !was_open {
                wxdragon::ffi::wxd_Clipboard_Close(clipboard);
            }
            return Ok(None);
        }
        let mut buffer = vec![0 as c_char; required as usize + 1];
        let read = wxdragon::ffi::wxd_Clipboard_GetText(clipboard, buffer.as_mut_ptr(), buffer.len());
        if !was_open {
            wxdragon::ffi::wxd_Clipboard_Close(clipboard);
        }
        if read < 0 {
            return Err("Could not read text from the system clipboard".to_owned());
        }
        let text = CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned();
        Ok(Some(text))
    }
}

fn copy_edit_selection(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    let target = state.borrow().edit_target;
    let text = match target {
        EditTarget::Topic(region_kind) => {
            let state = state.borrow();
            let Some(selection) = state
                .topic_selection
                .filter(|selection| selection.region == region_kind && !selection.is_empty())
            else {
                ui.status_bar.set_status_text("Nothing is selected to copy.", 0);
                return;
            };
            let Some(layout) = &state.layout else {
                ui.status_bar.set_status_text("Nothing is selected to copy.", 0);
                return;
            };
            selected_topic_text(topic_region(layout, region_kind), selection)
        }
        EditTarget::IndexQuery => ui.index_query.get_string_selection(),
        EditTarget::SearchQuery => ui.search_query.get_string_selection(),
    };

    if text.is_empty() {
        ui.status_bar.set_status_text("Nothing is selected to copy.", 0);
        return;
    }
    match clipboard_set_text(&text) {
        Ok(()) => ui.status_bar.set_status_text(&format!("Copied {} characters.", text.chars().count()), 0),
        Err(error) => ui.status_bar.set_status_text(&format!("Copy failed: {error}"), 0),
    }
}

fn paste_edit_selection(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    let target = state.borrow().edit_target;
    if matches!(target, EditTarget::Topic(_)) {
        ui.status_bar.set_status_text("Help topics are read-only; paste is available in Index and Search.", 0);
        return;
    }
    let text = match clipboard_get_text() {
        Ok(Some(text)) if !text.is_empty() => text,
        Ok(_) => {
            ui.status_bar.set_status_text("Clipboard does not contain text.", 0);
            return;
        }
        Err(error) => {
            ui.status_bar.set_status_text(&format!("Paste failed: {error}"), 0);
            return;
        }
    };

    let control = match target {
        EditTarget::IndexQuery => ui.index_query,
        EditTarget::SearchQuery => ui.search_query,
        EditTarget::Topic(_) => unreachable!(),
    };
    let (from, to) = control.get_selection();
    control.replace(from, to, &text);
    ui.status_bar.set_status_text(&format!("Pasted {} characters.", text.chars().count()), 0);
}

fn select_all_edit_target(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    let target = state.borrow().edit_target;
    match target {
        EditTarget::IndexQuery => ui.index_query.select_all(),
        EditTarget::SearchQuery => ui.search_query.select_all(),
        EditTarget::Topic(region_kind) => {
            let selected = select_all_topic_region(&mut state.borrow_mut(), region_kind);
            if selected {
                refresh_topic_region(ui, region_kind);
            } else {
                ui.status_bar.set_status_text("There is no text to select in this topic region.", 0);
            }
        }
    }
}


/// Tracks the hyperlink currently under a topic canvas so native tooltip text is only replaced
/// when the hit target actually changes. This avoids restarting the wxWidgets tooltip timer on
/// every mouse-motion event and avoids reopening an external HLP repeatedly while hovering.
struct HoverTooltipState {
    hotspot: Option<Hotspot>,
    generation: u64,
    #[cfg(target_os = "windows")]
    native_tooltip: Option<NativeHotspotTooltip>,
}

impl HoverTooltipState {
    fn for_canvas(canvas: Panel) -> Self {
        Self {
            hotspot: None,
            generation: 0,
            #[cfg(target_os = "windows")]
            native_tooltip: NativeHotspotTooltip::new(canvas.get_handle()),
        }
    }
}

/// Adds hyperlink hover previews to a main-window topic region.
///
/// Ordinary jumps expose the resolved destination title. Popup hotspots expose the popup topic's
/// actual visible text, which is the useful payload for untitled glossary/context popups. This hover
/// UI remains independent from build-fix 16's single-surface click-routing policy.
fn bind_main_hotspot_tooltip(
    canvas: Panel,
    state: Rc<RefCell<ViewerState>>,
    fixed_region: bool,
) {
    let hover = Rc::new(RefCell::new(HoverTooltipState::for_canvas(canvas)));
    let hover_for_motion = Rc::clone(&hover);
    let state_for_motion = Rc::clone(&state);
    canvas.on_mouse_motion(move |event: WindowEventData| {
        let WindowEventData::MouseMotion(mouse) = event else {
            return;
        };
        let Some(position) = mouse.get_position() else {
            return;
        };
        let (document, generation, hit) = {
            let state = state_for_motion.borrow();
            let Some(document) = state.document.clone() else {
                clear_hotspot_tooltip(canvas, &hover_for_motion);
                return;
            };
            let Some(layout) = &state.layout else {
                clear_hotspot_tooltip(canvas, &hover_for_motion);
                return;
            };
            let region = if fixed_region {
                &layout.fixed
            } else {
                &layout.scrolling
            };
            let hit = region
                .hit_test_box(LayoutPoint {
                    x: position.x,
                    y: position.y,
                })
                .and_then(|item| item.hotspot().cloned());
            (document, state.tooltip_generation, hit)
        };
        update_hotspot_tooltip(canvas, &hover_for_motion, &document, generation, hit);
    });

    let hover_for_leave = Rc::clone(&hover);
    canvas.on_mouse_leave(move |_event: WindowEventData| {
        clear_hotspot_tooltip(canvas, &hover_for_leave);
    });
}

/// Legacy auxiliary-surface tooltip binder. It is unreachable while single-surface routing is
/// enabled, and is intentionally not used to recreate floating topic windows.
fn bind_auxiliary_hotspot_tooltip(
    canvas: Panel,
    state: Rc<RefCell<AuxiliaryState>>,
    fixed_region: bool,
) {
    let hover = Rc::new(RefCell::new(HoverTooltipState::for_canvas(canvas)));
    let hover_for_motion = Rc::clone(&hover);
    let state_for_motion = Rc::clone(&state);
    canvas.on_mouse_motion(move |event: WindowEventData| {
        let WindowEventData::MouseMotion(mouse) = event else {
            return;
        };
        let Some(position) = mouse.get_position() else {
            return;
        };
        let (document, generation, hit) = {
            let state = state_for_motion.borrow();
            let Some(layout) = &state.layout else {
                clear_hotspot_tooltip(canvas, &hover_for_motion);
                return;
            };
            let region = if fixed_region {
                &layout.fixed
            } else {
                &layout.scrolling
            };
            let hit = region
                .hit_test_box(LayoutPoint {
                    x: position.x,
                    y: position.y,
                })
                .and_then(|item| item.hotspot().cloned());
            (state.document.clone(), state.tooltip_generation, hit)
        };
        update_hotspot_tooltip(canvas, &hover_for_motion, &document, generation, hit);
    });

    let hover_for_leave = Rc::clone(&hover);
    canvas.on_mouse_leave(move |_event: WindowEventData| {
        clear_hotspot_tooltip(canvas, &hover_for_leave);
    });
}

/// Replaces the hotspot preview only after the hovered WinHelp target changes.
fn update_hotspot_tooltip(
    canvas: Panel,
    hover: &Rc<RefCell<HoverTooltipState>>,
    current_document: &HelpDocument,
    generation: u64,
    hotspot: Option<Hotspot>,
) {
    {
        let hover = hover.borrow();
        if hover.generation == generation && hover.hotspot == hotspot {
            return;
        }
    }

    let text = hotspot
        .as_ref()
        .and_then(|hotspot| hotspot_destination_tooltip(current_document, hotspot));

    let mut hover = hover.borrow_mut();
    #[cfg(target_os = "windows")]
    if let Some(native_tooltip) = hover.native_tooltip.as_mut() {
        native_tooltip.set_text(text.as_deref());
    } else {
        // Defensive fallback if the dedicated common-control window could not be created.
        canvas.set_tooltip(text.as_deref().unwrap_or(""));
        if text.is_some() {
            apply_windows_tooltip_palette(canvas.get_handle());
        }
    }
    #[cfg(not(target_os = "windows"))]
    canvas.set_tooltip(text.as_deref().unwrap_or(""));

    hover.hotspot = hotspot;
    hover.generation = generation;
}

/// Removes a hotspot tooltip immediately when the pointer leaves the canvas or ordinary text is
/// entered.
fn clear_hotspot_tooltip(canvas: Panel, hover: &Rc<RefCell<HoverTooltipState>>) {
    if hover.borrow().hotspot.is_none() {
        return;
    }
    let mut hover = hover.borrow_mut();
    #[cfg(target_os = "windows")]
    if let Some(native_tooltip) = hover.native_tooltip.as_mut() {
        native_tooltip.set_text(None);
    } else {
        canvas.set_tooltip("");
    }
    #[cfg(not(target_os = "windows"))]
    canvas.set_tooltip("");
    hover.hotspot = None;
}

/// Resolves a safe navigation hotspot to its hover preview. Ordinary navigation links expose the
/// destination title; popup links expose the popup topic body. Executable macro hotspots
/// deliberately return no tooltip because they have no safe topic destination and remain blocked
/// by the existing activation path.
fn hotspot_destination_tooltip(
    current_document: &HelpDocument,
    hotspot: &Hotspot,
) -> Option<String> {
    match &hotspot.target {
        HotspotTarget::Internal { offset, popup } => {
            destination_topic_tooltip(current_document, *offset, *popup)
        }
        HotspotTarget::ContextHash { hash, popup } => {
            let topic_index = current_document
                .topic_index_for_context_hash(*hash)
                .or_else(|| current_document.resolve_topic_offset(TopicOffset(*hash)))?;
            destination_topic_tooltip_by_index(current_document, topic_index, *popup)
        }
        HotspotTarget::External {
            opcode,
            offset,
            help_file,
            ..
        } => {
            let target_document = load_linked_document(current_document, help_file.as_deref()).ok()?;
            destination_topic_tooltip(&target_document, *offset, opcode & 1 == 0)
        }
        HotspotTarget::Macro(_) => None,
    }
}

/// Builds one resolved hover preview. Popup targets prefer their actual topic body over the
/// synthetic/fallback topic label; ordinary jumps retain the compact destination-title tooltip.
fn destination_topic_tooltip(
    document: &HelpDocument,
    offset: TopicOffset,
    popup: bool,
) -> Option<String> {
    let topic_index = document.resolve_topic_offset(offset)?;
    destination_topic_tooltip_by_index(document, topic_index, popup)
}

fn destination_topic_tooltip_by_index(
    document: &HelpDocument,
    topic_index: usize,
    popup: bool,
) -> Option<String> {
    let presentation = document.presentations().get(topic_index)?;
    let title = topic_label(&presentation.title, topic_index);
    let popup_body = if popup {
        popup_topic_tooltip_body(presentation)
    } else {
        None
    };
    Some(format_destination_tooltip(&title, popup_body.as_deref(), popup))
}

/// Extracts the visible text from the formatting-decoded presentation used by the renderer.
/// Paragraph boundaries, explicit line breaks, and tabs are kept so a native multi-line tooltip
/// resembles the authored popup instead of exposing an internal topic number. Pictures and other
/// non-text inline objects intentionally contribute no fake description.
fn popup_topic_tooltip_body(presentation: &hlp::TopicPresentation) -> Option<String> {
    let mut text = String::new();
    let mut paragraph_count = 0_usize;

    for record in presentation
        .non_scrolling
        .iter()
        .chain(presentation.scrolling.iter())
    {
        for paragraph in &record.paragraphs {
            if paragraph_count != 0 {
                text.push('\n');
            }
            paragraph_count += 1;

            for inline in &paragraph.inlines {
                match inline {
                    hlp::Inline::Text(run) => text.push_str(&run.text),
                    hlp::Inline::LineBreak => text.push('\n'),
                    hlp::Inline::Tab => text.push('\t'),
                    hlp::Inline::Control85(_)
                    | hlp::Inline::Picture(_)
                    | hlp::Inline::EmbeddedWindow(_) => {}
                }
            }
        }
    }

    normalize_popup_tooltip_body(&text)
}

/// Chooses the user-facing hover text without exposing parser bookkeeping such as `Topic 6`.
/// A popup with actual visible text uses that content. A truly empty popup falls back to the
/// resolved title rather than presenting a blank native tooltip.
fn format_destination_tooltip(title: &str, popup_body: Option<&str>, popup: bool) -> String {
    if popup {
        if let Some(body) = popup_body.and_then(normalize_popup_tooltip_body) {
            return body;
        }
    }
    title.to_owned()
}

/// Normalizes line endings and trims only outer whitespace so authored multi-line popup text is
/// preserved in the native tooltip.
fn normalize_popup_tooltip_body(body: &str) -> Option<String> {
    let normalized = body.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = normalized.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Removes any native destination-title tooltip currently owned by the main topic canvases.
/// Incrementing the generation also invalidates the private hover cache in both canvas handlers,
/// so moving over the same hotspot after a topic change can create a fresh tooltip.
fn dismiss_main_hover_tooltips(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    ui.fixed_canvas.set_tooltip("");
    ui.scrolling_canvas.set_tooltip("");
    let mut state = state.borrow_mut();
    state.tooltip_generation = state.tooltip_generation.wrapping_add(1);
}

/// Closes the one transient WinHelp popup tracked by the main viewer.
/// Secondary windows are deliberately unaffected.
fn dismiss_active_popup(state: &Rc<RefCell<ViewerState>>) {
    let popup = state.borrow_mut().active_popup.take();
    if let Some(frame) = popup {
        let _ = frame.close(true);
    }
}

/// Dismisses transient UI before a main-window click or main-topic navigation.
fn dismiss_main_transients(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    dismiss_main_hover_tooltips(ui, state);
    dismiss_active_popup(state);
}

/// Binds outside-click dismissal to the main surfaces that can consume mouse input before it
/// reaches the frame. Every handler calls Skip so the original control still receives the click.
fn bind_main_transient_dismissal<T>(
    widget: T,
    ui: Rc<ViewerUi>,
    state: Rc<RefCell<ViewerState>>,
) where
    T: WxWidget + WindowEvents + Copy + 'static,
{
    widget.on_mouse_left_down(move |event: WindowEventData| {
        dismiss_main_transients(&ui, &state);
        event.skip(true);
    });
}

fn bind_main_transient_dismissal_surfaces(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
) {
    macro_rules! bind {
        ($widget:expr) => {
            bind_main_transient_dismissal($widget, Rc::clone(ui), Rc::clone(state));
        };
    }

    bind!(ui.frame);
    bind!(ui.toolbar);
    bind!(ui.body_splitter);
    bind!(ui.navigation_column);
    bind!(ui.content_column);
    bind!(ui.browse_bar);
    bind!(ui.browse_previous);
    bind!(ui.browse_next);
    bind!(ui.browse_prev_seq);
    bind!(ui.browse_next_seq);
    bind!(ui.browse_toggle_navigation);
    bind!(ui.browse_zoom_out);
    bind!(ui.browse_zoom_in);
    bind!(ui.navigation);
    bind!(ui.contents_hierarchical);
    bind!(ui.contents_show_all);
    bind!(ui.contents_tree);
    bind!(ui.index_query);
    bind!(ui.index_list);
    bind!(ui.search_query);
    bind!(ui.search_list);
    bind!(ui.bookmark_add);
    bind!(ui.bookmark_remove);
    bind!(ui.bookmarks_list);
    bind!(ui.history_list);
    bind!(ui.content_host);
    bind!(ui.page_border);
    bind!(ui.page_inner);
    // The topic canvases already dismiss in bind_hotspot_handler(), before hotspot activation.
}

/// Shows the native file picker and replaces the current document when selection succeeds.

/// Chooses which presentation topics should be printed, then sends their retained formatting to the
/// native printer backend. Topic numbers are one-based in the UI and follow physical presentation
/// order, matching the Contents "Show all" view.
fn print_topics(ui: &Rc<ViewerUi>, state: &Rc<RefCell<ViewerState>>) {
    let (topic_count, current_topic_index) = {
        let state = state.borrow();
        let Some(document) = state.document.as_ref() else {
            MessageDialog::builder(&ui.frame, "Open a help file before printing.", "Print")
                .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
                .build()
                .show_modal();
            return;
        };
        (document.presentations().len(), state.topic_index)
    };

    if topic_count == 0 {
        MessageDialog::builder(&ui.frame, "This help file contains no printable topics.", "Print")
            .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
            .build()
            .show_modal();
        return;
    }

    let Some(topic_indices) = choose_print_topic_indices(ui, current_topic_index, topic_count) else {
        ui.status_bar.set_status_text("Printing cancelled", 0);
        return;
    };

    let (topics, fonts) = {
        let state = state.borrow();
        let Some(document) = state.document.as_ref() else {
            return;
        };
        let topics = topic_indices
            .iter()
            .filter_map(|&index| {
                document.presentations().get(index).cloned().map(|presentation| PrintableTopic {
                    source_index: index,
                    presentation,
                })
            })
            .collect::<Vec<_>>();
        (topics, document.fonts().clone())
    };

    if topics.is_empty() {
        return;
    }

    match native_print_topics(ui.frame.get_handle(), &topics, &fonts) {
        Ok(PrintOutcome::Printed) => {
            let label = if topics.len() == 1 { "Topic" } else { "Topics" };
            ui.status_bar
                .set_status_text(&format!("{label} sent to printer"), 0);
        }
        Ok(PrintOutcome::Cancelled) => ui.status_bar.set_status_text("Printing cancelled", 0),
        Err(error) => {
            MessageDialog::builder(&ui.frame, &error, "Print failed")
                .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
                .build()
                .show_modal();
        }
    }
}

#[derive(Debug, Clone)]
struct PrintableTopic {
    source_index: usize,
    presentation: hlp::TopicPresentation,
}

fn choose_print_topic_indices(
    ui: &ViewerUi,
    current_topic_index: usize,
    topic_count: usize,
) -> Option<Vec<usize>> {
    let choices = [
        format!("Current topic ({})", current_topic_index.saturating_add(1)),
        "Topic range...".to_owned(),
        format!("All topics (1-{topic_count})"),
    ];
    let refs = choices.iter().map(String::as_str).collect::<Vec<_>>();
    let dialog = SingleChoiceDialog::builder(
        &ui.frame,
        "Choose the topics to print.",
        "Print Topics",
        &refs,
    )
    .build();
    if dialog.show_modal() != wxdragon::id::ID_OK {
        return None;
    }

    match dialog.get_selection() {
        0 => Some(vec![current_topic_index.min(topic_count.saturating_sub(1))]),
        1 => choose_print_topic_range(ui, current_topic_index, topic_count),
        2 => Some((0..topic_count).collect()),
        _ => None,
    }
}

fn choose_print_topic_range(
    ui: &ViewerUi,
    current_topic_index: usize,
    topic_count: usize,
) -> Option<Vec<usize>> {
    let default_value = current_topic_index
        .min(topic_count.saturating_sub(1))
        .saturating_add(1)
        .to_string();
    loop {
        let dialog = TextEntryDialog::builder(
            &ui.frame,
            &format!(
                "Enter topic numbers or ranges from 1 to {topic_count}.\n\nExamples: 3-8   or   1-3, 7, 10-12"
            ),
            "Print Topic Range",
        )
        .with_default_value(&default_value)
        .build();
        if dialog.show_modal() != wxdragon::id::ID_OK {
            return None;
        }
        let Some(value) = dialog.get_value() else {
            return None;
        };
        match parse_topic_range_spec(&value, topic_count) {
            Ok(indices) => return Some(indices),
            Err(error) => {
                MessageDialog::builder(&ui.frame, &error, "Invalid topic range")
                    .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
                    .build()
                    .show_modal();
            }
        }
    }
}

/// Parses one-based topic selections such as `3-8` or `1-3, 7, 10-12` into sorted zero-based
/// presentation indices. Overlapping entries are deliberately de-duplicated.
fn parse_topic_range_spec(spec: &str, topic_count: usize) -> Result<Vec<usize>, String> {
    if topic_count == 0 {
        return Err("This help file contains no printable topics.".to_owned());
    }
    let mut selected = BTreeSet::new();
    for raw_token in spec.split(|ch| ch == ',' || ch == ';') {
        let token = raw_token.trim();
        if token.is_empty() {
            continue;
        }
        let mut range = token.split('-').map(str::trim);
        let first = range.next().unwrap_or_default();
        let second = range.next();
        if range.next().is_some() || first.is_empty() || second.is_some_and(str::is_empty) {
            return Err(format!("'{token}' is not a valid topic number or range."));
        }

        let start = parse_print_topic_number(first, topic_count)?;
        let end = match second {
            Some(value) => parse_print_topic_number(value, topic_count)?,
            None => start,
        };
        if start > end {
            return Err(format!("Topic range {start}-{end} runs backwards."));
        }
        for topic_number in start..=end {
            selected.insert(topic_number - 1);
        }
    }

    if selected.is_empty() {
        return Err("Enter at least one topic number or range.".to_owned());
    }
    Ok(selected.into_iter().collect())
}

fn parse_print_topic_number(value: &str, topic_count: usize) -> Result<usize, String> {
    let number = value
        .parse::<usize>()
        .map_err(|_| format!("'{value}' is not a valid topic number."))?;
    if number == 0 || number > topic_count {
        return Err(format!("Topic {number} is outside the available range 1-{topic_count}."));
    }
    Ok(number)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrintOutcome {
    Printed,
    Cancelled,
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct NativePrintDialogW {
    l_struct_size: u32,
    hwnd_owner: HWND,
    h_dev_mode: HANDLE,
    h_dev_names: HANDLE,
    h_dc: HDC,
    flags: u32,
    from_page: u16,
    to_page: u16,
    min_page: u16,
    max_page: u16,
    copies: u16,
    h_instance: *mut std::ffi::c_void,
    l_cust_data: isize,
    lpfn_print_hook: *mut std::ffi::c_void,
    lpfn_setup_hook: *mut std::ffi::c_void,
    lp_print_template_name: *const u16,
    lp_setup_template_name: *const u16,
    h_print_template: HANDLE,
    h_setup_template: HANDLE,
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct NativeDocInfoW {
    cb_size: i32,
    doc_name: *const u16,
    output: *const u16,
    data_type: *const u16,
    fw_type: u32,
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct NativeBitmapInfoHeader {
    bi_size: u32,
    bi_width: i32,
    bi_height: i32,
    bi_planes: u16,
    bi_bit_count: u16,
    bi_compression: u32,
    bi_size_image: u32,
    bi_x_pels_per_meter: i32,
    bi_y_pels_per_meter: i32,
    bi_clr_used: u32,
    bi_clr_important: u32,
}

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Clone, Copy)]
struct NativeRgbQuad {
    blue: u8,
    green: u8,
    red: u8,
    reserved: u8,
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct NativeBitmapInfo {
    header: NativeBitmapInfoHeader,
    colors: [NativeRgbQuad; 1],
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
#[link(name = "comdlg32")]
unsafe extern "system" {
    fn PrintDlgW(dialog: *mut NativePrintDialogW) -> i32;
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
#[link(name = "gdi32")]
unsafe extern "system" {
    fn StartDocW(hdc: HDC, info: *const NativeDocInfoW) -> i32;
    fn EndDoc(hdc: HDC) -> i32;
    fn AbortDoc(hdc: HDC) -> i32;
    fn StartPage(hdc: HDC) -> i32;
    fn EndPage(hdc: HDC) -> i32;
    fn DeleteDC(hdc: HDC) -> i32;
    fn CreatePen(style: i32, width: i32, color: u32) -> *mut std::ffi::c_void;
    fn CreateSolidBrush(color: u32) -> *mut std::ffi::c_void;
    fn MoveToEx(hdc: HDC, x: i32, y: i32, previous: *mut std::ffi::c_void) -> i32;
    fn LineTo(hdc: HDC, x: i32, y: i32) -> i32;
    fn SaveDC(hdc: HDC) -> i32;
    fn RestoreDC(hdc: HDC, saved_dc: i32) -> i32;
    fn IntersectClipRect(hdc: HDC, left: i32, top: i32, right: i32, bottom: i32) -> i32;
    fn StretchDIBits(
        hdc: HDC,
        x_dest: i32,
        y_dest: i32,
        dest_width: i32,
        dest_height: i32,
        x_src: i32,
        y_src: i32,
        src_width: i32,
        src_height: i32,
        bits: *const std::ffi::c_void,
        bitmap_info: *const NativeBitmapInfo,
        usage: u32,
        raster_op: u32,
    ) -> i32;
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterClassW(class: *const NativeWindowClassW) -> u16;
    fn DefWindowProcW(hwnd: HWND, message: u32, wparam: usize, lparam: isize) -> isize;
    fn BeginPaint(hwnd: HWND, paint: *mut NativePaintStruct) -> HDC;
    fn EndPaint(hwnd: HWND, paint: *const NativePaintStruct) -> i32;
    fn FillRect(hdc: HDC, rect: *const RECT, brush: *mut std::ffi::c_void) -> i32;
    fn UpdateWindow(hwnd: HWND) -> i32;
    fn GetCursorPos(point: *mut POINT) -> i32;
    fn ScreenToClient(hwnd: HWND, point: *mut POINT) -> i32;
    fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> i32;
    fn SetPropW(hwnd: HWND, name: *const u16, data: *mut std::ffi::c_void) -> i32;
    fn GetPropW(hwnd: HWND, name: *const u16) -> *mut std::ffi::c_void;
    fn RemovePropW(hwnd: HWND, name: *const u16) -> *mut std::ffi::c_void;
    fn SetTimer(
        hwnd: HWND,
        timer_id: usize,
        elapse_ms: u32,
        timer_proc: Option<unsafe extern "system" fn(HWND, u32, usize, u32)>,
    ) -> usize;
    fn KillTimer(hwnd: HWND, timer_id: usize) -> i32;
    fn SetWindowPos(
        hwnd: HWND,
        hwnd_insert_after: HWND,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> i32;
}

#[cfg(target_os = "windows")]
struct PrinterTextContext {
    hdc: HDC,
    dpi_x: i32,
    dpi_y: i32,
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
impl PrinterTextContext {
    fn create_font(
        &self,
        style: &ResolvedTextStyle,
        text_zoom_percent: i32,
    ) -> Option<*mut std::ffi::c_void> {
        create_gdi_font_for_style(style, self.dpi_y, text_zoom_percent)
    }

    fn measure(&self, style: &ResolvedTextStyle, text: &str) -> TextMetrics {
        let fallback_height = font_pixel_height_from_twips(effective_authored_font_twips(style), self.dpi_y);
        let fallback = TextMetrics {
            width: i32::try_from(text.chars().count())
                .unwrap_or(i32::MAX)
                .saturating_mul(fallback_height)
                .saturating_div(2),
            height: fallback_height.max(1),
            baseline: fallback_height.saturating_mul(4).saturating_div(5).max(1),
        };
        let Some(font) = self.create_font(style, 100) else {
            return fallback;
        };
        let old_font = unsafe { SelectObject(self.hdc, font) };
        if old_font.is_null() {
            unsafe { DeleteObject(font); }
            return fallback;
        }

        let wide = text.encode_utf16().collect::<Vec<_>>();
        let count = i32::try_from(wide.len()).ok();
        let mut size = SIZE::default();
        let mut metrics = TEXTMETRICW::default();
        let extent_ok = match count {
            Some(0) => true,
            Some(count) => unsafe { GetTextExtentPoint32W(self.hdc, wide.as_ptr(), count, &mut size) != 0 },
            None => false,
        };
        let metrics_ok = unsafe { GetTextMetricsW(self.hdc, &mut metrics) != 0 };
        unsafe {
            SelectObject(self.hdc, old_font);
            DeleteObject(font);
        }
        if !extent_ok || !metrics_ok {
            return fallback;
        }
        let height = metrics.tmHeight.saturating_add(metrics.tmExternalLeading.max(0)).max(1);
        TextMetrics {
            width: size.cx.max(0),
            height,
            baseline: metrics.tmAscent.clamp(1, height),
        }
    }

    fn paint_text(
        &self,
        style: &ResolvedTextStyle,
        text: &str,
        foreground: Rgb,
        x: i32,
        y: i32,
    ) -> Result<(), String> {
        if text.is_empty() {
            return Ok(());
        }
        let wide = text.encode_utf16().collect::<Vec<_>>();
        let count = i32::try_from(wide.len()).map_err(|_| "A printable text run is too long.".to_owned())?;
        let font = self
            .create_font(style, 100)
            .ok_or_else(|| "Windows could not create a font required by this help topic.".to_owned())?;
        let old_font = unsafe { SelectObject(self.hdc, font) };
        if old_font.is_null() {
            unsafe { DeleteObject(font); }
            return Err("Windows could not select a font into the printer device context.".to_owned());
        }
        let old_background_mode = unsafe { SetBkMode(self.hdc, TRANSPARENT as i32) };
        let old_text_color = unsafe { SetTextColor(self.hdc, colorref_from_rgb(foreground)) };
        let painted = unsafe { TextOutW(self.hdc, x, y, wide.as_ptr(), count) != 0 };
        if old_background_mode != 0 {
            unsafe { SetBkMode(self.hdc, old_background_mode); }
        }
        if old_text_color != u32::MAX {
            unsafe { SetTextColor(self.hdc, old_text_color); }
        }
        unsafe {
            SelectObject(self.hdc, old_font);
            DeleteObject(font);
        }
        if painted {
            Ok(())
        } else {
            Err("Windows failed while drawing formatted text to the printer.".to_owned())
        }
    }
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn native_print_topics(
    owner: *mut std::ffi::c_void,
    topics: &[PrintableTopic],
    fonts: &hlp::FontTable,
) -> Result<PrintOutcome, String> {
    const PD_RETURNDC: u32 = 0x0000_0100;
    const PD_NOSELECTION: u32 = 0x0000_0004;
    const PD_NOPAGENUMS: u32 = 0x0000_0008;
    const PD_USEDEVMODECOPIESANDCOLLATE: u32 = 0x0004_0000;
    const HORZRES: i32 = 8;
    const VERTRES: i32 = 10;

    if topics.is_empty() {
        return Err("There are no topics selected for printing.".to_owned());
    }

    let mut dialog: NativePrintDialogW = unsafe { std::mem::zeroed() };
    dialog.l_struct_size = u32::try_from(std::mem::size_of::<NativePrintDialogW>())
        .map_err(|_| "Print dialog structure is too large".to_owned())?;
    dialog.hwnd_owner = owner as HWND;
    dialog.copies = 1;
    dialog.flags =
        PD_RETURNDC | PD_NOSELECTION | PD_NOPAGENUMS | PD_USEDEVMODECOPIESANDCOLLATE;
    if unsafe { PrintDlgW(&mut dialog) } == 0 {
        return Ok(PrintOutcome::Cancelled);
    }
    if dialog.h_dc.is_null() {
        return Err("The printer dialog did not return a printer device context.".to_owned());
    }

    struct DcGuard(HDC);
    impl Drop for DcGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { DeleteDC(self.0); }
            }
        }
    }
    let dc_guard = DcGuard(dialog.h_dc);
    let hdc = dc_guard.0;

    let page_width = unsafe { GetDeviceCaps(hdc, HORZRES) }.max(1);
    let page_height = unsafe { GetDeviceCaps(hdc, VERTRES) }.max(1);
    let dpi_x = unsafe { GetDeviceCaps(hdc, LOGPIXELSX as i32) }.max(72);
    let dpi_y = unsafe { GetDeviceCaps(hdc, LOGPIXELSY as i32) }.max(72);
    let margin_x = (dpi_x / 2).max(24);
    let margin_y = (dpi_y / 2).max(24);
    let printable_width = page_width.saturating_sub(margin_x.saturating_mul(2)).max(80);
    let printable_height = page_height.saturating_sub(margin_y.saturating_mul(2)).max(80);
    let text_context = PrinterTextContext { hdc, dpi_x, dpi_y };

    let document_label = if topics.len() == 1 {
        topic_label(&topics[0].presentation.title, topics[0].source_index)
    } else {
        format!("{} WinHelp topics", topics.len())
    };
    let doc_name = format!("Rust HLP Viewer - {document_label}\0").encode_utf16().collect::<Vec<_>>();
    let info = NativeDocInfoW {
        cb_size: i32::try_from(std::mem::size_of::<NativeDocInfoW>()).unwrap_or(0),
        doc_name: doc_name.as_ptr(),
        output: std::ptr::null(),
        data_type: std::ptr::null(),
        fw_type: 0,
    };
    if unsafe { StartDocW(hdc, &info) } <= 0 {
        return Err("Windows could not start the print job.".to_owned());
    }

    let result = (|| -> Result<(), String> {
        for topic in topics {
            print_formatted_topic(
                &text_context,
                fonts,
                topic,
                margin_x,
                margin_y,
                printable_width,
                printable_height,
            )?;
        }
        Ok(())
    })();

    match result {
        Ok(()) => {
            if unsafe { EndDoc(hdc) } <= 0 {
                Err("Windows could not finish the print job.".to_owned())
            } else {
                Ok(PrintOutcome::Printed)
            }
        }
        Err(error) => {
            unsafe { AbortDoc(hdc); }
            Err(error)
        }
    }
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn print_formatted_topic(
    text_context: &PrinterTextContext,
    fonts: &hlp::FontTable,
    topic: &PrintableTopic,
    margin_x: i32,
    margin_y: i32,
    printable_width: i32,
    printable_height: i32,
) -> Result<(), String> {
    let mut measure = |style: &ResolvedTextStyle, text: &str| text_context.measure(style, text);
    let layout = LayoutEngine::with_dpi_and_text_zoom(text_context.dpi_x, text_context.dpi_y, 100)
        .layout_topic_with_measurer(&topic.presentation, fonts, printable_width, &mut measure);

    let title = topic_label(&topic.presentation.title, topic.source_index);
    let title_height = font_pixel_height_from_twips(240, text_context.dpi_y).max(1);
    let title_gap = (text_context.dpi_y / 12).max(4);
    let body_origin = title_height.saturating_add(title_gap);
    let fixed_span = if layout.fixed.boxes.is_empty() { 0 } else { layout.fixed.height };
    let scrolling_span = if layout.scrolling.boxes.is_empty() { 0 } else { layout.scrolling.height };
    let fixed_origin = body_origin;
    let scrolling_origin = body_origin.saturating_add(fixed_span);
    let body_is_empty = fixed_span == 0 && scrolling_span == 0;
    let empty_height = if body_is_empty {
        font_pixel_height_from_twips(200, text_context.dpi_y).saturating_mul(2)
    } else {
        0
    };
    let total_height = body_origin
        .saturating_add(fixed_span)
        .saturating_add(scrolling_span)
        .saturating_add(empty_height)
        .max(1);
    let page_count = usize::try_from(
        (i64::from(total_height) + i64::from(printable_height) - 1) / i64::from(printable_height),
    )
    .unwrap_or(1)
    .max(1);

    for page_index in 0..page_count {
        if unsafe { StartPage(text_context.hdc) } <= 0 {
            return Err("Windows could not start a printer page.".to_owned());
        }
        let saved_dc = unsafe { SaveDC(text_context.hdc) };
        unsafe {
            IntersectClipRect(
                text_context.hdc,
                margin_x,
                margin_y,
                margin_x.saturating_add(printable_width),
                margin_y.saturating_add(printable_height),
            );
        }

        let page_start = i32::try_from(page_index)
            .unwrap_or(i32::MAX)
            .saturating_mul(printable_height);
        let page_end = page_start.saturating_add(printable_height);
        if page_index == 0 {
            printer_draw_simple_text(
                text_context.hdc,
                text_context.dpi_y,
                &title,
                margin_x,
                margin_y,
                12,
                700,
                Rgb { red: 0, green: 0, blue: 0 },
            )?;
        }

        if body_is_empty {
            if body_origin < page_end && body_origin.saturating_add(empty_height) > page_start {
                printer_draw_simple_text(
                    text_context.hdc,
                    text_context.dpi_y,
                    "(This topic contains no printable content.)",
                    margin_x,
                    margin_y.saturating_add(body_origin).saturating_sub(page_start),
                    10,
                    400,
                    Rgb { red: 0, green: 0, blue: 0 },
                )?;
            }
        } else {
            printer_paint_region(
                text_context,
                &layout.fixed,
                fixed_origin,
                page_start,
                page_end,
                margin_x,
                margin_y,
            )?;
            printer_paint_region(
                text_context,
                &layout.scrolling,
                scrolling_origin,
                page_start,
                page_end,
                margin_x,
                margin_y,
            )?;
        }

        if saved_dc != 0 {
            unsafe { RestoreDC(text_context.hdc, saved_dc); }
        }
        if unsafe { EndPage(text_context.hdc) } <= 0 {
            return Err("Windows could not finish a printer page.".to_owned());
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn printer_paint_region(
    text_context: &PrinterTextContext,
    region: &RegionLayout,
    region_origin: i32,
    page_start: i32,
    page_end: i32,
    margin_x: i32,
    margin_y: i32,
) -> Result<(), String> {
    for item in &region.boxes {
        let document_top = region_origin.saturating_add(item.bounds.y);
        let document_bottom = document_top.saturating_add(item.bounds.height.max(1));
        if document_bottom <= page_start || document_top >= page_end {
            continue;
        }
        let x = margin_x.saturating_add(item.bounds.x);
        let y = margin_y.saturating_add(document_top).saturating_sub(page_start);
        printer_paint_layout_box(text_context, item, x, y)?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn printer_paint_layout_box(
    text_context: &PrinterTextContext,
    item: &LayoutBox,
    x: i32,
    y: i32,
) -> Result<(), String> {
    match &item.kind {
        LayoutKind::Text { text, style, .. } => {
            let foreground = if style.foreground_inherits {
                Rgb { red: 0, green: 0, blue: 0 }
            } else {
                style.foreground
            };
            let background = if style.background_inherits {
                PRINT_PAGE_BACKGROUND
            } else {
                style.background
            };
            if background != PRINT_PAGE_BACKGROUND {
                printer_fill_rect(
                    text_context.hdc,
                    x,
                    y,
                    item.bounds.width,
                    item.bounds.height,
                    background,
                )?;
            }
            text_context.paint_text(style, text, foreground, x, y)
        }
        LayoutKind::Picture { image } => printer_draw_picture(
            text_context.hdc,
            x,
            y,
            item.bounds.width,
            item.bounds.height,
            image,
        ),
        LayoutKind::PictureHotspot { .. } => Ok(()),
        LayoutKind::PicturePlaceholder => {
            printer_draw_placeholder(text_context, x, y, item.bounds.width, item.bounds.height, "[embedded picture]")
        }
        LayoutKind::EmbeddedWindowPlaceholder { standard_button_label, .. } => {
            let label = standard_button_label
                .as_deref()
                .unwrap_or("[embedded WinHelp control]");
            printer_draw_placeholder(text_context, x, y, item.bounds.width, item.bounds.height, label)
        }
        LayoutKind::Border { flags, style } => {
            printer_draw_border(text_context, item, x, y, *flags, *style)
        }
    }
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn printer_fill_rect(
    hdc: HDC,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    color: Rgb,
) -> Result<(), String> {
    if width <= 0 || height <= 0 {
        return Ok(());
    }
    let brush = unsafe { CreateSolidBrush(colorref_from_rgb(color)) };
    if brush.is_null() {
        return Err("Windows could not create a printer fill brush.".to_owned());
    }
    let rect = RECT {
        left: x,
        top: y,
        right: x.saturating_add(width),
        bottom: y.saturating_add(height),
    };
    let painted = unsafe { FillRect(hdc, &rect, brush) } != 0;
    unsafe { DeleteObject(brush); }
    if painted {
        Ok(())
    } else {
        Err("Windows failed while printing an authored background colour.".to_owned())
    }
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn printer_draw_picture(
    hdc: HDC,
    x: i32,
    y: i32,
    target_width: i32,
    target_height: i32,
    image: &hlp::DecodedPicture,
) -> Result<(), String> {
    if target_width <= 0 || target_height <= 0 || image.width == 0 || image.height == 0 {
        return Ok(());
    }
    let source_width = i32::try_from(image.width).map_err(|_| "A help picture is too wide to print.".to_owned())?;
    let source_height = i32::try_from(image.height).map_err(|_| "A help picture is too tall to print.".to_owned())?;
    let mut bgra = Vec::with_capacity(image.rgba.len());
    for pixel in image.rgba.chunks_exact(4) {
        let alpha = u16::from(pixel[3]);
        let inverse = 255_u16.saturating_sub(alpha);
        let blend = |channel: u8| -> u8 {
            u8::try_from((u16::from(channel) * alpha + 255 * inverse + 127) / 255).unwrap_or(255)
        };
        bgra.extend_from_slice(&[blend(pixel[2]), blend(pixel[1]), blend(pixel[0]), 0]);
    }
    let bitmap_info = NativeBitmapInfo {
        header: NativeBitmapInfoHeader {
            bi_size: u32::try_from(std::mem::size_of::<NativeBitmapInfoHeader>()).unwrap_or(40),
            bi_width: source_width,
            bi_height: -source_height,
            bi_planes: 1,
            bi_bit_count: 32,
            bi_compression: 0,
            bi_size_image: 0,
            bi_x_pels_per_meter: 0,
            bi_y_pels_per_meter: 0,
            bi_clr_used: 0,
            bi_clr_important: 0,
        },
        colors: [NativeRgbQuad { blue: 0, green: 0, red: 0, reserved: 0 }],
    };
    let copied = unsafe {
        StretchDIBits(
            hdc,
            x,
            y,
            target_width,
            target_height,
            0,
            0,
            source_width,
            source_height,
            bgra.as_ptr().cast(),
            &bitmap_info,
            0,
            0x00CC_0020,
        )
    };
    if copied == 0 || copied == -1 {
        Err("Windows failed while drawing a help picture to the printer.".to_owned())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn printer_draw_placeholder(
    text_context: &PrinterTextContext,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    label: &str,
) -> Result<(), String> {
    printer_draw_rectangle_outline(text_context.hdc, x, y, width, height, 1)?;
    printer_draw_simple_text(
        text_context.hdc,
        text_context.dpi_y,
        label,
        x.saturating_add((text_context.dpi_x / 24).max(2)),
        y.saturating_add((text_context.dpi_y / 24).max(2)),
        8,
        400,
        Rgb { red: 96, green: 96, blue: 96 },
    )
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn printer_draw_rectangle_outline(
    hdc: HDC,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    pen_width: i32,
) -> Result<(), String> {
    let pen = unsafe { CreatePen(0, pen_width.max(1), 0) };
    if pen.is_null() {
        return Err("Windows could not create a printer border pen.".to_owned());
    }
    let old_pen = unsafe { SelectObject(hdc, pen) };
    let right = x.saturating_add(width);
    let bottom = y.saturating_add(height);
    unsafe {
        MoveToEx(hdc, x, y, std::ptr::null_mut());
        LineTo(hdc, right, y);
        LineTo(hdc, right, bottom);
        LineTo(hdc, x, bottom);
        LineTo(hdc, x, y);
        if !old_pen.is_null() { SelectObject(hdc, old_pen); }
        DeleteObject(pen);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn printer_draw_border(
    text_context: &PrinterTextContext,
    item: &LayoutBox,
    x: i32,
    y: i32,
    flags: hlp::BorderFlags,
    style: hlp::BorderStyle,
) -> Result<(), String> {
    if matches!(style, hlp::BorderStyle::Reserved(_)) {
        return Ok(());
    }
    let unit = (text_context.dpi_x / 96).max(1);
    let width = if matches!(style, hlp::BorderStyle::Thick) { unit.saturating_mul(2) } else { unit };
    let pen = unsafe { CreatePen(0, width, 0) };
    if pen.is_null() {
        return Err("Windows could not create a printer border pen.".to_owned());
    }
    let old_pen = unsafe { SelectObject(text_context.hdc, pen) };

    let compact_horizontal_separator = !flags.box_all
        && flags.top
        && flags.bottom
        && !flags.left
        && !flags.right
        && item.bounds.height <= 16;
    if compact_horizontal_separator {
        unsafe {
            MoveToEx(text_context.hdc, x, y, std::ptr::null_mut());
            LineTo(text_context.hdc, x.saturating_add(item.bounds.width), y);
        }
    } else {
        printer_draw_border_edges(text_context.hdc, x, y, item.bounds.width, item.bounds.height, flags, 0);
        match style {
            hlp::BorderStyle::Double if item.bounds.width > 4 && item.bounds.height > 4 => {
                printer_draw_border_edges(
                    text_context.hdc,
                    x,
                    y,
                    item.bounds.width,
                    item.bounds.height,
                    flags,
                    unit.saturating_mul(2),
                );
            }
            hlp::BorderStyle::Shadow => {
                printer_draw_border_shadow(text_context.hdc, x, y, item.bounds.width, item.bounds.height, flags, unit);
            }
            _ => {}
        }
    }

    unsafe {
        if !old_pen.is_null() { SelectObject(text_context.hdc, old_pen); }
        DeleteObject(pen);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn printer_draw_border_edges(
    hdc: HDC,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    flags: hlp::BorderFlags,
    inset: i32,
) {
    let left = x.saturating_add(inset);
    let top = y.saturating_add(inset);
    let right = x.saturating_add(width).saturating_sub(inset);
    let bottom = y.saturating_add(height).saturating_sub(inset);
    let all = flags.box_all;
    unsafe {
        if all || flags.top {
            MoveToEx(hdc, left, top, std::ptr::null_mut());
            LineTo(hdc, right, top);
        }
        if all || flags.left {
            MoveToEx(hdc, left, top, std::ptr::null_mut());
            LineTo(hdc, left, bottom);
        }
        if all || flags.bottom {
            MoveToEx(hdc, left, bottom, std::ptr::null_mut());
            LineTo(hdc, right, bottom);
        }
        if all || flags.right {
            MoveToEx(hdc, right, top, std::ptr::null_mut());
            LineTo(hdc, right, bottom);
        }
    }
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn printer_draw_border_shadow(
    hdc: HDC,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    flags: hlp::BorderFlags,
    offset: i32,
) {
    let right = x.saturating_add(width);
    let bottom = y.saturating_add(height);
    let all = flags.box_all;
    unsafe {
        if all || flags.bottom {
            MoveToEx(hdc, x.saturating_add(offset), bottom.saturating_add(offset), std::ptr::null_mut());
            LineTo(hdc, right.saturating_add(offset), bottom.saturating_add(offset));
        }
        if all || flags.right {
            MoveToEx(hdc, right.saturating_add(offset), y.saturating_add(offset), std::ptr::null_mut());
            LineTo(hdc, right.saturating_add(offset), bottom.saturating_add(offset));
        }
    }
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn printer_draw_simple_text(
    hdc: HDC,
    dpi_y: i32,
    text: &str,
    x: i32,
    y: i32,
    point_size: i32,
    weight: i32,
    color: Rgb,
) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    let mut font_definition = LOGFONTW::default();
    font_definition.lfHeight = -font_pixel_height_from_twips(point_size.saturating_mul(20), dpi_y);
    font_definition.lfWeight = weight.clamp(0, 1000);
    font_definition.lfCharSet = DEFAULT_CHARSET as u8;
    for (target, source) in font_definition
        .lfFaceName
        .iter_mut()
        .take(31)
        .zip("Segoe UI".encode_utf16())
    {
        *target = source;
    }
    let font = unsafe { CreateFontIndirectW(&font_definition) };
    if font.is_null() {
        return Err("Windows could not create a printer text font.".to_owned());
    }
    let old_font = unsafe { SelectObject(hdc, font) };
    if old_font.is_null() {
        unsafe { DeleteObject(font); }
        return Err("Windows could not select a printer text font.".to_owned());
    }
    let wide = text.encode_utf16().collect::<Vec<_>>();
    let count = i32::try_from(wide.len()).map_err(|_| "Printable text is too long.".to_owned())?;
    let old_background_mode = unsafe { SetBkMode(hdc, TRANSPARENT as i32) };
    let old_text_color = unsafe { SetTextColor(hdc, colorref_from_rgb(color)) };
    let painted = unsafe { TextOutW(hdc, x, y, wide.as_ptr(), count) != 0 };
    if old_background_mode != 0 {
        unsafe { SetBkMode(hdc, old_background_mode); }
    }
    if old_text_color != u32::MAX {
        unsafe { SetTextColor(hdc, old_text_color); }
    }
    unsafe {
        SelectObject(hdc, old_font);
        DeleteObject(font);
    }
    if painted {
        Ok(())
    } else {
        Err("Windows failed while drawing printer text.".to_owned())
    }
}

#[cfg(not(target_os = "windows"))]
fn native_print_topics(
    _owner: *mut std::ffi::c_void,
    _topics: &[PrintableTopic],
    _fonts: &hlp::FontTable,
) -> Result<PrintOutcome, String> {
    Err("Printing is currently implemented for Windows builds of Rust HLP Viewer.".to_owned())
}


/// Performs the scriptable HTML-export path without constructing wxWidgets. The source HLP becomes
/// both the navigation root and initial active document, while the same `.CNT`/`.GID` linked catalog
/// discovery used by the GUI supplies integrated Index/Search documents. The HTML exporter itself
/// recursively embeds safe relative cross-document destinations and preserves the root hierarchy.
fn export_html_headless(
    source_path: &Path,
    output_path: &Path,
) -> Result<html_export::HtmlExportReport, String> {
    let navigation_document = HelpDocument::open(source_path)
        .map_err(|error| format!("could not open '{}': {error}", source_path.display()))?;
    let active_topic_index = navigation_document.startup_topic_index().unwrap_or(0);
    let (catalog_documents, _catalog_warnings) = load_related_documents(&navigation_document);

    html_export::export_to_html(
        html_export::HtmlExportRequest {
            navigation_document: &navigation_document,
            active_document: &navigation_document,
            active_topic_index,
            catalog_documents: &catalog_documents,
            // Headless exports need a stable viewport because no GUI canvas exists to supply one.
            // These are the same practical defaults used by the interactive exporter before a
            // document has accumulated user-specific resize/zoom state.
            layout_width: html_export::DEFAULT_EXPORT_LAYOUT_WIDTH,
            text_zoom_percent: DEFAULT_TEXT_ZOOM_PERCENT,
        },
        output_path,
    )
}

/// Exports the user-opened help system to one self-contained interactive HTML file. Cross-document
/// topics may be embedded, but Contents/Index/Search stay rooted in `navigation_document`, matching
/// the native viewer's build-fix 68 navigation policy.
fn export_html_dialog(ui: &Rc<ViewerUi>, state: &Rc<RefCell<ViewerState>>) {
    let (navigation_document, active_document, active_topic_index, catalog_documents, layout_width, text_zoom_percent) = {
        let state = state.borrow();
        let Some(active_document) = state.document.as_ref() else {
            MessageDialog::builder(
                &ui.frame,
                "Open a help file before exporting to HTML.",
                "Export to HTML",
            )
            .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
            .build()
            .show_modal();
            return;
        };
        let navigation_document = state
            .navigation_document
            .as_ref()
            .unwrap_or(active_document);
        (
            navigation_document.clone(),
            active_document.clone(),
            state.topic_index,
            state.related_documents.clone(),
            state.layout_width.max(MIN_LAYOUT_WIDTH),
            state.text_zoom_percent,
        )
    };

    let default_name = navigation_document
        .source_path()
        .file_stem()
        .map(|stem| format!("{}.html", stem.to_string_lossy()))
        .unwrap_or_else(|| "windows-help.html".to_owned());

    let dialog = FileDialog::builder(&ui.frame)
        .with_message("Export Windows Help to HTML")
        .with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
        .with_wildcard("HTML files (*.html)|*.html|All files (*.*)|*.*")
        .build();
    dialog.set_filename(&default_name);
    if let Some(directory) = navigation_document
        .source_path()
        .parent()
        .and_then(Path::to_str)
    {
        dialog.set_directory(directory);
    }

    if dialog.show_modal() != wxdragon::id::ID_OK {
        ui.status_bar.set_status_text("HTML export cancelled", 0);
        return;
    }
    let Some(path) = dialog.get_path() else {
        return;
    };

    ui.status_bar
        .set_status_text("Exporting interactive HTML...", 0);
    match html_export::export_to_html(
        html_export::HtmlExportRequest {
            navigation_document: &navigation_document,
            active_document: &active_document,
            active_topic_index,
            catalog_documents: &catalog_documents,
            layout_width,
            text_zoom_percent,
        },
        Path::new(&path),
    ) {
        Ok(report) => {
            let warning_note = if report.warning_count == 0 {
                String::new()
            } else {
                format!(
                    "\n\n{} linked item(s) could not be embedded or resolved and were retained as safe unavailable actions.",
                    report.warning_count
                )
            };
            let message = format!(
                "Exported {} topic(s) from {} help document(s) to:\n{}{}",
                report.topic_count,
                report.document_count,
                report.output_path.display(),
                warning_note
            );
            ui.status_bar.set_status_text(
                &format!("HTML export complete: {}", report.output_path.display()),
                0,
            );
            MessageDialog::builder(&ui.frame, &message, "Export to HTML")
                .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
                .build()
                .show_modal();
        }
        Err(error) => {
            ui.status_bar.set_status_text("HTML export failed", 0);
            MessageDialog::builder(&ui.frame, &error, "HTML export failed")
                .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
                .build()
                .show_modal();
        }
    }
}

fn open_document_dialog(ui: &Rc<ViewerUi>, state: &Rc<RefCell<ViewerState>>) {
    let dialog = FileDialog::builder(&ui.frame)
        .with_message("Open Windows Help file")
        .with_style(FileDialogStyle::Open | FileDialogStyle::FileMustExist)
        .with_wildcard("Windows Help files (*.hlp)|*.hlp|All files (*.*)|*.*")
        .build();

    if dialog.show_modal() != wxdragon::id::ID_OK {
        return;
    }
    let Some(path) = dialog.get_path() else {
        return;
    };
    load_document(ui, state, Path::new(&path), true);
}

/// Loads a file into the main window. Manual/new startup opens intentionally reset browser history.
fn load_document(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    path: &Path,
    clear_history: bool,
) {
    ui.status_bar.set_status_text("Opening and decoding HLP...", 0);
    match HelpDocument::open(path) {
        Ok(document) => {
            let topic_index = document.startup_topic_index().unwrap_or(0);
            install_document(ui, state, document, topic_index, clear_history);
            remember_recent_document(ui, state, path);
        }
        Err(error) => show_open_error(ui, path, &error.to_string()),
    }
}

fn install_document(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    document: HelpDocument,
    topic_index: usize,
    clear_history: bool,
) {
    dismiss_main_transients(ui, state);
    let installed_identity = path_identity(document.source_path());
    let installed_topic = topic_index;

    // A manual/startup open establishes a new navigation root. Cross-document jumps only replace
    // the active topic document; they must not replace Contents/Index/Search with the referenced
    // file's structure. The `is_none()` guard also keeps this safe for any future non-manual first
    // install path.
    let replace_navigation_root = {
        let state = state.borrow();
        clear_history || state.navigation_document.is_none()
    };
    let (related_documents, navigation_warnings) = if replace_navigation_root {
        load_related_documents(&document)
    } else {
        (Vec::new(), Vec::new())
    };
    let navigation_document = replace_navigation_root.then(|| document.clone());

    let display_name = document
        .source_path()
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("help file")
        .to_owned();
    {
        let mut state = state.borrow_mut();
        state.document = Some(document);
        if let Some(navigation_document) = navigation_document {
            state.navigation_document = Some(navigation_document);
            state.related_documents = related_documents;
            state.navigation_warnings = navigation_warnings;
        }
        state.topic_index = topic_index;
        if clear_history {
            state.contents_view_mode = ContentsViewMode::Hierarchical;
        }
        state.layout = None;
        state.layout_width = 0;
        if clear_history {
            state.history.clear();
        }
    }
    ui.frame.set_title(&format!("{display_name} - Rust HLP Viewer"));
    refresh_topic_layout(ui, state);
    if replace_navigation_root {
        refresh_navigation_pane(ui, state);
        if let Some(warning) = navigation_warning_summary(&state.borrow().navigation_warnings) {
            ui.status_bar.set_status_text(&warning, 0);
        }
    } else {
        // Cross-file installs preserve the original navigation widgets verbatim. Only session history
        // and the browse strip need to notice that the active topic document changed.
        refresh_history_list(ui, state);
    }
    ui.scrolling_canvas.set_focus();

    // Native WinHelp runs file CONFIG macros when a help file is opened, followed by the active
    // topic's own macros. If CONFIG navigation changes the location, that navigation path executes
    // the destination topic macros itself, avoiding a duplicate run here.
    execute_document_config_macros_main(ui, state);
    let still_initial = {
        let state = state.borrow();
        state.topic_index == installed_topic
            && state
                .document
                .as_ref()
                .is_some_and(|document| path_identity(document.source_path()) == installed_identity)
    };
    if still_initial {
        execute_current_topic_macros_main(ui, state);
    }
}

/// Loads one-hop help files named by the navigation root's Contents metadata for integrated Index/Search.
/// The metadata can come directly from `.CNT` or from GID `|FILES`. Failures remain non-fatal and
/// are surfaced in the status bar; they never prevent the base HLP from opening.
fn load_related_documents(document: &HelpDocument) -> (Vec<HelpDocument>, Vec<String>) {
    let mut warnings = Vec::new();
    if let Some(warning) = document.contents_warning() {
        warnings.push(warning.to_owned());
    }
    warnings.extend(document.keywords().warnings.iter().cloned());
    let Some(contents) = document.contents_file() else {
        return (Vec::new(), warnings);
    };
    warnings.extend(contents.warnings.iter().cloned());

    let mut requested = Vec::new();
    requested.extend(contents.index_links.iter().map(|link| link.help_file.as_str()));
    requested.extend(contents.search_links.iter().map(|link| link.help_file.as_str()));

    let current_key = path_identity(document.source_path());
    let mut seen = BTreeSet::new();
    seen.insert(current_key);
    let mut related = Vec::new();
    let mut attempted = 0_usize;

    for target in requested {
        if !automatic_catalog_reference_allowed(target) {
            warnings.push(format!(
                "skipped automatic linked-help catalog outside the opened HLP's relative path space: {target}"
            ));
            continue;
        }
        let path = resolve_external_help_path(document.source_path(), target);
        if !seen.insert(path_identity(&path)) {
            continue;
        }
        if attempted >= MAX_RELATED_HELP_FILES {
            warnings.push(format!(
                "linked-help catalog limit ({MAX_RELATED_HELP_FILES}) reached; remaining :Index/:Link files were not opened"
            ));
            break;
        }
        attempted += 1;
        match HelpDocument::open(&path) {
            Ok(linked) => {
                warnings.extend(linked.keywords().warnings.iter().map(|warning| {
                    format!("{}: {warning}", linked.source_path().display())
                }));
                related.push(linked);
            }
            Err(error) => warnings.push(format!(
                "could not open linked help file '{}': {error}",
                path.display()
            )),
        }
    }
    (related, warnings)
}

/// Prevents untrusted `.CNT`/`.GID` metadata from causing automatic absolute/UNC/network file access.
/// Explicit user-activated hyperlinks keep their existing cross-file behavior.
fn automatic_catalog_reference_allowed(target: &str) -> bool {
    let normalized = target.trim().replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') || normalized.starts_with("//") {
        return false;
    }
    let bytes = normalized.as_bytes();
    !bytes.get(1).is_some_and(|byte| *byte == b':')
}

fn path_identity(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

fn navigation_warning_summary(warnings: &[String]) -> Option<String> {
    let first = warnings.first()?;
    if warnings.len() == 1 {
        Some(format!("Navigation metadata warning: {first}"))
    } else {
        Some(format!(
            "Navigation metadata: {} warnings (first: {first})",
            warnings.len()
        ))
    }
}

fn show_open_error(ui: &ViewerUi, path: &Path, error: &str) {
    let legacy_family = error.starts_with("unsupported HLP container:");
    ui.status_bar.set_status_text(
        if legacy_family {
            "Unsupported HLP family"
        } else {
            "Failed to open HLP"
        },
        0,
    );
    let message = format!("Could not open:\n{}\n\n{error}", path.display());
    let title = if legacy_family {
        "Unsupported HLP family"
    } else {
        "Invalid or unsupported HLP file"
    };
    MessageDialog::builder(&ui.frame, &message, title)
        .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
        .build()
        .show_modal();
}

fn current_location(state: &ViewerState) -> Option<NavigationLocation> {
    let document = state.document.as_ref()?;
    Some(NavigationLocation {
        source_path: document.source_path().to_path_buf(),
        topic_index: state.topic_index,
        topic_offset: document.topic_start_offset(state.topic_index),
        window_name: None,
    })
}

/// Navigates within the currently loaded document and records a browser-history visit.
fn navigate_same_document(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    topic_index: usize,
) {
    dismiss_main_transients(ui, state);
    let changed = {
        let mut state = state.borrow_mut();
        let Some(document) = state.document.as_ref() else {
            return;
        };
        if topic_index >= document.presentations().len() || topic_index == state.topic_index {
            return;
        }
        let Some(current) = current_location(&state) else {
            return;
        };
        let next = NavigationLocation {
            source_path: document.source_path().to_path_buf(),
            topic_index,
            topic_offset: document.topic_start_offset(topic_index),
            window_name: None,
        };
        state.history.visit(current, &next);
        state.topic_index = topic_index;
        true
    };
    if changed {
        refresh_topic_layout(ui, state);
        refresh_history_list(ui, state);
        ui.scrolling_canvas.set_focus();
        execute_current_topic_macros_main(ui, state);
    }
}

/// Moves by physical presentation index, deliberately ignoring any authored HLP browse sequence.
fn navigate_adjacent_topic(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    forward: bool,
) {
    let target = {
        let state = state.borrow();
        let Some(document) = state.document.as_ref() else {
            return;
        };
        let topic_count = document.presentations().len();
        if forward {
            state
                .topic_index
                .checked_add(1)
                .filter(|&index| index < topic_count)
        } else {
            state.topic_index.checked_sub(1)
        }
    };

    if let Some(index) = target {
        navigate_same_document(ui, state, index);
    } else {
        ui.status_bar.set_status_text(
            if forward {
                "Already at the last topic"
            } else {
                "Already at the first topic"
            },
            0,
        );
    }
}

fn navigate_contents(ui: &Rc<ViewerUi>, state: &Rc<RefCell<ViewerState>>) {
    let target = {
        let state = state.borrow();
        state
            .navigation_document
            .as_ref()
            .and_then(|document| {
                document
                    .contents_topic_index()
                    .map(|topic_index| (document.clone(), topic_index))
            })
    };
    if let Some((document, index)) = target {
        navigate_main_to_document(ui, state, document, index, None);
    } else {
        ui.status_bar
            .set_status_text("The original HLP has no resolvable contents topic", 0);
    }
}

/// Restores browser history, including re-opening another HLP when a cross-file jump is reversed.
fn navigate_history(ui: &Rc<ViewerUi>, state: &Rc<RefCell<ViewerState>>, backward: bool) {
    dismiss_main_transients(ui, state);
    let (destination, history_before) = {
        let mut state = state.borrow_mut();
        let Some(current) = current_location(&state) else {
            return;
        };
        let history_before = state.history.clone();
        let destination = if backward {
            state.history.back(current)
        } else {
            state.history.forward(current)
        };
        (destination, history_before)
    };
    let Some(destination) = destination else {
        ui.status_bar.set_status_text(if backward { "No Back history" } else { "No Forward history" }, 0);
        return;
    };

    if let Err(error) = restore_location(ui, state, &destination) {
        state.borrow_mut().history = history_before;
        MessageDialog::builder(&ui.frame, &error, "Navigation failed")
            .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
            .build()
            .show_modal();
    }
    refresh_history_list(ui, state);
}

fn restore_location(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    destination: &NavigationLocation,
) -> Result<(), String> {
    dismiss_main_transients(ui, state);
    let same_file = state
        .borrow()
        .document
        .as_ref()
        .is_some_and(|document| document.source_path() == destination.source_path.as_path());

    if same_file {
        let index = {
            let state = state.borrow();
            let document = state.document.as_ref().expect("same-file check required document");
            destination
                .topic_offset
                .and_then(|offset| document.resolve_topic_offset(offset))
                .unwrap_or(destination.topic_index)
        };
        {
            let mut state = state.borrow_mut();
            let count = state.document.as_ref().map_or(0, |document| document.presentations().len());
            if index >= count {
                return Err(format!("Saved topic index {index} is outside the help file"));
            }
            state.topic_index = index;
        }
        refresh_topic_layout(ui, state);
        ui.scrolling_canvas.set_focus();
        execute_current_topic_macros_main(ui, state);
        return Ok(());
    }

    let document = HelpDocument::open(&destination.source_path).map_err(|error| {
        format!("Could not restore {}:\n\n{error}", destination.source_path.display())
    })?;
    let index = destination
        .topic_offset
        .and_then(|offset| document.resolve_topic_offset(offset))
        .unwrap_or(destination.topic_index);
    if index >= document.presentations().len() {
        return Err(format!("Saved topic index {index} is outside {}", destination.source_path.display()));
    }
    install_document(ui, state, document, index, false);
    Ok(())
}

/// Executes only the safe navigation subset of WinHelp hotspots.
fn activate_hotspot(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    hotspot: &Hotspot,
    anchor: Option<WxPoint>,
) {
    match &hotspot.target {
        HotspotTarget::Internal { offset, popup } => {
            let document = state.borrow().document.clone();
            let Some(document) = document else {
                return;
            };
            let Some(topic_index) = document.resolve_topic_offset(*offset) else {
                ui.status_bar
                    .set_status_text(&format!("Unresolved internal TOPICOFFSET {}", offset.0), 0);
                return;
            };
            if *popup {
                show_topic_window(
                    ui,
                    state,
                    ui.frame,
                    &document,
                    topic_index,
                    None,
                    AuxiliaryKind::Popup,
                    anchor,
                );
            } else {
                route_to_main_or_default_window(ui, state, document, topic_index, Some(*offset));
            }
        }
        HotspotTarget::ContextHash { hash, popup } => {
            let document = state.borrow().document.clone();
            let Some(document) = document else {
                return;
            };
            let Some(topic_index) = document
                .topic_index_for_context_hash(*hash)
                .or_else(|| document.resolve_topic_offset(TopicOffset(*hash)))
            else {
                ui.status_bar
                    .set_status_text(&format!("Unresolved internal context hash 0x{:08X}", *hash as u32), 0);
                return;
            };
            if *popup {
                show_topic_window(
                    ui,
                    state,
                    ui.frame,
                    &document,
                    topic_index,
                    None,
                    AuxiliaryKind::Popup,
                    anchor,
                );
            } else {
                let offset = document.topic_start_offset(topic_index);
                route_to_main_or_default_window(ui, state, document, topic_index, offset);
            }
        }
        HotspotTarget::External {
            opcode,
            offset,
            window_number,
            help_file,
            window_name,
            ..
        } => activate_external_hotspot(
            ui,
            state,
            *opcode,
            *offset,
            *window_number,
            help_file.as_deref(),
            window_name.as_deref(),
            anchor,
        ),
        HotspotTarget::Macro(text) => {
            execute_macro_text_main(ui, state, text, "hotspot", anchor);
        }
    }
}

/// Resolves cross-file hotspots and dispatches popup, secondary-window, or main-window behavior.
fn activate_external_hotspot(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    opcode: u8,
    offset: TopicOffset,
    window_number: Option<u8>,
    help_file: Option<&str>,
    window_name: Option<&str>,
    anchor: Option<WxPoint>,
) {
    let Some(current_document) = state.borrow().document.clone() else {
        return;
    };
    let target_document = match load_linked_document(&current_document, help_file) {
        Ok(document) => document,
        Err((path, error)) => {
            show_open_error(ui, &path, &error);
            return;
        }
    };
    let Some(topic_index) = target_document.resolve_topic_offset(offset) else {
        ui.status_bar.set_status_text(
            &format!(
                "Unresolved TOPICOFFSET {} in {}",
                offset.0,
                target_document.source_path().display()
            ),
            0,
        );
        return;
    };

    // WinHelp uses the low bit consistently for external context commands: even opcodes are
    // popup links, odd opcodes navigate. This covers both text (EA/EB) and picture (EE/EF) links.
    if opcode & 1 == 0 {
        show_topic_window(
            ui,
            state,
            ui.frame,
            &target_document,
            topic_index,
            None,
            AuxiliaryKind::Popup,
            anchor,
        );
        return;
    }

    let explicit_window = resolve_explicit_window(&target_document, window_number, window_name);
    let explicit_main = is_explicit_main_window(window_name, explicit_window.as_ref());
    if (window_name.is_some() || window_number.is_some()) && !explicit_main {
        show_topic_window(
            ui,
            state,
            ui.frame,
            &target_document,
            topic_index,
            explicit_window.as_ref(),
            AuxiliaryKind::Secondary,
            None,
        );
        return;
    }
    if explicit_main {
        navigate_main_to_document(ui, state, target_document, topic_index, Some(offset));
        return;
    }

    route_to_main_or_default_window(ui, state, target_document, topic_index, Some(offset));
}

/// Opens the document named by a WinHelp external hotspot, or clones the current document.
fn load_linked_document(
    current_document: &HelpDocument,
    help_file: Option<&str>,
) -> Result<HelpDocument, (PathBuf, String)> {
    let target_path = help_file
        .filter(|name| !name.is_empty())
        .map_or_else(
            || current_document.source_path().to_path_buf(),
            |name| resolve_external_help_path(current_document.source_path(), name),
        );
    if target_path == current_document.source_path() {
        return Ok(current_document.clone());
    }
    HelpDocument::open(&target_path)
        .map_err(|error| (target_path, error.to_string()))
}

fn resolve_explicit_window(
    document: &HelpDocument,
    window_number: Option<u8>,
    window_name: Option<&str>,
) -> Option<WindowDefinition> {
    window_name
        .and_then(|name| document.window_by_name(name))
        .or_else(|| window_number.and_then(|number| document.window_by_number(number)))
        .cloned()
}

fn is_explicit_main_window(
    window_name: Option<&str>,
    definition: Option<&WindowDefinition>,
) -> bool {
    window_name.is_some_and(|name| name.eq_ignore_ascii_case("main"))
        || definition.is_some_and(|window| {
            window
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case("main"))
        })
}

fn non_main_default_window(document: &HelpDocument, topic_index: usize) -> Option<WindowDefinition> {
    document
        .default_window_for_topic(topic_index)
        .filter(|window| {
            !window
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case("main"))
        })
        .cloned()
}

/// Sends an ordinary jump to its HLP-assigned secondary window or to the main viewer.
fn route_to_main_or_default_window(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    document: HelpDocument,
    topic_index: usize,
    source_offset: Option<TopicOffset>,
) {
    if let Some(window) = non_main_default_window(&document, topic_index) {
        show_topic_window(
            ui,
            state,
            ui.frame,
            &document,
            topic_index,
            Some(&window),
            AuxiliaryKind::Secondary,
            None,
        );
    } else {
        navigate_main_to_document(ui, state, document, topic_index, source_offset);
    }
}

/// Navigates the main viewer to a same-file or cross-file topic while preserving browser history.
fn navigate_main_to_document(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    document: HelpDocument,
    topic_index: usize,
    source_offset: Option<TopicOffset>,
) {
    let same_file = state
        .borrow()
        .document
        .as_ref()
        .is_some_and(|current| current.source_path() == document.source_path());
    if same_file {
        navigate_same_document(ui, state, topic_index);
        return;
    }

    let next = NavigationLocation {
        source_path: document.source_path().to_path_buf(),
        topic_index,
        topic_offset: document.topic_start_offset(topic_index).or(source_offset),
        window_name: None,
    };
    {
        let mut state = state.borrow_mut();
        if let Some(current) = current_location(&state) {
            state.history.visit(current, &next);
        }
    }
    install_document(ui, state, document, topic_index, false);
}

/// Adds one bounded macro diagnostic without allowing hostile HLPs to grow memory indefinitely.
fn log_macro_diagnostic(state: &Rc<RefCell<ViewerState>>, message: impl Into<String>) {
    let message = message.into();
    let message = if message.chars().count() > MAX_MACRO_DIAGNOSTIC_CHARS {
        let mut truncated = message
            .chars()
            .take(MAX_MACRO_DIAGNOSTIC_CHARS)
            .collect::<String>();
        truncated.push('…');
        truncated
    } else {
        message
    };
    let mut state = state.borrow_mut();
    if state.macro_diagnostics.len() >= MAX_MACRO_DIAGNOSTICS {
        state.macro_diagnostics.remove(0);
    }
    state.macro_diagnostics.push(message);
}

/// Presents the runtime macro audit trail collected from CONFIG, topic, and hotspot macros.
fn show_macro_diagnostics(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    let text = {
        let state = state.borrow();
        if state.macro_diagnostics.is_empty() {
            "No WinHelp macros have been parsed or executed in this session.".to_owned()
        } else {
            state.macro_diagnostics.join("\n")
        }
    };
    MessageDialog::builder(&ui.frame, &text, "WinHelp Macro Diagnostics")
        .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
        .build()
        .show_modal();
}

/// Makes a navigation page visible before a macro moves keyboard focus into it.
fn show_navigation_page(
    ui: &ViewerUi,
    state: &Rc<RefCell<ViewerState>>,
    page: usize,
) {
    set_navigation_pane_visible(ui, state, true);
    let _ = ui.navigation.set_selection(page);
}

#[derive(Debug, Clone)]
enum MacroTopicTarget {
    Contents,
    Map(i32),
    Hash(i32),
    Id(String),
}

fn resolve_macro_topic(document: &HelpDocument, target: &MacroTopicTarget) -> Option<usize> {
    match target {
        MacroTopicTarget::Contents => document.contents_topic_index(),
        MacroTopicTarget::Map(value) => document.topic_index_for_map_id(*value),
        MacroTopicTarget::Hash(value) => document.topic_index_for_context_hash(*value),
        MacroTopicTarget::Id(value) => document.topic_index_for_context_name(value),
    }
}

fn split_macro_path_window(value: &str) -> (&str, Option<&str>) {
    value
        .split_once('>')
        .map_or((value.trim(), None), |(file, window)| {
            let window = window.trim();
            (file.trim(), (!window.is_empty()).then_some(window))
        })
}

/// Executes one macro program through a bounded, default-deny dispatcher.
fn execute_macro_text_main(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    text: &str,
    origin: &str,
    anchor: Option<WxPoint>,
) {
    let program = match HelpMacroProgram::parse(text) {
        Ok(program) => program,
        Err(error) => {
            log_macro_diagnostic(state, format!("{origin}: malformed macro blocked: {error}"));
            ui.status_bar.set_status_text("Malformed WinHelp macro blocked", 0);
            return;
        }
    };

    {
        let mut state = state.borrow_mut();
        if state.macro_execution_depth == 0 {
            state.macro_execution_budget = MAX_MACRO_EXECUTION_STEPS;
        }
        state.macro_execution_depth = state.macro_execution_depth.saturating_add(1);
    }

    for parsed in program.macros {
        let has_budget = {
            let mut state = state.borrow_mut();
            if state.macro_execution_budget == 0 {
                false
            } else {
                state.macro_execution_budget -= 1;
                true
            }
        };
        if !has_budget {
            log_macro_diagnostic(
                state,
                format!(
                    "{origin}: macro execution stopped after {MAX_MACRO_EXECUTION_STEPS} commands"
                ),
            );
            ui.status_bar.set_status_text("WinHelp macro execution limit reached", 0);
            break;
        }

        match parsed {
            HelpMacro::Allowed(command) => {
                log_macro_diagnostic(state, format!("{origin}: allowed {command:?}"));
                execute_safe_macro_main(ui, state, command, origin, anchor);
            }
            HelpMacro::Blocked(blocked) => {
                log_macro_diagnostic(
                    state,
                    format!("{origin}: blocked {} — {}", blocked.invocation, blocked.reason),
                );
                ui.status_bar
                    .set_status_text("Blocked an unsafe or unsupported WinHelp macro", 0);
            }
        }
    }

    let mut state = state.borrow_mut();
    state.macro_execution_depth = state.macro_execution_depth.saturating_sub(1);
}

fn execute_safe_macro_main(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    command: SafeHelpMacro,
    origin: &str,
    anchor: Option<WxPoint>,
) {
    match command {
        SafeHelpMacro::ALink { keywords } => macro_alink_main(ui, state, &keywords, origin),
        SafeHelpMacro::About => show_about(&ui.frame),
        SafeHelpMacro::Back => navigate_history(ui, state, true),
        SafeHelpMacro::BackFlush => {
            state.borrow_mut().history.clear();
            refresh_history_list(ui, state);
            ui.status_bar.set_status_text("WinHelp Back history cleared", 0);
        }
        SafeHelpMacro::BookmarkDefine => add_current_bookmark(ui, state),
        SafeHelpMacro::BookmarkMore => {
            show_navigation_page(ui, state, 3);
            ui.bookmarks_list.set_focus();
        }
        SafeHelpMacro::BrowseButtons => enable_macro_browse_buttons(ui, state),
        SafeHelpMacro::Contents => navigate_contents(ui, state),
        SafeHelpMacro::Finder => {
            show_navigation_page(ui, state, 1);
            ui.index_query.set_focus();
        }
        SafeHelpMacro::FocusWindow { window } => {
            if window.trim().is_empty() || window.eq_ignore_ascii_case("main") {
                ui.frame.set_focus();
            } else {
                log_macro_diagnostic(
                    state,
                    format!("{origin}: FocusWindow({window:?}) ignored: no matching live window registry"),
                );
            }
        }
        SafeHelpMacro::History => {
            show_navigation_page(ui, state, 4);
            refresh_history_list(ui, state);
            ui.history_list.set_focus();
        }
        SafeHelpMacro::JumpContents { help_file, window } => {
            macro_jump_main(
                ui,
                state,
                &help_file,
                (!window.trim().is_empty()).then_some(window.as_str()),
                MacroTopicTarget::Contents,
                origin,
            );
        }
        SafeHelpMacro::JumpContext {
            help_file,
            window,
            context,
        } => {
            macro_jump_main(
                ui,
                state,
                &help_file,
                (!window.trim().is_empty()).then_some(window.as_str()),
                MacroTopicTarget::Map(context),
                origin,
            );
        }
        SafeHelpMacro::JumpHash {
            help_file,
            window,
            hash,
        } => {
            macro_jump_main(
                ui,
                state,
                &help_file,
                (!window.trim().is_empty()).then_some(window.as_str()),
                MacroTopicTarget::Hash(hash),
                origin,
            );
        }
        SafeHelpMacro::JumpId {
            path_window,
            topic_id,
        } => {
            let (help_file, window) = split_macro_path_window(&path_window);
            macro_jump_main(
                ui,
                state,
                help_file,
                window,
                MacroTopicTarget::Id(topic_id),
                origin,
            );
        }
        SafeHelpMacro::Next => macro_browse_main(ui, state, true),
        SafeHelpMacro::Prev => macro_browse_main(ui, state, false),
        SafeHelpMacro::PopupContext { help_file, context } => {
            macro_popup_main(
                ui,
                state,
                &help_file,
                MacroTopicTarget::Map(context),
                origin,
                anchor,
            );
        }
        SafeHelpMacro::PopupHash { help_file, hash } => {
            macro_popup_main(
                ui,
                state,
                &help_file,
                MacroTopicTarget::Hash(hash),
                origin,
                anchor,
            );
        }
        SafeHelpMacro::PopupId { help_file, topic_id } => {
            macro_popup_main(
                ui,
                state,
                &help_file,
                MacroTopicTarget::Id(topic_id),
                origin,
                anchor,
            );
        }
        SafeHelpMacro::Search => {
            show_navigation_page(ui, state, 2);
            ui.search_query.set_focus();
        }
        SafeHelpMacro::SetPopupColor { red, green, blue } => {
            let key = state
                .borrow()
                .document
                .as_ref()
                .map(|document| path_identity(document.source_path()));
            if let Some(key) = key {
                // Retain the decoded macro value for diagnostics/compatibility state, but there is
                // no detached popup surface to repaint in build-fix16's single-window UI.
                state
                    .borrow_mut()
                    .popup_colors
                    .insert(key, Rgb { red, green, blue });
                ui.status_bar
                    .set_status_text("Popup colour retained; floating popup windows are disabled", 0);
            }
        }
    }
}

/// Resolves exact semicolon-delimited ALink names through the HLP A-keyword table.
fn resolve_alink_topic_indices(document: &HelpDocument, keywords: &str) -> Vec<usize> {
    let mut topic_indices = Vec::new();
    for offset in document.keywords().lookup_exact('A', keywords) {
        if let Some(index) = document.resolve_topic_offset(offset) {
            if !topic_indices.contains(&index) {
                topic_indices.push(index);
            }
        }
    }
    topic_indices
}

/// Executes a safe associative-link lookup in the main help surface.
fn macro_alink_main(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    keywords: &str,
    origin: &str,
) {
    let Some(document) = state.borrow().document.clone() else {
        return;
    };

    let topic_indices = resolve_alink_topic_indices(&document, keywords);

    if topic_indices.is_empty() {
        log_macro_diagnostic(state, format!("{origin}: ALink {keywords:?} found no A-table topics"));
        ui.status_bar.set_status_text("No related topics found", 0);
        return;
    }

    let choices = topic_indices.iter().map(|&index| {
        document.presentations().get(index)
            .map(|topic| topic_label(&topic.title, index))
            .unwrap_or_else(|| format!("Topic {}", index + 1))
    }).collect::<Vec<_>>();

    let selected = if topic_indices.len() == 1 {
        Some(0_usize)
    } else {
        let refs = choices.iter().map(String::as_str).collect::<Vec<_>>();
        let dialog = SingleChoiceDialog::builder(
            &ui.frame,
            "Select a related topic.",
            "Topics Found",
            &refs,
        ).build();
        if dialog.show_modal() != wxdragon::id::ID_OK {
            None
        } else {
            usize::try_from(dialog.get_selection()).ok()
        }
    };

    if let Some(topic_index) = selected.and_then(|selection| topic_indices.get(selection).copied()) {
        navigate_same_document(ui, state, topic_index);
    }
}

fn enable_macro_browse_buttons(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
) {
    let should_add = {
        let mut state = state.borrow_mut();
        if state.macro_browse_tools_added {
            false
        } else {
            state.macro_browse_tools_added = true;
            true
        }
    };
    if should_add {
        let empty_bitmap = Bitmap::null_bitmap();
        ui.toolbar.add_separator();
        let _ = ui.toolbar.add_tool(
            ID_BROWSE_PREVIOUS,
            "Browse Prev",
            &empty_bitmap,
            "Open the previous topic in this HLP's authored browse sequence",
        );
        let _ = ui.toolbar.add_tool(
            ID_BROWSE_NEXT,
            "Browse Next",
            &empty_bitmap,
            "Open the next topic in this HLP's authored browse sequence",
        );
        let _ = ui.toolbar.realize();
    }
    refresh_browsing_toolbar(ui, state);
}

fn macro_browse_main(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    forward: bool,
) {
    let target = {
        let state = state.borrow();
        let Some(document) = state.document.as_ref() else {
            return;
        };
        if forward {
            document.browse_next_index(state.topic_index)
        } else {
            document.browse_previous_index(state.topic_index)
        }
    };
    if let Some(index) = target {
        navigate_same_document(ui, state, index);
    } else {
        ui.status_bar.set_status_text(
            if forward {
                "No authored Next topic"
            } else {
                "No authored Previous topic"
            },
            0,
        );
    }
}

fn macro_jump_main(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    help_file: &str,
    window_name: Option<&str>,
    target: MacroTopicTarget,
    origin: &str,
) {
    let Some(current) = state.borrow().document.clone() else {
        return;
    };
    let document = match load_linked_document(&current, Some(help_file)) {
        Ok(document) => document,
        Err((path, error)) => {
            log_macro_diagnostic(
                state,
                format!("{origin}: macro jump could not open {}: {error}", path.display()),
            );
            show_open_error(ui, &path, &error);
            return;
        }
    };
    let Some(topic_index) = resolve_macro_topic(&document, &target) else {
        log_macro_diagnostic(
            state,
            format!("{origin}: unresolved macro target {target:?} in {}", document.source_path().display()),
        );
        ui.status_bar.set_status_text("Unresolved WinHelp macro target", 0);
        return;
    };

    let explicit_window = window_name
        .and_then(|name| document.window_by_name(name))
        .cloned();
    if window_name.is_some() && !is_explicit_main_window(window_name, explicit_window.as_ref()) {
        show_topic_window(
            ui,
            state,
            ui.frame,
            &document,
            topic_index,
            explicit_window.as_ref(),
            AuxiliaryKind::Secondary,
            None,
        );
    } else if window_name.is_some() {
        navigate_main_to_document(ui, state, document, topic_index, None);
    } else {
        route_to_main_or_default_window(ui, state, document, topic_index, None);
    }
}

fn macro_popup_main(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
    help_file: &str,
    target: MacroTopicTarget,
    origin: &str,
    anchor: Option<WxPoint>,
) {
    let Some(current) = state.borrow().document.clone() else {
        return;
    };
    let document = match load_linked_document(&current, Some(help_file)) {
        Ok(document) => document,
        Err((path, error)) => {
            log_macro_diagnostic(
                state,
                format!("{origin}: popup macro could not open {}: {error}", path.display()),
            );
            show_open_error(ui, &path, &error);
            return;
        }
    };
    let Some(topic_index) = resolve_macro_topic(&document, &target) else {
        log_macro_diagnostic(
            state,
            format!("{origin}: unresolved popup macro target {target:?}"),
        );
        ui.status_bar.set_status_text("Unresolved WinHelp popup macro target", 0);
        return;
    };
    show_topic_window(
        ui,
        state,
        ui.frame,
        &document,
        topic_index,
        None,
        AuxiliaryKind::Popup,
        anchor,
    );
}

fn execute_document_config_macros_main(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
) {
    let macros = state
        .borrow()
        .document
        .as_ref()
        .map(|document| document.system().config_macros.clone())
        .unwrap_or_default();
    for (index, text) in macros.iter().enumerate() {
        execute_macro_text_main(ui, state, text, &format!("CONFIG #{}", index + 1), None);
    }
}

fn execute_current_topic_macros_main(
    ui: &Rc<ViewerUi>,
    state: &Rc<RefCell<ViewerState>>,
) {
    let (topic_index, macros) = {
        let state = state.borrow();
        let Some(document) = state.document.as_ref() else {
            return;
        };
        let topic_index = state.topic_index;
        let macros = document
            .topics()
            .get(topic_index)
            .map(|topic| topic.macros.clone())
            .unwrap_or_default();
        (topic_index, macros)
    };
    for (index, text) in macros.iter().enumerate() {
        execute_macro_text_main(
            ui,
            state,
            text,
            &format!("topic {} macro #{}", topic_index + 1, index + 1),
            None,
        );
    }
}

/// Compatibility routing shim for WinHelp popup/secondary metadata. No floating frame is created.
fn show_topic_window(
    main_ui: &Rc<ViewerUi>,
    main_state: &Rc<RefCell<ViewerState>>,
    _owner: Frame,
    document: &HelpDocument,
    topic_index: usize,
    _definition: Option<&WindowDefinition>,
    _kind: AuxiliaryKind,
    _anchor: Option<WxPoint>,
) {
    // build-fix16 deliberately removes native floating topic windows. WinHelp popup and
    // secondary-window metadata still resolve the correct destination, but that destination
    // is installed in the one main viewer surface so links remain functional and Back works.
    let source_offset = document.topic_start_offset(topic_index);
    navigate_main_to_document(
        main_ui,
        main_state,
        document.clone(),
        topic_index,
        source_offset,
    );
}


fn auxiliary_frame_style(
    definition: Option<&WindowDefinition>,
    kind: AuxiliaryKind,
) -> FrameStyle {
    match kind {
        AuxiliaryKind::Popup => {
            FrameStyle::NoTaskbar | FrameStyle::FloatOnParent | FrameStyle::ClipChildren
        }
        AuxiliaryKind::Secondary => {
            let mut style = FrameStyle::Default | FrameStyle::ClipChildren;
            if definition.is_some_and(|window| window.always_on_top) {
                style |= FrameStyle::StayOnTop;
            }
            style
        }
    }
}

fn window_position(window: &WindowDefinition) -> Option<WxPoint> {
    let x = i32::from(window.x?);
    let y = i32::from(window.y?);
    (x >= 0 && y >= 0).then_some(WxPoint { x, y })
}

fn auxiliary_width(definition: Option<&WindowDefinition>, kind: AuxiliaryKind) -> i32 {
    let default_width = if kind == AuxiliaryKind::Popup {
        POPUP_DEFAULT_WIDTH
    } else {
        760
    };
    definition
        .and_then(|window| window.width)
        .filter(|value| *value > 0)
        .map_or(default_width, |value| {
            (i32::from(value) * 9 / 10).clamp(360, 1200)
        })
}

fn auxiliary_height(
    document: &HelpDocument,
    presentation: &hlp::TopicPresentation,
    definition: Option<&WindowDefinition>,
    kind: AuxiliaryKind,
    width: i32,
) -> i32 {
    if kind == AuxiliaryKind::Popup || definition.is_some_and(|window| window.auto_size_height) {
        let layout_width = width.saturating_sub(32).max(MIN_LAYOUT_WIDTH);
        let layout = LayoutEngine::default().layout_topic(presentation, document.fonts(), layout_width);
        let content_height = layout
            .fixed
            .height
            .saturating_add(layout.scrolling.height)
            .saturating_add(16);
        if kind == AuxiliaryKind::Popup {
            return content_height.clamp(POPUP_MIN_HEIGHT, POPUP_MAX_HEIGHT);
        }
        return content_height.clamp(240, 900);
    }
    definition
        .and_then(|window| window.height)
        .filter(|value| *value > 0)
        .map_or(560, |value| (i32::from(value) * 7 / 10).clamp(240, 900))
}

/// Repaints one auxiliary region from its current mutable topic state.
fn bind_auxiliary_paint(
    canvas: Panel,
    state: Rc<RefCell<AuxiliaryState>>,
    fixed_region: bool,
) {
    canvas.on_paint(move |_event: WindowEventData| {
        invalidate_whole_canvas(canvas);
        let dc = PaintDC::new(&canvas);
        let state = state.borrow();
        let background = auxiliary_background(&state, fixed_region);
        dc.set_background(colour_from_rgb(background));
        dc.clear();
        dc.set_background_mode(wxdragon::dc::BackgroundMode::Transparent);
        let Some(layout) = &state.layout else {
            return;
        };
        let region = if fixed_region {
            &layout.fixed
        } else {
            &layout.scrolling
        };
        paint_region(canvas, &dc, region, state.text_zoom_percent, background);
    });
}

fn auxiliary_background(state: &AuxiliaryState, fixed_region: bool) -> Rgb {
    if state.kind == AuxiliaryKind::Popup {
        if let Some(color) = state.popup_color_override {
            return color;
        }
    }
    let from_definition = state.definition.as_ref().and_then(|window| {
        if fixed_region {
            window.non_scrolling_color
        } else {
            window.scrolling_color
        }
    });
from_definition.unwrap_or(if state.kind == AuxiliaryKind::Popup { POPUP_BACKGROUND } else { HELP_BACKGROUND })
}

/// Gives auxiliary windows the same hyperlink hit-testing as the main topic surface.
fn bind_auxiliary_hotspot_handler(
    main_ui: Rc<ViewerUi>,
    main_state: Rc<RefCell<ViewerState>>,
    ui: Rc<AuxiliaryUi>,
    state: Rc<RefCell<AuxiliaryState>>,
    fixed_region: bool,
) {
    let canvas = if fixed_region {
        ui.fixed_canvas
    } else {
        ui.scrolling_canvas
    };
    canvas.on_mouse_left_down(move |event: WindowEventData| {
        canvas.set_tooltip("");
        {
            let mut state = state.borrow_mut();
            state.tooltip_generation = state.tooltip_generation.wrapping_add(1);
        }
        let WindowEventData::MouseButton(mouse) = event else {
            return;
        };
        let Some(position) = mouse.get_position() else {
            return;
        };
        let hit = {
            let state = state.borrow();
            let Some(layout) = &state.layout else {
                return;
            };
            let region = if fixed_region {
                &layout.fixed
            } else {
                &layout.scrolling
            };
            region
                .hit_test_box(LayoutPoint {
                    x: position.x,
                    y: position.y,
                })
                .and_then(|item| item.hotspot().cloned().map(|hotspot| (item.bounds, hotspot)))
        };
        let Some((bounds, hotspot)) = hit else {
            return;
        };
        let anchor = canvas.client_to_screen(WxPoint {
            x: bounds.x.saturating_add(12),
            y: bounds.y.saturating_add(bounds.height).saturating_add(4),
        });
        activate_auxiliary_hotspot(
            &main_ui,
            &main_state,
            &ui,
            &state,
            &hotspot,
            Some(anchor),
        );
    });
}

/// Implements transient popup dismissal and Escape handling without affecting secondary windows.
fn bind_auxiliary_lifetime(
    ui: Rc<AuxiliaryUi>,
    state: Rc<RefCell<AuxiliaryState>>,
    main_state: Rc<RefCell<ViewerState>>,
) {
    let frame_for_activate = ui.frame;
    let state_for_activate = Rc::clone(&state);
    ui.frame.on_activate(move |event: WindowEventData| {
        if let WindowEventData::Activate(activation) = &event {
            let should_close = {
                let mut state = state_for_activate.borrow_mut();
                if activation.is_active() {
                    state.activated_once = true;
                    false
                } else {
                    state.kind == AuxiliaryKind::Popup && state.activated_once
                }
            };
            if should_close {
                frame_for_activate.close(true);
            }
        }
        event.skip(true);
    });

    if state.borrow().kind == AuxiliaryKind::Popup {
        let tracked_handle = ui.frame.get_handle() as usize;
        let main_state_for_close = Rc::clone(&main_state);
        ui.frame.on_close(move |event: WindowEventData| {
            let mut main_state = main_state_for_close.borrow_mut();
            let is_tracked = main_state
                .active_popup
                .is_some_and(|popup| popup.get_handle() as usize == tracked_handle);
            if is_tracked {
                main_state.active_popup = None;
            }
            event.skip(true);
        });

        bind_escape_handler(ui.frame, ui.frame);
        bind_escape_handler(ui.host, ui.frame);
        bind_escape_handler(ui.fixed_canvas, ui.frame);
        bind_escape_handler(ui.scrolled, ui.frame);
        bind_escape_handler(ui.scrolling_canvas, ui.frame);
    }
}

fn bind_escape_handler(widget: impl WxWidget + WindowEvents + Copy + 'static, frame: Frame) {
    widget.on_key_down(move |event: WindowEventData| {
        let escape = matches!(
            &event,
            WindowEventData::Keyboard(keyboard) if keyboard.get_key_code() == Some(VK_ESCAPE)
        );
        if escape {
            frame.close(true);
        } else {
            event.skip(true);
        }
    });
}

/// Executes a WinHelp macro program in the context of a popup or secondary help window.
fn execute_macro_text_auxiliary(
    main_ui: &Rc<ViewerUi>,
    main_state: &Rc<RefCell<ViewerState>>,
    ui: &Rc<AuxiliaryUi>,
    state: &Rc<RefCell<AuxiliaryState>>,
    text: &str,
    origin: &str,
    anchor: Option<WxPoint>,
) {
    let program = match HelpMacroProgram::parse(text) {
        Ok(program) => program,
        Err(error) => {
            log_macro_diagnostic(main_state, format!("{origin}: malformed macro blocked: {error}"));
            main_ui.status_bar.set_status_text("Malformed WinHelp macro blocked", 0);
            return;
        }
    };

    {
        let mut main_state = main_state.borrow_mut();
        if main_state.macro_execution_depth == 0 {
            main_state.macro_execution_budget = MAX_MACRO_EXECUTION_STEPS;
        }
        main_state.macro_execution_depth = main_state.macro_execution_depth.saturating_add(1);
    }

    for parsed in program.macros {
        let has_budget = {
            let mut main_state = main_state.borrow_mut();
            if main_state.macro_execution_budget == 0 {
                false
            } else {
                main_state.macro_execution_budget -= 1;
                true
            }
        };
        if !has_budget {
            log_macro_diagnostic(
                main_state,
                format!(
                    "{origin}: macro execution stopped after {MAX_MACRO_EXECUTION_STEPS} commands"
                ),
            );
            main_ui
                .status_bar
                .set_status_text("WinHelp macro execution limit reached", 0);
            break;
        }
        match parsed {
            HelpMacro::Allowed(command) => {
                log_macro_diagnostic(main_state, format!("{origin}: allowed {command:?}"));
                execute_safe_macro_auxiliary(
                    main_ui,
                    main_state,
                    ui,
                    state,
                    command,
                    origin,
                    anchor,
                );
            }
            HelpMacro::Blocked(blocked) => {
                log_macro_diagnostic(
                    main_state,
                    format!("{origin}: blocked {} — {}", blocked.invocation, blocked.reason),
                );
                main_ui
                    .status_bar
                    .set_status_text("Blocked an unsafe or unsupported WinHelp macro", 0);
            }
        }
    }

    let mut main_state = main_state.borrow_mut();
    main_state.macro_execution_depth = main_state.macro_execution_depth.saturating_sub(1);
}

fn execute_safe_macro_auxiliary(
    main_ui: &Rc<ViewerUi>,
    main_state: &Rc<RefCell<ViewerState>>,
    ui: &Rc<AuxiliaryUi>,
    state: &Rc<RefCell<AuxiliaryState>>,
    command: SafeHelpMacro,
    origin: &str,
    anchor: Option<WxPoint>,
) {
    match command {
        SafeHelpMacro::ALink { keywords } => {
            let document = state.borrow().document.clone();
            let topic_indices = resolve_alink_topic_indices(&document, &keywords);
            if topic_indices.is_empty() {
                main_ui.status_bar.set_status_text("No related topics found", 0);
            } else {
                let choices = topic_indices.iter().map(|&index| {
                    document.presentations().get(index)
                        .map(|topic| topic_label(&topic.title, index))
                        .unwrap_or_else(|| format!("Topic {}", index + 1))
                }).collect::<Vec<_>>();
                let selected = if topic_indices.len() == 1 {
                    Some(0_usize)
                } else {
                    let refs = choices.iter().map(String::as_str).collect::<Vec<_>>();
                    let dialog = SingleChoiceDialog::builder(
                        &ui.frame,
                        "Select a related topic.",
                        "Topics Found",
                        &refs,
                    ).build();
                    if dialog.show_modal() != wxdragon::id::ID_OK {
                        None
                    } else {
                        usize::try_from(dialog.get_selection()).ok()
                    }
                };
                if let Some(topic_index) = selected.and_then(|selection| topic_indices.get(selection).copied()) {
                    follow_regular_auxiliary_jump(
                        main_ui, main_state, ui, state, document, topic_index, None,
                    );
                }
            }
        }
        SafeHelpMacro::About => show_about(&ui.frame),
        SafeHelpMacro::Back | SafeHelpMacro::BackFlush => {
            log_macro_diagnostic(
                main_state,
                format!("{origin}: auxiliary Back history is not modeled; command ignored"),
            );
        }
        SafeHelpMacro::BookmarkDefine => add_auxiliary_bookmark(main_ui, main_state, state),
        SafeHelpMacro::BookmarkMore => {
            show_navigation_page(main_ui, main_state, 3);
            main_ui.bookmarks_list.set_focus();
        }
        SafeHelpMacro::BrowseButtons => enable_macro_browse_buttons(main_ui, main_state),
        SafeHelpMacro::Contents => {
            let (document, target) = {
                let state = state.borrow();
                (
                    state.document.clone(),
                    state.document.contents_topic_index(),
                )
            };
            if let Some(topic_index) = target {
                follow_regular_auxiliary_jump(
                    main_ui,
                    main_state,
                    ui,
                    state,
                    document,
                    topic_index,
                    None,
                );
            }
        }
        SafeHelpMacro::Finder => {
            show_navigation_page(main_ui, main_state, 1);
            main_ui.index_query.set_focus();
        }
        SafeHelpMacro::FocusWindow { window } => {
            let is_main = window.trim().is_empty() || window.eq_ignore_ascii_case("main");
            let is_current = state
                .borrow()
                .definition
                .as_ref()
                .and_then(|definition| definition.name.as_deref())
                .is_some_and(|name| name.eq_ignore_ascii_case(window.trim()));
            if is_main {
                main_ui.frame.set_focus();
            } else if is_current {
                ui.frame.set_focus();
            } else {
                log_macro_diagnostic(
                    main_state,
                    format!("{origin}: FocusWindow({window:?}) did not match this auxiliary window"),
                );
            }
        }
        SafeHelpMacro::History => {
            show_navigation_page(main_ui, main_state, 4);
            refresh_history_list(main_ui, main_state);
            main_ui.history_list.set_focus();
        }
        SafeHelpMacro::JumpContents { help_file, window } => macro_jump_auxiliary(
            main_ui,
            main_state,
            ui,
            state,
            &help_file,
            (!window.trim().is_empty()).then_some(window.as_str()),
            MacroTopicTarget::Contents,
            origin,
        ),
        SafeHelpMacro::JumpContext {
            help_file,
            window,
            context,
        } => macro_jump_auxiliary(
            main_ui,
            main_state,
            ui,
            state,
            &help_file,
            (!window.trim().is_empty()).then_some(window.as_str()),
            MacroTopicTarget::Map(context),
            origin,
        ),
        SafeHelpMacro::JumpHash {
            help_file,
            window,
            hash,
        } => macro_jump_auxiliary(
            main_ui,
            main_state,
            ui,
            state,
            &help_file,
            (!window.trim().is_empty()).then_some(window.as_str()),
            MacroTopicTarget::Hash(hash),
            origin,
        ),
        SafeHelpMacro::JumpId {
            path_window,
            topic_id,
        } => {
            let (help_file, window) = split_macro_path_window(&path_window);
            macro_jump_auxiliary(
                main_ui,
                main_state,
                ui,
                state,
                help_file,
                window,
                MacroTopicTarget::Id(topic_id),
                origin,
            );
        }
        SafeHelpMacro::Next => macro_browse_auxiliary(main_ui, main_state, ui, state, true),
        SafeHelpMacro::Prev => macro_browse_auxiliary(main_ui, main_state, ui, state, false),
        SafeHelpMacro::PopupContext { help_file, context } => macro_popup_auxiliary(
            main_ui,
            main_state,
            ui,
            state,
            &help_file,
            MacroTopicTarget::Map(context),
            origin,
            anchor,
        ),
        SafeHelpMacro::PopupHash { help_file, hash } => macro_popup_auxiliary(
            main_ui,
            main_state,
            ui,
            state,
            &help_file,
            MacroTopicTarget::Hash(hash),
            origin,
            anchor,
        ),
        SafeHelpMacro::PopupId { help_file, topic_id } => macro_popup_auxiliary(
            main_ui,
            main_state,
            ui,
            state,
            &help_file,
            MacroTopicTarget::Id(topic_id),
            origin,
            anchor,
        ),
        SafeHelpMacro::Search => {
            show_navigation_page(main_ui, main_state, 2);
            main_ui.search_query.set_focus();
        }
        SafeHelpMacro::SetPopupColor { red, green, blue } => {
            let (key, is_popup) = {
                let state = state.borrow();
                (path_identity(state.document.source_path()), state.kind == AuxiliaryKind::Popup)
            };
            let color = Rgb { red, green, blue };
            main_state.borrow_mut().popup_colors.insert(key, color);
            if is_popup {
                state.borrow_mut().popup_color_override = Some(color);
                ui.fixed_canvas.refresh(false, None);
                ui.scrolling_canvas.refresh(false, None);
            }
        }
    }
}

fn macro_browse_auxiliary(
    main_ui: &Rc<ViewerUi>,
    main_state: &Rc<RefCell<ViewerState>>,
    ui: &Rc<AuxiliaryUi>,
    state: &Rc<RefCell<AuxiliaryState>>,
    forward: bool,
) {
    let (document, target) = {
        let state = state.borrow();
        let target = if forward {
            state.document.browse_next_index(state.topic_index)
        } else {
            state.document.browse_previous_index(state.topic_index)
        };
        (state.document.clone(), target)
    };
    if let Some(topic_index) = target {
        follow_regular_auxiliary_jump(
            main_ui,
            main_state,
            ui,
            state,
            document,
            topic_index,
            None,
        );
    }
}

fn add_auxiliary_bookmark(
    main_ui: &ViewerUi,
    main_state: &Rc<RefCell<ViewerState>>,
    state: &Rc<RefCell<AuxiliaryState>>,
) {
    let (location, label) = {
        let state = state.borrow();
        let file = state
            .document
            .source_path()
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("help file");
        let title = state
            .document
            .presentations()
            .get(state.topic_index)
            .map(|topic| topic_label(&topic.title, state.topic_index))
            .unwrap_or_else(|| format!("Topic {}", state.topic_index + 1));
        (
            NavigationLocation {
                source_path: state.document.source_path().to_path_buf(),
                topic_index: state.topic_index,
                topic_offset: state.document.topic_start_offset(state.topic_index),
                window_name: state.definition.as_ref().and_then(|window| window.name.clone()),
            },
            format!("{title} — {file}"),
        )
    };
    let mut main_state_mut = main_state.borrow_mut();
    let added = if main_state_mut
        .bookmarks
        .iter()
        .any(|bookmark| bookmark.location == location)
    {
        false
    } else {
        main_state_mut.bookmarks.push(BookmarkEntry { label, location });
        true
    };
    drop(main_state_mut);
    refresh_bookmark_list(main_ui, main_state);
    if added {
        if let Err(error) = save_bookmarks(main_state) {
            main_ui.status_bar.set_status_text(
                &format!("Bookmark added, but could not save: {error}"),
                0,
            );
        }
    }
}

fn macro_jump_auxiliary(
    main_ui: &Rc<ViewerUi>,
    main_state: &Rc<RefCell<ViewerState>>,
    ui: &Rc<AuxiliaryUi>,
    state: &Rc<RefCell<AuxiliaryState>>,
    help_file: &str,
    window_name: Option<&str>,
    target: MacroTopicTarget,
    origin: &str,
) {
    let current = state.borrow().document.clone();
    let document = match load_linked_document(&current, Some(help_file)) {
        Ok(document) => document,
        Err((path, error)) => {
            log_macro_diagnostic(
                main_state,
                format!("{origin}: macro jump could not open {}: {error}", path.display()),
            );
            show_open_error(main_ui, &path, &error);
            return;
        }
    };
    let Some(topic_index) = resolve_macro_topic(&document, &target) else {
        log_macro_diagnostic(
            main_state,
            format!("{origin}: unresolved auxiliary macro target {target:?}"),
        );
        return;
    };
    let explicit_window = window_name
        .and_then(|name| document.window_by_name(name))
        .cloned();
    if is_explicit_main_window(window_name, explicit_window.as_ref()) {
        if state.borrow().kind == AuxiliaryKind::Popup {
            {
                let mut state = state.borrow_mut();
                state.macro_navigation_generation = state.macro_navigation_generation.wrapping_add(1);
            }
            ui.frame.close(true);
        }
        navigate_main_to_document(main_ui, main_state, document, topic_index, None);
        return;
    }
    if window_name.is_some() {
        if state.borrow().kind == AuxiliaryKind::Popup {
            {
                let mut state = state.borrow_mut();
                state.macro_navigation_generation = state.macro_navigation_generation.wrapping_add(1);
            }
            ui.frame.close(true);
        }
        if state.borrow().kind == AuxiliaryKind::Secondary
            && window_definitions_match(state.borrow().definition.as_ref(), explicit_window.as_ref())
        {
            install_auxiliary_document(
                main_ui,
                main_state,
                ui,
                state,
                document,
                topic_index,
                explicit_window,
            );
        } else {
            show_topic_window(
                main_ui,
                main_state,
                main_ui.frame,
                &document,
                topic_index,
                explicit_window.as_ref(),
                AuxiliaryKind::Secondary,
                None,
            );
        }
        return;
    }
    follow_regular_auxiliary_jump(
        main_ui,
        main_state,
        ui,
        state,
        document,
        topic_index,
        None,
    );
}

fn macro_popup_auxiliary(
    main_ui: &Rc<ViewerUi>,
    main_state: &Rc<RefCell<ViewerState>>,
    ui: &Rc<AuxiliaryUi>,
    state: &Rc<RefCell<AuxiliaryState>>,
    help_file: &str,
    target: MacroTopicTarget,
    origin: &str,
    anchor: Option<WxPoint>,
) {
    let current = state.borrow().document.clone();
    let document = match load_linked_document(&current, Some(help_file)) {
        Ok(document) => document,
        Err((path, error)) => {
            log_macro_diagnostic(
                main_state,
                format!("{origin}: popup macro could not open {}: {error}", path.display()),
            );
            return;
        }
    };
    let Some(topic_index) = resolve_macro_topic(&document, &target) else {
        log_macro_diagnostic(
            main_state,
            format!("{origin}: unresolved auxiliary popup target {target:?}"),
        );
        return;
    };
    replace_or_open_popup(
        main_ui,
        main_state,
        ui,
        state,
        document,
        topic_index,
        anchor,
    );
}

fn execute_document_config_macros_auxiliary(
    main_ui: &Rc<ViewerUi>,
    main_state: &Rc<RefCell<ViewerState>>,
    ui: &Rc<AuxiliaryUi>,
    state: &Rc<RefCell<AuxiliaryState>>,
) {
    let macros = state.borrow().document.system().config_macros.clone();
    for (index, text) in macros.iter().enumerate() {
        execute_macro_text_auxiliary(
            main_ui,
            main_state,
            ui,
            state,
            text,
            &format!("aux CONFIG #{}", index + 1),
            None,
        );
    }
}

fn execute_current_topic_macros_auxiliary(
    main_ui: &Rc<ViewerUi>,
    main_state: &Rc<RefCell<ViewerState>>,
    ui: &Rc<AuxiliaryUi>,
    state: &Rc<RefCell<AuxiliaryState>>,
) {
    let (topic_index, macros) = {
        let state = state.borrow();
        let topic_index = state.topic_index;
        let macros = state
            .document
            .topics()
            .get(topic_index)
            .map(|topic| topic.macros.clone())
            .unwrap_or_default();
        (topic_index, macros)
    };
    for (index, text) in macros.iter().enumerate() {
        execute_macro_text_auxiliary(
            main_ui,
            main_state,
            ui,
            state,
            text,
            &format!("aux topic {} macro #{}", topic_index + 1, index + 1),
            None,
        );
    }
}

/// Executes a hyperlink clicked inside a popup or secondary window.
fn activate_auxiliary_hotspot(
    main_ui: &Rc<ViewerUi>,
    main_state: &Rc<RefCell<ViewerState>>,
    ui: &Rc<AuxiliaryUi>,
    state: &Rc<RefCell<AuxiliaryState>>,
    hotspot: &Hotspot,
    anchor: Option<WxPoint>,
) {
    match &hotspot.target {
        HotspotTarget::Macro(text) => {
            execute_macro_text_auxiliary(
                main_ui,
                main_state,
                ui,
                state,
                text,
                "auxiliary hotspot",
                anchor,
            );
        },
        HotspotTarget::Internal { offset, popup } => {
            let current = state.borrow().document.clone();
            let Some(topic_index) = current.resolve_topic_offset(*offset) else {
                return;
            };
            if *popup {
                replace_or_open_popup(main_ui, main_state, ui, state, current, topic_index, anchor);
            } else {
                follow_regular_auxiliary_jump(
                    main_ui,
                    main_state,
                    ui,
                    state,
                    current,
                    topic_index,
                    Some(*offset),
                );
            }
        }
        HotspotTarget::ContextHash { hash, popup } => {
            let current = state.borrow().document.clone();
            let Some(topic_index) = current
                .topic_index_for_context_hash(*hash)
                .or_else(|| current.resolve_topic_offset(TopicOffset(*hash)))
            else {
                return;
            };
            if *popup {
                replace_or_open_popup(main_ui, main_state, ui, state, current, topic_index, anchor);
            } else {
                let offset = current.topic_start_offset(topic_index);
                follow_regular_auxiliary_jump(
                    main_ui,
                    main_state,
                    ui,
                    state,
                    current,
                    topic_index,
                    offset,
                );
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
            let current = state.borrow().document.clone();
            let target = match load_linked_document(&current, help_file.as_deref()) {
                Ok(document) => document,
                Err((path, error)) => {
                    let parent = auxiliary_feedback_parent(main_ui, ui, state);
                    MessageDialog::builder(
                        &parent,
                        &format!("Could not open:\n{}\n\n{error}", path.display()),
                        "Navigation failed",
                    )
                    .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
                    .build()
                    .show_modal();
                    return;
                }
            };
            let Some(topic_index) = target.resolve_topic_offset(*offset) else {
                return;
            };
            if *opcode & 1 == 0 {
                replace_or_open_popup(main_ui, main_state, ui, state, target, topic_index, anchor);
                return;
            }

            let explicit_window = resolve_explicit_window(&target, *window_number, window_name.as_deref());
            if is_explicit_main_window(window_name.as_deref(), explicit_window.as_ref()) {
                if state.borrow().kind == AuxiliaryKind::Popup {
                    ui.frame.close(true);
                }
                navigate_main_to_document(main_ui, main_state, target, topic_index, Some(*offset));
                return;
            }
            if window_name.is_some() || window_number.is_some() {
                if state.borrow().kind == AuxiliaryKind::Popup {
                    ui.frame.close(true);
                }
                if state.borrow().kind == AuxiliaryKind::Secondary
                    && window_definitions_match(state.borrow().definition.as_ref(), explicit_window.as_ref())
                {
                    install_auxiliary_document(main_ui, main_state, ui, state, target, topic_index, explicit_window);
                } else {
                    show_topic_window(
                        main_ui,
                        main_state,
                        main_ui.frame,
                        &target,
                        topic_index,
                        explicit_window.as_ref(),
                        AuxiliaryKind::Secondary,
                        None,
                    );
                }
                return;
            }

            follow_regular_auxiliary_jump(
                main_ui,
                main_state,
                ui,
                state,
                target,
                topic_index,
                Some(*offset),
            );
        }
    }
}

/// Returns a stable owner for modal feedback; transient popups are dismissed first.
fn auxiliary_feedback_parent(
    main_ui: &Rc<ViewerUi>,
    ui: &Rc<AuxiliaryUi>,
    state: &Rc<RefCell<AuxiliaryState>>,
) -> Frame {
    if state.borrow().kind == AuxiliaryKind::Popup {
        ui.frame.close(true);
        main_ui.frame
    } else {
        ui.frame
    }
}

/// Popup-to-popup links replace the transient popup; secondary-window popup links open a child popup.
fn replace_or_open_popup(
    main_ui: &Rc<ViewerUi>,
    main_state: &Rc<RefCell<ViewerState>>,
    ui: &Rc<AuxiliaryUi>,
    state: &Rc<RefCell<AuxiliaryState>>,
    document: HelpDocument,
    topic_index: usize,
    anchor: Option<WxPoint>,
) {
    let kind = state.borrow().kind;
    let owner = if kind == AuxiliaryKind::Popup {
        {
            let mut state = state.borrow_mut();
            state.macro_navigation_generation = state.macro_navigation_generation.wrapping_add(1);
        }
        ui.frame.close(true);
        main_ui.frame
    } else {
        ui.frame
    };
    show_topic_window(
        main_ui,
        main_state,
        owner,
        &document,
        topic_index,
        None,
        AuxiliaryKind::Popup,
        anchor,
    );
}

/// Regular jumps stay inside secondary windows; regular jumps leave a transient popup.
fn follow_regular_auxiliary_jump(
    main_ui: &Rc<ViewerUi>,
    main_state: &Rc<RefCell<ViewerState>>,
    ui: &Rc<AuxiliaryUi>,
    state: &Rc<RefCell<AuxiliaryState>>,
    document: HelpDocument,
    topic_index: usize,
    source_offset: Option<TopicOffset>,
) {
    if state.borrow().kind == AuxiliaryKind::Secondary {
        install_auxiliary_document(main_ui, main_state, ui, state, document, topic_index, None);
    } else {
        {
            let mut state = state.borrow_mut();
            state.macro_navigation_generation = state.macro_navigation_generation.wrapping_add(1);
        }
        ui.frame.close(true);
        route_to_main_or_default_window(main_ui, main_state, document, topic_index, source_offset);
    }
}

fn window_definitions_match(
    current: Option<&WindowDefinition>,
    target: Option<&WindowDefinition>,
) -> bool {
    match (current, target) {
        (Some(left), Some(right)) => match (left.name.as_deref(), right.name.as_deref()) {
            (Some(left_name), Some(right_name)) => left_name.eq_ignore_ascii_case(right_name),
            _ => left == right,
        },
        (None, None) => true,
        _ => false,
    }
}

/// Replaces the document/topic shown by an existing secondary window.
fn install_auxiliary_document(
    main_ui: &Rc<ViewerUi>,
    main_state: &Rc<RefCell<ViewerState>>,
    ui: &Rc<AuxiliaryUi>,
    state: &Rc<RefCell<AuxiliaryState>>,
    document: HelpDocument,
    topic_index: usize,
    definition: Option<WindowDefinition>,
) {
    let previous_identity = path_identity(state.borrow().document.source_path());
    let installed_identity = path_identity(document.source_path());
    let changed_file = previous_identity != installed_identity;
    {
        let mut state = state.borrow_mut();
        state.macro_navigation_generation = state.macro_navigation_generation.wrapping_add(1);
        state.document = document;
        state.topic_index = topic_index;
        if definition.is_some() {
            state.definition = definition;
        }
        state.layout = None;
        state.layout_width = 0;
        state.popup_color_override = if state.kind == AuxiliaryKind::Popup {
            main_state.borrow().popup_colors.get(&installed_identity).copied()
        } else {
            None
        };
        state.tooltip_generation = state.tooltip_generation.wrapping_add(1);
    }
    ui.fixed_canvas.set_tooltip("");
    ui.scrolling_canvas.set_tooltip("");
    refresh_auxiliary_layout(ui, state);
    if changed_file {
        execute_document_config_macros_auxiliary(main_ui, main_state, ui, state);
    }
    execute_current_topic_macros_auxiliary(main_ui, main_state, ui, state);
}

/// Rebuilds retained geometry after auxiliary navigation or resize.
fn refresh_auxiliary_layout(ui: &AuxiliaryUi, state: &Rc<RefCell<AuxiliaryState>>) {
    let width = usable_layout_width(ui.scrolled);
    let (fixed_height, scrolling_height, has_fixed, fixed_background, scrolling_background, title, kind) = {
        let mut state = state.borrow_mut();
        let topic_count = state.document.presentations().len();
        if topic_count == 0 {
            state.layout = None;
            return;
        }
        state.topic_index = state.topic_index.min(topic_count - 1);
        let topic_index = state.topic_index;
        let (layout, has_fixed, title) = {
            let presentation = &state.document.presentations()[topic_index];
            let has_fixed = !presentation.non_scrolling.is_empty();
            let title = if presentation.title.is_empty() {
                "Help".to_owned()
            } else {
                presentation.title.clone()
            };
            let layout = layout_topic_native(
                ui.scrolling_canvas,
                presentation,
                state.document.fonts(),
                width,
                state.text_zoom_percent,
            );
            (layout, has_fixed, title)
        };
        let fixed_height = layout.fixed.height.max(1);
        let scrolling_height = layout.scrolling.height.max(1);
        let fixed_background = auxiliary_background(&state, true);
        let scrolling_background = auxiliary_background(&state, false);
        state.layout_width = width;
        state.layout = Some(layout);
        (
            fixed_height,
            scrolling_height,
            has_fixed,
            fixed_background,
            scrolling_background,
            title,
            state.kind,
        )
    };

    ui.fixed_canvas.set_background_color(colour_from_rgb(fixed_background));
    ui.scrolled.set_background_color(colour_from_rgb(scrolling_background));
    ui.scrolling_canvas
        .set_background_color(colour_from_rgb(scrolling_background));
    ui.fixed_canvas.show(has_fixed);
    ui.fixed_canvas
        .set_min_size(Size::new(-1, if has_fixed { fixed_height } else { 0 }));
    let viewport_height = ui.scrolled.get_client_size().height.max(1);
    reset_scrolling_canvas_to_origin(
        ui.scrolled,
        ui.scrolling_canvas,
        width,
        scrolling_height.max(viewport_height),
        scrolling_height,
    );
    if kind == AuxiliaryKind::Secondary {
        let caption = state
            .borrow()
            .definition
            .as_ref()
            .and_then(|window| window.caption.as_deref())
            .filter(|caption| !caption.is_empty())
            .unwrap_or(&title)
            .to_owned();
        ui.frame.set_title(&caption);
    }
    ui.host.layout();
    ui.frame.layout();
    ui.fixed_canvas.refresh(true, None);
    ui.scrolling_canvas.refresh(true, None);
}


/// Rebuilds retained geometry, sizes both native canvases, and resets the scrolling region.
fn refresh_topic_layout(ui: &ViewerUi, state: &Rc<RefCell<ViewerState>>) {
    let width = usable_layout_width(ui.scrolled);
    let (fixed_height, scrolling_height, has_fixed, topic_number, topic_count, title, warning_count, context_name, text_zoom_percent) = {
        let mut state = state.borrow_mut();
        let Some(topic_count) = state.document.as_ref().map(|document| document.presentations().len()) else {
            return;
        };
        if topic_count == 0 {
            state.layout = None;
            ui.status_bar.set_status_text("No displayable topics were decoded", 0);
            ui.status_bar.set_status_text("0 topics", 1);
            ui.scrolling_canvas.refresh(true, None);
            return;
        }
        state.topic_index = state.topic_index.min(topic_count - 1);
        let topic_index = state.topic_index;
        let (layout, has_fixed, warning_count, title, context_name) = {
            let document = state.document.as_ref().expect("document checked above");
            let presentation = &document.presentations()[topic_index];
            let has_fixed = !presentation.non_scrolling.is_empty();
            let warning_count = presentation.warnings.len();
            let title = if presentation.title.is_empty() {
                "<untitled>".to_owned()
            } else {
                presentation.title.clone()
            };
            let context_name = document
                .topic_start_offset(topic_index)
                .and_then(|offset| document.navigation().context_name_for_offset(offset))
                .map(str::to_owned);
            let layout = layout_topic_native(
                ui.scrolling_canvas,
                presentation,
                document.fonts(),
                width,
                state.text_zoom_percent,
            );
            (layout, has_fixed, warning_count, title, context_name)
        };
        let fixed_height = layout.fixed.height.max(1);
        let scrolling_height = layout.scrolling.height.max(1);
        state.layout_width = width;
        state.layout = Some(layout);
        // Reflow can replace the retained text-box indices, so a prior selection is no longer valid.
        state.topic_selection = None;
        (
            fixed_height,
            scrolling_height,
            has_fixed,
            state.topic_index + 1,
            topic_count,
            title,
            warning_count,
            context_name,
            state.text_zoom_percent,
        )
    };

    ui.fixed_canvas.show(has_fixed);
    ui.fixed_canvas.set_min_size(Size::new(-1, if has_fixed { fixed_height } else { 0 }));

    let viewport_height = ui.scrolled.get_client_size().height.max(1);
    reset_scrolling_canvas_to_origin(
        ui.scrolled,
        ui.scrolling_canvas,
        width,
        scrolling_height.max(viewport_height),
        scrolling_height,
    );
    ui.frame.layout();
    ui.fixed_canvas.refresh(true, None);
    ui.scrolling_canvas.refresh(true, None);

    ui.status_bar.set_status_text(
        &format!(
            "{}{}",
            title,
            if warning_count == 0 {
                String::new()
            } else {
                format!("  ({warning_count} formatting warning(s))")
            }
        ),
        0,
    );
    let context_suffix = context_name.map_or_else(String::new, |value| format!("  [{value}]"));
    ui.status_bar.set_status_text(
        &format!(
            "Topic {topic_number}/{topic_count}{context_suffix}  ·  {text_zoom_percent}%"
        ),
        1,
    );
    sync_contents_selection(ui, state);
}

/// Leaves a small right-side allowance for the native vertical scrollbar before line wrapping.
fn usable_layout_width(scrolled: ScrolledWindow) -> i32 {
    scrolled
        .get_client_size()
        .width
        .saturating_sub(20)
        .max(MIN_LAYOUT_WIDTH)
}

fn units_for(pixels: i32, unit: i32) -> i32 {
    pixels.saturating_add(unit - 1) / unit.max(1)
}

/// Resets both wxScrolledWindow's logical view and the retained child panel's native position.
///
/// wxScrolledWindow implements scrolling by physically moving child windows. Rebuilding a topic
/// used to update only the child panel's size and then request logical position `(0, 0)`. After a
/// tall or image-heavy topic had been scrolled, `set_scrollbars(... y_pos: 0)` could already make
/// the helper believe it was at the origin while the child panel still retained its negative Y
/// position. The next topic was therefore painted with its title and first lines above the visible
/// client area, making the text look cropped.
///
/// Force the panel's native rectangle back to `(0, 0)` both before and after resetting the scroll
/// helper. The second placement deliberately reconciles the physical child position with the
/// newly installed logical origin even when wxWidgets considers the final `scroll_coords(0, 0)` a
/// no-op.
fn reset_scrolling_canvas_to_origin(
    scrolled: ScrolledWindow,
    canvas: Panel,
    width: i32,
    canvas_height: i32,
    scrolling_height: i32,
) {
    scrolled.scroll_coords(0, 0);
    canvas.set_size_with_pos(0, 0, width, canvas_height);
    scrolled.set_scrollbars(ScrollBarConfig {
        pixels_per_unit_x: SCROLL_UNIT,
        pixels_per_unit_y: SCROLL_UNIT,
        no_units_x: 0,
        no_units_y: units_for(scrolling_height, SCROLL_UNIT),
        x_pos: 0,
        y_pos: 0,
        no_refresh: false,
    });
    scrolled.scroll_coords(0, 0);
    canvas.set_size_with_pos(0, 0, width, canvas_height);
}

/// Paints the milestone text shown before a document has been opened.
fn paint_welcome(dc: &PaintDC) {
    let font = Font::new_with_details(
        10,
        FontFamily::Swiss.as_i32(),
        FontStyle::Normal.as_i32(),
        FontWeight::Normal.as_i32(),
        false,
        if cfg!(target_os = "windows") { "Segoe UI" } else { "" },
    )
    .unwrap_or_else(Font::new);
    dc.set_font(&font);
    dc.set_text_foreground(wxdragon::color::colours::BLACK);
    let mut y = 18;
    for line in WELCOME_TEXT.lines() {
        dc.draw_text(line, 18, y);
        y += 20;
    }
}

/// Paints retained boxes using native wxDragon device-context primitives.
fn paint_region(
    canvas: Panel,
    dc: &PaintDC,
    region: &RegionLayout,
    text_zoom_percent: i32,
    default_background: Rgb,
) {
    let native_text = NativeTextContext::new(canvas);
    dc.set_brush(wxdragon::color::colours::WHITE, BrushStyle::Transparent);

    // Pass 1 paints everything that reaches the surface through the wx PaintDC. Text is held back
    // because it is emitted through a separate GDI device context (see WindowsTextDc): a bitmap
    // blit issued later in box order would otherwise erase glyphs that were already on the pixels
    // it covers, which is why text next to a picture disappeared.
    for item in &region.boxes {
        match &item.kind {
            LayoutKind::Text { .. } | LayoutKind::PictureHotspot { .. } => {}
            LayoutKind::Picture { image } => paint_picture(dc, item, image),
            LayoutKind::PicturePlaceholder => paint_picture_placeholder(dc, item),
            LayoutKind::EmbeddedWindowPlaceholder {
                descriptor,
                standard_button_label,
                ..
            } => {
                paint_embedded_window_placeholder(
                    dc,
                    item,
                    descriptor,
                    standard_button_label.as_deref(),
                );
            }
            LayoutKind::Border { flags, style } => {
                paint_border(dc, item, *flags, *style);
            }
        }
    }

    // Pass 2 paints the retained text runs on top, in authored order.
    for item in &region.boxes {
        if let LayoutKind::Text { text, style, .. } = &item.kind {
            paint_text(
                dc,
                &native_text,
                item,
                text,
                style,
                text_zoom_percent,
                default_background,
            );
        }
    }
}

/// Paints a retained region and overlays the active topic text selection.
fn paint_region_with_selection(
    canvas: Panel,
    dc: &PaintDC,
    region: &RegionLayout,
    text_zoom_percent: i32,
    default_background: Rgb,
    selection: Option<TopicTextSelection>,
) {
    let native_text = NativeTextContext::new(canvas);
    dc.set_brush(wxdragon::color::colours::WHITE, BrushStyle::Transparent);
    let ordered_selection = selection.map(TopicTextSelection::ordered);

    // Pass 1: PaintDC primitives only. See paint_region for why text is deferred.
    for item in &region.boxes {
        match &item.kind {
            LayoutKind::Text { .. } | LayoutKind::PictureHotspot { .. } => {}
            LayoutKind::Picture { image } => paint_picture(dc, item, image),
            LayoutKind::PicturePlaceholder => paint_picture_placeholder(dc, item),
            LayoutKind::EmbeddedWindowPlaceholder { descriptor, standard_button_label, .. } => {
                paint_embedded_window_placeholder(dc, item, descriptor, standard_button_label.as_deref());
            }
            LayoutKind::Border { flags, style } => paint_border(dc, item, *flags, *style),
        }
    }

    // Pass 2: retained text plus the selection overlay.
    for (box_index, item) in region.boxes.iter().enumerate() {
        let LayoutKind::Text { text, style, .. } = &item.kind else {
            continue;
        };
        paint_text(
            dc,
            &native_text,
            item,
            text,
            style,
            text_zoom_percent,
            default_background,
        );
        let Some((start, end)) = ordered_selection else {
            continue;
        };
        let Some((from, to)) = selection_byte_range_for_box(text.len(), box_index, start, end) else {
            continue;
        };
        if !text.is_char_boundary(from) || !text.is_char_boundary(to) {
            continue;
        }
        let prefix_width = measure_text_width(
            canvas,
            &native_text,
            style,
            &text[..from],
            text_zoom_percent,
        );
        let selected_width = measure_text_width(
            canvas,
            &native_text,
            style,
            &text[from..to],
            text_zoom_percent,
        );
        if selected_width <= 0 {
            continue;
        }
        let selection_x = item.bounds.x.saturating_add(prefix_width);
        dc.set_pen(colour_from_rgb(TEXT_SELECTION_BACKGROUND), 1, PenStyle::Solid);
        dc.set_brush(colour_from_rgb(TEXT_SELECTION_BACKGROUND), BrushStyle::Solid);
        dc.draw_rectangle(selection_x, item.bounds.y, selected_width, item.bounds.height);

        if !native_text.paint(
            style,
            &text[from..to],
            text_zoom_percent,
            TEXT_SELECTION_FOREGROUND,
            selection_x,
            item.bounds.y,
        ) {
            let font = make_native_font(style, text_zoom_percent);
            dc.set_font(&font);
            dc.set_text_foreground(colour_from_rgb(TEXT_SELECTION_FOREGROUND));
            dc.set_background_mode(wxdragon::dc::BackgroundMode::Transparent);
            dc.draw_text(&text[from..to], selection_x, item.bounds.y);
        }
    }
}

/// Converts a GUI-independent RGBA image to a native wxBitmap and paints it at layout size.
/// Oversized images are proportionally reduced by the layout engine, so resize only when needed.
fn paint_picture(dc: &PaintDC, item: &LayoutBox, image: &hlp::DecodedPicture) {
    let target_width = u32::try_from(item.bounds.width).unwrap_or(0);
    let target_height = u32::try_from(item.bounds.height).unwrap_or(0);
    if target_width == 0 || target_height == 0 {
        return;
    }

    let scaled = if target_width == image.width && target_height == image.height {
        None
    } else {
        Some(scale_rgba_nearest(
            image.rgba.as_ref(),
            image.width,
            image.height,
            target_width,
            target_height,
        ))
    };
    let pixels = scaled.as_deref().unwrap_or(image.rgba.as_ref());

    if let Some(bitmap) = Bitmap::from_rgba(pixels, target_width, target_height) {
        dc.draw_bitmap(&bitmap, item.bounds.x, item.bounds.y, image.has_alpha);
    } else {
        paint_picture_placeholder(dc, item);
    }
}

/// Small dependency-free nearest-neighbour resize used only when an authored image is wider than
/// the topic viewport. Native-size images take the zero-copy branch in `paint_picture`.
fn scale_rgba_nearest(
    source: &[u8],
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
) -> Vec<u8> {
    let target_len = usize::try_from(target_width)
        .ok()
        .and_then(|width| {
            usize::try_from(target_height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .unwrap_or(0);
    let mut output = vec![0_u8; target_len];
    if output.is_empty() || source_width == 0 || source_height == 0 {
        return output;
    }

    let source_width_usize = usize::try_from(source_width).unwrap_or(0);
    let target_width_usize = usize::try_from(target_width).unwrap_or(0);
    for target_y in 0..target_height {
        let source_y = u64::from(target_y) * u64::from(source_height) / u64::from(target_height);
        for target_x in 0..target_width {
            let source_x = u64::from(target_x) * u64::from(source_width) / u64::from(target_width);
            let source_index = (usize::try_from(source_y).unwrap_or(0) * source_width_usize
                + usize::try_from(source_x).unwrap_or(0))
                * 4;
            let target_index = (usize::try_from(target_y).unwrap_or(0) * target_width_usize
                + usize::try_from(target_x).unwrap_or(0))
                * 4;
            if let (Some(source_pixel), Some(target_pixel)) = (
                source.get(source_index..source_index + 4),
                output.get_mut(target_index..target_index + 4),
            ) {
                target_pixel.copy_from_slice(source_pixel);
            }
        }
    }
    output
}

fn paint_text(
    dc: &PaintDC,
    native_text: &NativeTextContext,
    item: &LayoutBox,
    text: &str,
    style: &ResolvedTextStyle,
    text_zoom_percent: i32,
    default_background: Rgb,
) {
    // WinHlp32 uses opaque text output and treats exact RGB(1,1,0) descriptor colours as
    // "inherit current". Keep the viewer's deterministic black-on-authored-page defaults,
    // while preserving every other HLP foreground/background verbatim.
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

    // WinHlp32 emits text opaquely, but reproducing that with one opaque DrawText per run is
    // destructive here: anti-aliased glyph edges bleed roughly a pixel past their character
    // cell, so every run's opaque cell erased the right edge of the run before it. That is what
    // shaved the glyphs and made the anti-aliasing look broken. Painting the fill only when the
    // descriptor actually asks for a different background keeps every authored highlight while
    // leaving neighbouring glyph edges intact - filling with the colour already on the page was
    // a no-op that cost fidelity.
    if background != default_background {
        dc.set_pen(colour_from_rgb(background), 1, PenStyle::Solid);
        dc.set_brush(colour_from_rgb(background), BrushStyle::Solid);
        dc.draw_rectangle(
            item.bounds.x,
            item.bounds.y,
            item.bounds.width,
            item.bounds.height,
        );
    }

    // On Windows, paint with the same LOGFONT/GDI path used for measurement so retained HC30
    // half-point sizes survive all the way to the native device. wxDragon currently exposes only
    // integer point sizes; its path remains the safe portable/failure fallback.
    if native_text.paint(
        style,
        text,
        text_zoom_percent,
        foreground,
        item.bounds.x,
        item.bounds.y,
    ) {
        return;
    }

    let font = make_native_font(style, text_zoom_percent);
    dc.set_font(&font);
    dc.set_text_foreground(colour_from_rgb(foreground));
    dc.set_text_background(colour_from_rgb(background));
    dc.set_background_mode(wxdragon::dc::BackgroundMode::Transparent);
    dc.draw_text(text, item.bounds.x, item.bounds.y);
}

/// Native text backend used by both retained measurement and painting.
///
/// On Windows this owns a GDI device context for the actual canvas HWND, retaining the device's
/// horizontal/vertical DPI and accepting a pixel-height LOGFONT. Other platforms intentionally
/// keep the existing wxWidgets path until an equivalent fractional-point API is exposed there.
struct NativeTextContext {
    #[cfg(target_os = "windows")]
    windows: Option<WindowsTextDc>,
}

impl NativeTextContext {
    fn new(canvas: Panel) -> Self {
        #[cfg(not(target_os = "windows"))]
        let _ = canvas;
        Self {
            #[cfg(target_os = "windows")]
            windows: WindowsTextDc::new(canvas),
        }
    }

    fn dpi(&self) -> (i32, i32) {
        #[cfg(target_os = "windows")]
        if let Some(windows) = &self.windows {
            return (windows.dpi_x, windows.dpi_y);
        }
        (96, 96)
    }

    fn measure(
        &self,
        style: &ResolvedTextStyle,
        text: &str,
        text_zoom_percent: i32,
    ) -> Option<TextMetrics> {
        #[cfg(target_os = "windows")]
        if let Some(windows) = &self.windows {
            return windows.measure(style, text, text_zoom_percent);
        }
        #[cfg(not(target_os = "windows"))]
        let _ = (style, text, text_zoom_percent);
        None
    }

    fn paint(
        &self,
        style: &ResolvedTextStyle,
        text: &str,
        text_zoom_percent: i32,
        foreground: Rgb,
        x: i32,
        y: i32,
    ) -> bool {
        #[cfg(target_os = "windows")]
        if let Some(windows) = &self.windows {
            return windows.paint(style, text, text_zoom_percent, foreground, x, y);
        }
        #[cfg(not(target_os = "windows"))]
        let _ = (style, text, text_zoom_percent, foreground, x, y);
        false
    }
}

#[cfg(target_os = "windows")]
struct WindowsTextDc {
    hwnd: HWND,
    hdc: HDC,
    dpi_x: i32,
    dpi_y: i32,
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
impl WindowsTextDc {
    fn new(canvas: Panel) -> Option<Self> {
        let hwnd = canvas.get_handle() as HWND;
        if hwnd.is_null() {
            return None;
        }
        let hdc = unsafe { GetDC(hwnd) };
        if hdc.is_null() {
            return None;
        }
        let dpi_x = unsafe { GetDeviceCaps(hdc, LOGPIXELSX as i32) }.max(1);
        let dpi_y = unsafe { GetDeviceCaps(hdc, LOGPIXELSY as i32) }.max(1);
        Some(Self {
            hwnd,
            hdc,
            dpi_x,
            dpi_y,
        })
    }

    fn create_font(
        &self,
        style: &ResolvedTextStyle,
        text_zoom_percent: i32,
    ) -> Option<*mut core::ffi::c_void> {
        create_gdi_font_for_style(style, self.dpi_y, text_zoom_percent)
    }

    fn measure(
        &self,
        style: &ResolvedTextStyle,
        text: &str,
        text_zoom_percent: i32,
    ) -> Option<TextMetrics> {
        let font = self.create_font(style, text_zoom_percent)?;
        let old_font = unsafe { SelectObject(self.hdc, font) };
        if old_font.is_null() {
            unsafe { DeleteObject(font) };
            return None;
        }

        let wide: Vec<u16> = text.encode_utf16().collect();
        let count = i32::try_from(wide.len()).ok();
        let mut size = SIZE::default();
        let mut metrics = TEXTMETRICW::default();
        let extent_ok = match count {
            Some(0) => true,
            Some(count) => unsafe {
                GetTextExtentPoint32W(self.hdc, wide.as_ptr(), count, &mut size) != 0
            },
            None => false,
        };
        let metrics_ok = unsafe { GetTextMetricsW(self.hdc, &mut metrics) != 0 };

        unsafe {
            SelectObject(self.hdc, old_font);
            DeleteObject(font);
        }

        if !extent_ok || !metrics_ok {
            return None;
        }
        let height = metrics
            .tmHeight
            .saturating_add(metrics.tmExternalLeading.max(0))
            .max(1);
        Some(TextMetrics {
            width: size.cx.max(0),
            height,
            baseline: metrics.tmAscent.clamp(1, height),
        })
    }

    fn paint(
        &self,
        style: &ResolvedTextStyle,
        text: &str,
        text_zoom_percent: i32,
        foreground: Rgb,
        x: i32,
        y: i32,
    ) -> bool {
        if text.is_empty() {
            return true;
        }
        let wide: Vec<u16> = text.encode_utf16().collect();
        let Ok(count) = i32::try_from(wide.len()) else {
            return false;
        };
        let Some(font) = self.create_font(style, text_zoom_percent) else {
            return false;
        };
        let old_font = unsafe { SelectObject(self.hdc, font) };
        if old_font.is_null() {
            unsafe { DeleteObject(font) };
            return false;
        }

        let old_background_mode = unsafe { SetBkMode(self.hdc, TRANSPARENT as i32) };
        let old_text_color = unsafe { SetTextColor(self.hdc, colorref_from_rgb(foreground)) };
        let painted = unsafe { TextOutW(self.hdc, x, y, wide.as_ptr(), count) != 0 };
        if old_background_mode != 0 {
            unsafe { SetBkMode(self.hdc, old_background_mode) };
        }
        if old_text_color != u32::MAX {
            unsafe { SetTextColor(self.hdc, old_text_color) };
        }
        unsafe {
            SelectObject(self.hdc, old_font);
            DeleteObject(font);
        }
        painted
    }
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
impl Drop for WindowsTextDc {
    fn drop(&mut self) {
        unsafe {
            ReleaseDC(self.hwnd, self.hdc);
        }
    }
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn create_gdi_font_for_style(
    style: &ResolvedTextStyle,
    dpi_y: i32,
    text_zoom_percent: i32,
) -> Option<*mut std::ffi::c_void> {
    let mut logical_font = LOGFONTW::default();
    let scaled_twips = zoomed_font_twips(
        effective_authored_font_twips(style),
        text_zoom_percent,
    );
    logical_font.lfHeight = -font_pixel_height_from_twips(scaled_twips, dpi_y);
    logical_font.lfWeight = i32::from(style.weight).clamp(0, 1000);
    logical_font.lfItalic = u8::from(style.italic);
    logical_font.lfUnderline = u8::from(style.underline);
    logical_font.lfStrikeOut = u8::from(style.strike_out);
    logical_font.lfCharSet = style.charset.unwrap_or(DEFAULT_CHARSET as u8);

    let (_, preferred_face) = native_font_policy(style);
    let max_face_units = logical_font.lfFaceName.len().saturating_sub(1);
    for (target, source) in logical_font
        .lfFaceName
        .iter_mut()
        .take(max_face_units)
        .zip(preferred_face.encode_utf16())
    {
        *target = source;
    }

    let font = unsafe { CreateFontIndirectW(&logical_font) };
    (!font.is_null()).then_some(font)
}

#[cfg(target_os = "windows")]
fn colorref_from_rgb(value: Rgb) -> u32 {
    u32::from(value.red) | (u32::from(value.green) << 8) | (u32::from(value.blue) << 16)
}

/// Widens the pending WM_PAINT update region to the whole client area before `BeginPaint` runs.
///
/// Retained text is emitted through a private `GetDC` handle (`WindowsTextDc`) so that HC30
/// half-point sizes survive as a pixel-height LOGFONT. That handle is clipped to the window's
/// *visible* region, whereas the `PaintDC` returned by `BeginPaint` is clipped to the *update*
/// region. The two agree on a full repaint, and disagree after a scroll: `wxScrolledWindow` moves
/// the retained child window, so Windows blits the existing pixels and invalidates only the newly
/// exposed strip. The background clear, the picture blits and the selection fills were then
/// confined to that strip while every glyph in the topic was re-emitted across the whole canvas,
/// on top of glyphs that had never been erased.
///
/// Redrawing anti-aliased text over itself in TRANSPARENT mode drives the partially covered edge
/// pixels toward the solid glyph colour on each pass. After a few scroll steps the edges saturate,
/// which is exactly the reported "anti-aliasing is off" appearance, and the surviving fragments of
/// the previous, un-erased copy read as clipped or missing glyphs.
///
/// Invalidating the whole client area first keeps both device contexts covering the same pixels,
/// so every scroll step draws over a freshly cleared background. `FALSE` for `bErase` is correct
/// here: the canvas uses `BackgroundStyle::Paint` and clears itself through the `PaintDC`.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn invalidate_whole_canvas(canvas: Panel) {
    let hwnd = canvas.get_handle() as HWND;
    if hwnd.is_null() {
        return;
    }
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

#[cfg(not(target_os = "windows"))]
fn invalidate_whole_canvas(canvas: Panel) {
    let _ = canvas;
}

/// Returns the authored font height in twentieths of a point after WinHlp32 small-caps scaling.
fn effective_authored_font_twips(style: &ResolvedTextStyle) -> i32 {
    if style.small_caps {
        // KB917607 WinHlp32 0x411a59..0x411a6c scales HC30 small-caps lfHeight to 2/3.
        style.point_size_twips.saturating_mul(2) / 3
    } else {
        style.point_size_twips
    }
}

/// Applies viewer zoom without discarding the HLP's retained half-point precision.
fn zoomed_font_twips(point_size_twips: i32, text_zoom_percent: i32) -> i32 {
    let zoom = i64::from(
        text_zoom_percent.clamp(MIN_TEXT_ZOOM_PERCENT, MAX_TEXT_ZOOM_PERCENT),
    );
    let authored_twips = i64::from(point_size_twips).abs().max(20);
    i32::try_from(
        authored_twips
            .saturating_mul(zoom)
            .saturating_add(50)
            / 100,
    )
    .unwrap_or(i32::MAX)
    .clamp(20, 81_920)
}

/// Converts a retained twip size to a negative-LOGFONT-compatible positive pixel magnitude.
fn font_pixel_height_from_twips(point_size_twips: i32, dpi_y: i32) -> i32 {
    let numerator = i64::from(point_size_twips.abs().max(20))
        .saturating_mul(i64::from(dpi_y.max(1)));
    i32::try_from(numerator.saturating_add(720) / 1_440)
        .unwrap_or(i32::MAX)
        .max(1)
}

/// Builds exactly the portable fallback font used for both wxWidgets measurement and painting.
fn make_native_font(style: &ResolvedTextStyle, text_zoom_percent: i32) -> Font {
    let font_style = if style.italic {
        FontStyle::Italic
    } else {
        FontStyle::Normal
    };
    let font_weight = weight_for(style.weight);
    let (font_family, preferred_face) = native_font_policy(style);
    let authored_twips = effective_authored_font_twips(style);
    let point_size = zoomed_point_size_from_twips(authored_twips, text_zoom_percent);
    let mut font = Font::new_with_details(
        point_size,
        font_family.as_i32(),
        font_style.as_i32(),
        font_weight.as_i32(),
        style.underline,
        preferred_face,
    )
    .or_else(|| {
        // Empty face name asks wxWidgets/the native font mapper for the closest platform face.
        Font::new_with_details(
            point_size,
            font_family.as_i32(),
            font_style.as_i32(),
            font_weight.as_i32(),
            style.underline,
            "",
        )
    })
    .unwrap_or_else(Font::new);
    font.set_strikethrough(style.strike_out);
    font
}

/// Portable fallback text measurement used when the Windows GDI backend is unavailable.
fn wx_text_metrics(
    canvas: Panel,
    style: &ResolvedTextStyle,
    text: &str,
    text_zoom_percent: i32,
) -> TextMetrics {
    let font = make_native_font(style, text_zoom_percent);
    let (size, descent, external_leading) = canvas.get_full_text_extent(text, Some(&font));
    let height = size.height.saturating_add(external_leading.max(0)).max(1);
    // wxWidgets exposes the font descent separately from the text-cell height. Retain the
    // resulting baseline instead of throwing it away: different faces can report the same cell
    // height while using different ascents, which is exactly the CALC.HLP list-marker case.
    let baseline = size.height.saturating_sub(descent.max(0)).clamp(1, height);
    TextMetrics {
        width: size.width.max(0),
        height,
        baseline,
    }
}

/// Runs retained layout with the same native backend used for painting.
fn layout_topic_native(
    canvas: Panel,
    presentation: &hlp::TopicPresentation,
    fonts: &hlp::FontTable,
    width: i32,
    text_zoom_percent: i32,
) -> TopicLayout {
    let native_text = NativeTextContext::new(canvas);
    let (dpi_x, dpi_y) = native_text.dpi();
    let mut measure = |style: &ResolvedTextStyle, text: &str| {
        native_text
            .measure(style, text, text_zoom_percent)
            .unwrap_or_else(|| wx_text_metrics(canvas, style, text, text_zoom_percent))
    };
    LayoutEngine::with_dpi_and_text_zoom(dpi_x, dpi_y, text_zoom_percent)
        .layout_topic_with_measurer(presentation, fonts, width, &mut measure)
}

/// Rounds to the integer point size accepted by wxWidgets only on the portable fallback path.
fn zoomed_point_size_from_twips(point_size_twips: i32, text_zoom_percent: i32) -> i32 {
    let scaled_twips = i64::from(zoomed_font_twips(point_size_twips, text_zoom_percent));
    i32::try_from(scaled_twips.saturating_add(10) / 20)
        .unwrap_or(i32::MAX)
        .clamp(1, 4096)
}

/// Converts the retained WinHelp family into wxWidgets' closest coarse native family.
#[cfg(target_os = "windows")]
fn wx_family_for_hlp_family(family: hlp::HlpFontFamily) -> FontFamily {
    match family {
        hlp::HlpFontFamily::Roman => FontFamily::Roman,
        hlp::HlpFontFamily::Swiss => FontFamily::Swiss,
        hlp::HlpFontFamily::Script => FontFamily::Script,
        hlp::HlpFontFamily::Decorative => FontFamily::Decorative,
        hlp::HlpFontFamily::Modern => FontFamily::Modern,
        hlp::HlpFontFamily::Unknown(_) => FontFamily::Default,
    }
}

/// Chooses modern native faces without discarding fixed-pitch intent or symbol-font semantics.
#[cfg(target_os = "windows")]
fn native_font_policy(style: &ResolvedTextStyle) -> (FontFamily, &str) {
    if is_semantic_symbol_face(&style.face_name) {
        return (FontFamily::Decorative, &style.face_name);
    }
    // KB917607 does not substitute a modern UI face before shaping legacy international text.
    // It passes the authored face plus GDI charset into CreateFontIndirect and lets the Windows
    // font mapper select the concrete font. Preserve that path for every non-ANSI/default legacy
    // charset so DBCS, Johab, Hebrew, Arabic, Greek, Cyrillic, Thai, etc. reach GDI with the same
    // face/charset pair. The viewer's modern-face policy remains in effect for ordinary western
    // ANSI/default runs, where it was an intentional readability choice.
    if style.charset.is_some_and(|charset| !matches!(charset, 0x00 | 0x01)) {
        return (wx_family_for_hlp_family(style.source_family), &style.face_name);
    }
    if let Some((family, replacement)) = legacy_raster_face_replacement(&style.face_name) {
        return (family, replacement);
    }
    if style.family == ResolvedFontFamily::Monospace {
        return (FontFamily::Modern, "Consolas");
    }
    match style.source_family {
        hlp::HlpFontFamily::Roman => (FontFamily::Roman, "Times New Roman"),
        hlp::HlpFontFamily::Swiss => (FontFamily::Swiss, "Segoe UI"),
        hlp::HlpFontFamily::Script => (FontFamily::Script, "Segoe Script"),
        hlp::HlpFontFamily::Decorative => (FontFamily::Decorative, &style.face_name),
        hlp::HlpFontFamily::Modern => (FontFamily::Modern, "Consolas"),
        hlp::HlpFontFamily::Unknown(_) => (FontFamily::Default, "Segoe UI"),
    }
}

#[cfg(not(target_os = "windows"))]
fn native_font_policy(style: &ResolvedTextStyle) -> (FontFamily, &str) {
    if is_semantic_symbol_face(&style.face_name) {
        return (FontFamily::Decorative, &style.face_name);
    }
    if style.family == ResolvedFontFamily::Monospace {
        return (FontFamily::Modern, "");
    }
    match style.source_family {
        hlp::HlpFontFamily::Roman => (FontFamily::Roman, ""),
        hlp::HlpFontFamily::Swiss => (FontFamily::Swiss, ""),
        hlp::HlpFontFamily::Script => (FontFamily::Script, ""),
        hlp::HlpFontFamily::Decorative => (FontFamily::Decorative, ""),
        hlp::HlpFontFamily::Modern => (FontFamily::Modern, ""),
        hlp::HlpFontFamily::Unknown(_) => (FontFamily::Default, ""),
    }
}

/// Maps the legacy raster faces classic HLP files request onto modern outline equivalents.
///
/// `Helv`, `MS Sans Serif`, `System`, `Small Fonts`, `Courier` and `Terminal` are bitmap-strike
/// fonts. GDI cannot anti-alias a raster strike, and when the requested size has no strike it
/// scales one by pixel replication, which is exactly the shaved, crunchy text this viewer showed.
/// Outline faces an author chose deliberately - Arial, Times New Roman, Verdana, Tahoma, Courier
/// New - are deliberately left alone so authored typography survives.
#[cfg(target_os = "windows")]
fn legacy_raster_face_replacement(face_name: &str) -> Option<(FontFamily, &'static str)> {
    match face_name.trim().to_ascii_lowercase().as_str() {
        "helv" | "helvetica" | "ms sans serif" | "sans serif" | "system" | "small fonts"
        | "ms shell dlg" | "ms shell dlg 2" => Some((FontFamily::Swiss, "Segoe UI")),
        "tms rmn" | "ms serif" | "serif" | "roman" | "times" => {
            Some((FontFamily::Roman, "Times New Roman"))
        }
        "courier" | "terminal" | "fixedsys" | "modern" => Some((FontFamily::Modern, "Consolas")),
        _ => None,
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

fn paint_picture_placeholder(dc: &PaintDC, item: &LayoutBox) {
    dc.set_pen(wxdragon::color::colours::GRAY, 1, PenStyle::Solid);
    dc.set_brush(wxdragon::color::colours::WHITE, BrushStyle::Transparent);
    dc.draw_rectangle(item.bounds.x, item.bounds.y, item.bounds.width, item.bounds.height);
    let font = Font::new_with_details(
        8,
        FontFamily::Swiss.as_i32(),
        FontStyle::Normal.as_i32(),
        FontWeight::Normal.as_i32(),
        false,
        if cfg!(target_os = "windows") { "Segoe UI" } else { "" },
    )
    .unwrap_or_else(Font::new);
    dc.set_font(&font);
    dc.set_text_foreground(wxdragon::color::colours::GRAY);
    dc.draw_text("[embedded picture]", item.bounds.x + 6, item.bounds.y + 6);
}

fn paint_embedded_window_placeholder(
    dc: &PaintDC,
    item: &LayoutBox,
    descriptor: &str,
    standard_button_label: Option<&str>,
) {
    if let Some(label) = standard_button_label {
        // KB917607 factory 0x4240F4 creates the leading-`!` descriptor form with the stock
        // BUTTON class. Paint the classic raised face but deliberately do not attach/execute
        // the authored macro stored after the comma.
        let face = colour_from_rgb(Rgb { red: 192, green: 192, blue: 192 });
        let shadow = colour_from_rgb(Rgb { red: 128, green: 128, blue: 128 });
        dc.set_pen(face, 1, PenStyle::Solid);
        dc.set_brush(face, BrushStyle::Solid);
        dc.draw_rectangle(item.bounds.x, item.bounds.y, item.bounds.width, item.bounds.height);

        let left = item.bounds.x;
        let top = item.bounds.y;
        let right = left.saturating_add(item.bounds.width.saturating_sub(1));
        let bottom = top.saturating_add(item.bounds.height.saturating_sub(1));
        dc.set_pen(wxdragon::color::colours::WHITE, 1, PenStyle::Solid);
        dc.draw_line(left, bottom, left, top);
        dc.draw_line(left, top, right, top);
        dc.set_pen(shadow, 1, PenStyle::Solid);
        dc.draw_line(right, top, right, bottom);
        dc.draw_line(right, bottom, left, bottom);

        if !label.is_empty() {
            let font = Font::new_with_details(
                8,
                FontFamily::Swiss.as_i32(),
                FontStyle::Normal.as_i32(),
                FontWeight::Normal.as_i32(),
                false,
                if cfg!(target_os = "windows") { "Segoe UI" } else { "" },
            )
            .unwrap_or_else(Font::new);
            dc.set_font(&font);
            dc.set_text_foreground(wxdragon::color::colours::BLACK);
            dc.draw_text(label, item.bounds.x + 4, item.bounds.y + 1);
        }
        return;
    }

    dc.set_pen(wxdragon::color::colours::GRAY, 1, PenStyle::Solid);
    dc.set_brush(wxdragon::color::colours::WHITE, BrushStyle::Transparent);
    dc.draw_rectangle(item.bounds.x, item.bounds.y, item.bounds.width, item.bounds.height);
    let font = Font::new_with_details(
        8,
        FontFamily::Swiss.as_i32(),
        FontStyle::Normal.as_i32(),
        FontWeight::Normal.as_i32(),
        false,
        if cfg!(target_os = "windows") { "Segoe UI" } else { "" },
    )
    .unwrap_or_else(Font::new);
    dc.set_font(&font);
    dc.set_text_foreground(wxdragon::color::colours::GRAY);
    let label = if descriptor.trim().is_empty() {
        "[embedded WinHelp control]".to_owned()
    } else {
        let mut text = descriptor.trim().replace('\r', " ").replace('\n', " ");
        if text.chars().count() > 48 {
            text = text.chars().take(45).collect::<String>() + "...";
        }
        format!("[embedded WinHelp control: {text}]")
    };
    dc.draw_text(&label, item.bounds.x + 6, item.bounds.y + 6);
}

fn paint_border(
    dc: &PaintDC,
    item: &LayoutBox,
    flags: hlp::BorderFlags,
    style: hlp::BorderStyle,
) {
    // Styles 5-7 have zero clearance and no defined style setup in the verified Microsoft
    // switch. Do not invent a normal one-pixel border for these reserved values.
    if matches!(style, hlp::BorderStyle::Reserved(_)) {
        return;
    }
    // KB917607 WinHlp32 treats the high three border bits as one style code. Style 1
    // is thicker, style 2 adds a second edge two pixels inward, and style 3 adds the
    // classic bottom/right shadow. Style 4 follows normal geometry in the verified path.
    let width = if matches!(style, hlp::BorderStyle::Thick) { 2 } else { 1 };
    dc.set_pen(wxdragon::color::colours::BLACK, width, PenStyle::Solid);
    let compact_horizontal_separator = !flags.box_all
        && flags.top
        && flags.bottom
        && !flags.left
        && !flags.right
        && item.bounds.height <= 16;
    if compact_horizontal_separator {
        dc.draw_line(
            item.bounds.x,
            item.bounds.y,
            item.bounds.x.saturating_add(item.bounds.width),
            item.bounds.y,
        );
        return;
    }
    draw_border_edges(dc, item, flags, 0);

    match style {
        hlp::BorderStyle::Double if item.bounds.width > 4 && item.bounds.height > 4 => {
            draw_border_edges(dc, item, flags, 2);
        }
        hlp::BorderStyle::Shadow => {
            draw_border_shadow(dc, item, flags);
        }
        _ => {}
    }
}

fn draw_border_shadow(dc: &PaintDC, item: &LayoutBox, flags: hlp::BorderFlags) {
    let left = item.bounds.x;
    let top = item.bounds.y;
    let right = item.bounds.x.saturating_add(item.bounds.width);
    let bottom = item.bounds.y.saturating_add(item.bounds.height);
    let all = flags.box_all;
    // The reference style-3 box path at 0x41553d..0x41556b adds an offset
    // bottom/right pair rather than a second complete rectangle.
    if all || flags.bottom {
        dc.draw_line(
            left.saturating_add(1),
            bottom.saturating_add(1),
            right.saturating_add(1),
            bottom.saturating_add(1),
        );
    }
    if all || flags.right {
        dc.draw_line(
            right.saturating_add(1),
            top.saturating_add(1),
            right.saturating_add(1),
            bottom.saturating_add(1),
        );
    }
}

fn draw_border_edges(dc: &PaintDC, item: &LayoutBox, flags: hlp::BorderFlags, inset: i32) {
    let left = item.bounds.x.saturating_add(inset);
    let top = item.bounds.y.saturating_add(inset);
    let right = item.bounds.x.saturating_add(item.bounds.width).saturating_sub(inset);
    let bottom = item.bounds.y.saturating_add(item.bounds.height).saturating_sub(inset);
    let all = flags.box_all;
    if all || flags.top {
        dc.draw_line(left, top, right, top);
    }
    if all || flags.left {
        dc.draw_line(left, top, left, bottom);
    }
    if all || flags.bottom {
        dc.draw_line(left, bottom, right, bottom);
    }
    if all || flags.right {
        dc.draw_line(right, top, right, bottom);
    }
}

fn colour_from_rgb(value: Rgb) -> wxdragon::color::Colour {
    wxdragon::color::Colour::rgb(value.red, value.green, value.blue)
}

fn weight_for(weight: i16) -> FontWeight {
    match weight {
        ..=300 => FontWeight::Light,
        301..=549 => FontWeight::Normal,
        550..=649 => FontWeight::SemiBold,
        _ => FontWeight::Bold,
    }
}

/// Produces a safe status-line description instead of executing macro or external-link behavior.
fn describe_hotspot(hotspot: &Hotspot) -> String {
    match &hotspot.target {
        HotspotTarget::Internal { offset, popup } => {
            let kind = if *popup { "Popup" } else { "Topic" };
            format!("{kind} link: TOPICOFFSET {}", offset.0)
        }
        HotspotTarget::ContextHash { hash, popup } => {
            let kind = if *popup { "Popup" } else { "Topic" };
            format!("{kind} link: context hash 0x{:08X}", *hash as u32)
        }
        HotspotTarget::External {
            offset,
            help_file,
            window_name,
            ..
        } => {
            let file = help_file.as_deref().unwrap_or("current help file");
            let window = window_name
                .as_deref()
                .map(|value| format!(", window {value}"))
                .unwrap_or_default();
            format!("External link: {file}, TOPICOFFSET {}{window}", offset.0)
        }
        HotspotTarget::Macro(text) => format!("WinHelp macro hotspot: {text}"),
    }
}

/// Displays an informational dialog describing the current compatibility milestone.
fn show_about(frame: &Frame) {
    let message = concat!(
        "Rust HLP Viewer 0.7.1\n\n",
        "A native Rust viewer for classic Microsoft Windows HLP files.\n",
        "GUI: wxDragon / wxWidgets"
    );
    MessageDialog::builder(frame, message, "About Rust HLP Viewer")
        .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
        .build()
        .show_modal();
}


#[cfg(test)]
mod tooltip_tests {
    use super::{format_destination_tooltip, popup_topic_tooltip_body};

    #[test]
    fn ordinary_hover_uses_destination_title() {
        assert_eq!(
            format_destination_tooltip(
                "Keyboard equivalents",
                Some("This body belongs to the destination topic."),
                false,
            ),
            "Keyboard equivalents"
        );
    }

    #[test]
    fn popup_hover_uses_actual_popup_topic_body() {
        assert_eq!(
            format_destination_tooltip(
                "Topic 6",
                Some("  Decimal: base ten.\r\nHexadecimal: base sixteen.  "),
                true,
            ),
            "Decimal: base ten.\nHexadecimal: base sixteen."
        );
    }


    #[test]
    fn popup_body_is_taken_from_rendered_presentation_text() {
        let presentation = hlp::TopicPresentation {
            id: hlp::TopicId(hlp::TopicPos(1)),
            title: String::new(),
            non_scrolling: vec![hlp::FormattedRecord::from_plain_text("Numeral systems")],
            scrolling: vec![hlp::FormattedRecord::from_plain_text(
                "Decimal uses base ten.",
            )],
            warnings: Vec::new(),
        };

        assert_eq!(
            popup_topic_tooltip_body(&presentation).as_deref(),
            Some("Numeral systems\nDecimal uses base ten.")
        );
    }

    #[test]
    fn empty_popup_body_falls_back_to_destination_title() {
        assert_eq!(
            format_destination_tooltip("Popup glossary", Some("  \r\n  "), true),
            "Popup glossary"
        );
    }
}

#[cfg(test)]
mod native_sizing_tests {
    use super::{font_pixel_height_from_twips, zoomed_font_twips, zoomed_point_size_from_twips};

    #[test]
    fn zoom_retains_half_point_precision_before_native_device_conversion() {
        assert_eq!(zoomed_font_twips(170, 100), 170); // 8.5 pt remains 8.5 pt.
        assert_eq!(zoomed_font_twips(170, 110), 187); // 9.35 pt, not an early 9 pt.
        assert_eq!(zoomed_point_size_from_twips(170, 100), 9); // wx fallback only.
    }

    #[test]
    fn font_pixel_height_uses_vertical_device_dpi() {
        assert_eq!(font_pixel_height_from_twips(170, 96), 11);
        assert_eq!(font_pixel_height_from_twips(170, 144), 17);
        assert_eq!(font_pixel_height_from_twips(187, 144), 19);
    }
}

#[cfg(test)]
mod print_range_tests {
    use super::parse_topic_range_spec;

    #[test]
    fn parses_single_topics_and_multiple_ranges() {
        assert_eq!(
            parse_topic_range_spec("1-3, 7, 10-12", 12).unwrap(),
            vec![0, 1, 2, 6, 9, 10, 11]
        );
    }

    #[test]
    fn de_duplicates_overlapping_topic_ranges() {
        assert_eq!(parse_topic_range_spec("3-5, 4, 2-3", 8).unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn rejects_backwards_and_out_of_bounds_ranges() {
        assert!(parse_topic_range_spec("5-3", 8).is_err());
        assert!(parse_topic_range_spec("0-3", 8).is_err());
        assert!(parse_topic_range_spec("1-9", 8).is_err());
    }
}
