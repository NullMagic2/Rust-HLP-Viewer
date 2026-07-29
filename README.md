Build-fix 77 changes the HTML exporter at the architectural level. Exported topics are now formatted as ordinary semantic HTML derived directly from the decoded WinHelp paragraph/font/table model, rather than as absolutely positioned headless-layout text tokens that JavaScript tries to reconcile with browser font metrics. Paragraph margins, first-line indentation, spacing, authored hard breaks, tabs, alignment, borders, fonts, links, independent table columns, pictures, and standard buttons are translated from the same structures documented by the reverse-engineering reference. Browser word wrapping is therefore natural and occurs only when the rendered line actually reaches its paragraph margin. This removes the overlap/double-wrap class of regressions from build-fixes 70-76 while retaining green underlined hyperlinks, Related Topics/ALink, hierarchical Contents, cross-document navigation-root behavior, and `--export-html`.

Build-fix 76 makes HTML wrapping paragraph-aware instead of treating each already-wrapped retained baseline as an independent source line. The core layout now tags automatic wraps separately from authored hard breaks and tab/list segments, allowing the browser to recompute only the wraps actually required by its natural font metrics near the paragraph right margin. This removes redundant blank-looking continuation rows while keeping bullets, rules, pictures, controls, and later paragraphs synchronized with signed vertical-flow changes. Short isolated topic/section headings are also rendered bold. Build-fix 74's green underlined hyperlinks, build-fix 71 hierarchical Contents, and build-fix 72 scriptable `--export-html <source.hlp> [target.html]` remain intact.

Build-fix 68 separates the document currently displayed in the main topic surface from the document that owns the navigation pane. A manual/startup open establishes the navigation root; cross-document jumps may render another HLP in the main window without replacing Contents, Index, or Search. Contents actions, Show all, and the Contents command continue to resolve against the original HLP, while Index/Search continue to use that original file plus only the linked catalogs it authored. Back/Forward and external-topic rendering remain cross-file aware. This prevents jumps such as WORDPAD.HLP -> COMMON.HLP from replacing WordPad's navigation structure with COMMON.HLP's structure.

Build-fix 67 replaces the navigation overflow reveal with a viewer-owned, explicitly positioned Windows popup rather than asking `tooltips_class32` to move itself. When a Contents / Index / Search / Bookmarks / History label is actually clipped, the shared overflow binder computes the label's screen-space text origin and the custom popup paints the complete label directly over that cropped line. The popup uses the source control's real `WM_GETFONT` font, the existing RGB(249,249,158) WinHelp information yellow, black text and a one-pixel frame; it is no-activate and hit-test transparent, so it does not steal focus or mouse ownership. Index, Search, Bookmarks and History all use the same ListBox implementation; Contents uses the same popup with TreeCtrl text bounds. The 350 ms initial hover delay remains, while non-Windows/custom-window creation failures retain the wxToolTip fallback.

Build-fix 62 fixes the shared navigation overflow reveal at the point where it was actually disappearing. Because the tracking tooltip is deliberately placed over the clipped row, Windows may send `WM_MOUSELEAVE` to the underlying TreeCtrl/ListBox when the tooltip itself occupies that point. The viewer now checks the physical cursor against the real control client rectangle before hiding, so this synthetic leave no longer kills the tooltip immediately. Contents, Index, Search, Bookmarks, and History remain on the same shared row-aligned implementation and the same RGB(249,249,158) palette.

Build-fix 61 repairs the common navigation overflow reveal after build-fix 60 broke the tracking-tooltip contract. The shared Windows tooltip now identifies child controls the same way Microsoft's own control-tooltip example does (`TOOLINFO.hwnd` = containing window, `uId` = TreeCtrl/ListBox HWND with `TTF_IDISHWND`) and activates the tracking tool before sending its absolute position. All five navigation views continue to use the same overflow-only, row-aligned implementation and RGB(249,249,158) palette.

# Rust HLP Viewer

Build-fix 60 fixes the common native tracking-tooltip registration used by every navigation item view. The shared tooltip is now registered as a real window tool (`TTF_IDISHWND`) with the TreeCtrl/ListBox HWND in `uId`, matching the Win32 tracking-tooltip contract, and the tooltip window is owned by that same control so transparent mouse forwarding returns to the correct widget. The Windows navigation path also no longer creates empty wxToolTip objects during binding or list/tree rebuilds. This is one fix for Contents, Index, Search, Bookmarks, and History rather than another tab-specific workaround.

Build-fix 59 fixes the missing Contents overflow reveal seen in build-fix 58. Windows Contents no longer assumes wxWidgets/TreeCtrl will display its automatic clipped-label tooltip. The tree view's built-in tooltip association is explicitly detached with `TVM_SETTOOLTIPS`, and Contents now uses the same pre-coloured row-anchored native tracking-tooltip implementation as Index, Search, Bookmarks, and History. A clipped label therefore has exactly one single-line reveal on its own row; labels that fit remain tooltip-free.

Build-fix 58 makes navigation overflow reveals visually consistent with the native Windows Contents label tip. Contents uses only the TreeCtrl's own clipped-label tooltip, eliminating the duplicate tooltip added in build-fix 57. Index, Search, Bookmarks, and History now use a pre-coloured native tracking tooltip anchored to the hovered row's top-left corner, so the complete clipped label appears directly on the same line instead of below the mouse. Rows that fit remain tooltip-free.

Build-fix 57 extends overflow-only tooltips across the complete navigation notebook. Contents, Index, Search, Bookmarks, and History now reveal the full hovered item only when its label is clipped by the current pane width; labels that fit remain tooltip-free. Contents uses tree-item hit testing and its actual on-screen label origin, while all four list boxes share the existing scrolled-row-aware overflow behavior.

Build-fix 56 replaces the old plain-text printer with the retained WinHelp layout on Windows. **File > Print... (Ctrl+P)** can print the current topic, a one-based topic range such as `3-8` or `1-3, 7, 10-12`, or every presentation topic. Printer layout now retains authored font face/family mapping, size, weight, italic/underline/strikeout/small-caps scaling, foreground and explicit background colours, paragraph/table geometry, pictures, paragraph borders, and safe embedded-control placeholders. Each selected topic starts on a new printer page and paginates against the printer device context without printing navigation chrome or text-selection highlighting.

Build-fix 55 removes the remaining first-frame blink from Windows HLP hotspot previews. Instead of letting wxWidgets lazily create a tooltip in the system `#FFFFE1` colour and recolouring it after it appears, each topic canvas now owns a hidden native `tooltips_class32` control that is created and styled to `RGB(249,249,158)` (`#F9F99E`) before any hover tool is registered. The control then uses normal Windows tooltip subclassing/timing while hotspot changes only update its retained text buffer. There is no palette polling worker and no post-show recolour.

Build-fix 54 removes the mild flicker that could appear after build-fix 53 successfully recoloured a lazily-created Windows tooltip. The retry worker now exits as soon as it finds and styles the real `tooltips_class32` HWND, and the repeated `TTM_UPDATE` repaint forcing has been removed. The requested tooltip colour remains exactly `RGB(249,249,158)` (`#F9F99E`) with black text; the normal topic page remains `RGB(255,255,228)` (`#FFFFE4`).

Build-fix 53 fixes the reason build-fix 52's requested `RGB(249,249,158)` (`#F9F99E`) never appeared on-screen. wxWidgets creates the native Windows `tooltips_class32` HWND lazily, after `set_tooltip()` returns, so the previous one-shot Win32 palette call often ran before there was a tooltip window to modify. Hotspot tooltips now arm a short bounded palette retry window after the hover target changes; once the native tooltip HWND appears, the same `TTM_SETWINDOWTHEME`, `TTM_SETTIPBKCOLOR`, and `TTM_SETTIPTEXTCOLOR` path is re-applied to the actual control. The requested colour constant itself is unchanged from build-fix 52.

Build-fix 52 changes the shared WinHelp tooltip/popup information surface to the user-sampled `RGB(249,249,158)` (`#F9F99E`), making it clearly darker and more yellow than the `RGB(255,255,228)` (`#FFFFE4`) topic background. Native hover tooltips and legacy popup-note rendering continue to use the same palette constant.

Build-fix 51 matches WinHlp's information yellow exactly: hover tooltips and the legacy popup-note fallback now share `RGB(255,255,225)` (`#FFFFE1`), visibly darker than the normal `RGB(255,255,228)` (`#FFFFE4`) help-page cream. This replaces the older over-saturated popup fallback while retaining all build-fix 50 splitter and bookmark-tooltip behavior.

Build-fix 50 makes the complete Contents / Index / Search / Bookmarks / History navigation column horizontally resizable with a native wxDragon splitter. Drag the divider between the navigation column and help document; the current width is remembered across F9 hide/show operations. Build-fix 59 makes clipped-label reveals consistent across the notebook: all five navigation item widgets use the same row-anchored native tracking-tooltip path on Windows. Build-fix 49's printing compilation fixes remain included.

Build-fix 49 fixes the build-fix 48 Windows printing backend under the project's strict `-D unsafe-code` policy by scoping `#[allow(unsafe_code)]` only to the Win32 print FFI/backend functions. It also constructs `MessageDialog` instances before calling `show_modal()`, matching wxDragon's builder API.

A native, from-scratch Windows Help (`.HLP`) viewer written in Rust. The desktop interface uses **wxDragon 0.9.17** (wxWidgets) by default; binary parsing, document semantics, navigation, and retained layout remain GUI-independent.

### Printing

Build-fix 56 routes Windows printing through the same retained topic layout model used on screen instead of reconstructing plain text. Before the native printer dialog, choose **Current topic**, **Topic range...**, or **All topics**. Range input accepts comma/semicolon-separated one-based items and inclusive ranges such as `1-3, 7, 10-12`; overlapping entries are de-duplicated. Selected topics start on separate printer pages and preserve retained text styling, authored colours/backgrounds, paragraph/table placement, pictures, borders, and safe embedded-control placeholders. Printing remains read-only and never writes to the HLP, CNT, or GID.

## Milestone 0.7.1

Build-fix 47 adds native text selection and clipboard editing to the viewer. Drag across retained topic text to select it, then use **Edit > Copy** or `Ctrl+C`; **Edit > Select All** / `Ctrl+A` targets the active topic region or focused query field. **Edit > Paste** / `Ctrl+V` inserts plain clipboard text into the focused Index or Search query field while topic surfaces remain read-only. Hyperlinks now activate on mouse-up only when the gesture did not become a selection drag; existing Left/Right topic navigation and Alt+Left/Alt+Right Back/Forward navigation are preserved.

Build-fix 46 adds read-only WinHelp 4.x `.GID` Contents support. A readable authored `.CNT` remains the preferred source; when it is absent, the engine now discovers a same-basename `.GID` case-insensitively and reconstructs the cached hierarchy from `|CntText`, `|CntJump`, and the empirically decoded trailing Contents record in `|Flags`. The supplied Windows 95 WordPad specimen proves 35 cached rows whose hierarchy bytes exactly reproduce the 35 numbered CNT levels, while a second GID lacking `|CntText`/`|CntJump` is correctly treated as non-Contents-bearing. `|FILES` is also decoded for cached `:Index`/`:Link` catalogs, with absolute Win9x cache paths reduced to portable sibling filenames. The viewer does not write, refresh, or invalidate GID caches; this milestone is deliberately a safe reader/fallback only.

Build-fix 44 is a compile-only correction to the build-fix 43 regression tests: the synthetic bad-magic constant is explicitly typed as `u32` before `to_le_bytes()`, removing Rust error E0689 without changing runtime parsing.

Build-fix 45 restores two pieces of WinHelp navigation/UI fidelity. Windows tooltips use the classic Microsoft `InfoWindow` shade `RGB(255,255,225)` with black text instead of inheriting the host theme's white tooltip surface. The Contents pane now opens in **Hierarchical view**, which shows only the hierarchy authored by the discovered `.CNT` sidecar; a separate **Show all** button exposes every decoded physical topic. Missing `.CNT` data is reported explicitly rather than silently flattening all topics into the Contents tree.

Build-fix 21 corrects the Calculator `Related Topics` control after a direct KB917607 WinHlp32 trace. The object previously shown as `[embedded picture]` is not a bitmap: its inline compact record is type `0x05`, and Microsoft routes that family through the hosted-window renderer. CALC's descriptor `!,AL("A_CALC_LIST_EQUIV;A_CALC_KEYB_SEQ")` selects WinHlp32's built-in blank `BUTTON`; after creation WinHlp32 resizes the empty-label form to a final 12x12 control. The viewer now decodes inline compact objects by their nested TOPICLINK type, paints this verified standard-button form without creating the authored native child/control, and preserves the existing picture path for real `0x03`/`0x22` graphics. Build-fix 30 additionally routes the button's retained macro through the safe viewer-local macro dispatcher, so `ALink` works without enabling arbitrary hosted code.

Build-fix 21 also adds **File > Recent Documents** for the five most recently opened HLP files. Build-fix 22 makes that state portable: the small human-readable `hlp-viewer.cfg` is stored **in the same directory as the running viewer executable**, on Windows and other platforms, with no `%LOCALAPPDATA%`, `%APPDATA%`, XDG, or working-directory fallback. The config is created after the first successful document open and contains one `recent_document=` entry per path, newest first.

Build-fix 23 is a compile-only correction to that MRU implementation: the config parser now explicitly constructs `Vec<PathBuf>` instead of leaving the accumulator type to inference, and the stale `PanelStyle` import has been removed. Runtime MRU/config behavior is unchanged.

Build-fix 24 corrects the vertical alignment of CALC.HLP's `Equivalentes de teclado` table. The file deliberately interleaves empty compact display cells between visible cells in independently flowing columns. WinHlp32 consumes the completely empty display form without advancing its vertical cursor; the Rust layout had treated it as a normal blank text line, causing selected columns to start lower and inflating row spacing. Table-cell empty fillers now have zero height, while real blank-line commands and visible/bordered content keep their normal layout semantics. An authored rule-only display immediately before a table also receives an 8-pixel post-rule gap so the first table line does not crowd the lower horizontal rule.

Build-fix 28 retains real native font-baseline data for genuinely mixed-font text lines, replacing the earlier height-only heuristics. A subsequent direct decode of the supplied CALC.HLP proved that this was not, however, the list-marker path: the square and triangle markers are compact inline `0x22` graphics (`|bm0`, 3x7 pixels, and `|bm1`, 4x8 pixels), not text glyphs. Build-fix 29 therefore fixes the actual object geometry. Inline pictures now participate in line baseline finalization with their bottom edge as the object baseline, while text uses its retained font baseline; picture hotspot overlays follow the same shift. This is the path that changes the visible CALC markers. Floating pictures and ordinary paragraph geometry remain unchanged.

Build-fix 30 completes the CALC `Related Topics` block. The blank 12x12 stock ALink button and its label row are aligned to the left edge of the authored separator and receive an additional 4-pixel gap below it. The retained `AL("A_CALC_LIST_EQUIV;A_CALC_KEYB_SEQ")` command is now a safe viewer-local operation: `ALink`/`AL` performs an exact semicolon-delimited lookup in the HLP's authored A keyword table, opens a native **Topics Found** chooser for multiple matches, and navigates through the existing topic/history machinery. The stock blank button itself is also clickable and uses the same default-deny macro dispatcher.

Build-fix 31 corrects the last horizontal alignment defect in that block. Build-fix 30 had achieved a margin-aligned screenshot by zeroing the ALink paragraph's authored left/first-line indents. The renderer now leaves those signed indents intact, remembers the preceding rule-only record's actual rendered border x-coordinate, and translates only the stock 12x12 ALink button plus its same-line text to that exact edge. The 4-pixel post-rule gap and safe ALink behavior are unchanged.

Build-fix 32 removes two native sizing approximations that could move otherwise-correct WinHelp formatting. The retained layout engine now carries independent horizontal and vertical device DPI: x-axis paragraph indents/tabs/table geometry use the canvas `LOGPIXELSX`, while paragraph/line spacing and font-height fallbacks use `LOGPIXELSY`. On Windows, HLP text measurement and painting share a GDI `LOGFONTW` backend created from the retained twip size, so HC30 half-point sizes such as 8.5 pt are not rounded to a whole wxWidgets point before native measurement. The existing wxDragon font path remains the portable fallback. Build-fix 33 extends that same per-axis DPI context to authored bitmap physical resolution and WMF mapping-mode natural dimensions.

Build-fix 33 implements that graphics follow-up. Bitmap alternatives with authored x/y resolution now retain those values and derive their natural document size from the actual target `LOGPIXELSX`/`LOGPIXELSY`; zero-resolution bitmaps remain pixel-sized. WMF physical mapping modes likewise use the target x/y DPI instead of a fixed 96-DPI layout assumption. The bounded WMF adapter may still rasterize to its stable compatibility surface internally; retained layout and picture hotspots scale that safe RGBA result to the reference natural size.

Build-fix 34 completes the common international text path. LinkData2 now decodes the major legacy Windows SBCS families plus Japanese Shift-JIS, Korean, GBK and Big5 DBCS text before retained layout. Explicit non-default per-face record-11 charset bytes remain authoritative; absent/default metadata uses conservative face-name/`LANGID` inference for common historical Windows families. CJK runs gain no-space break opportunities while Latin words remain grouped and basic kinsoku-style punctuation stays attached. Build-fix 39 subsequently closes Johab as Windows CP1361 and proves that `OEM_CHARSET` is intentionally selected through the host GDI charset/code-page database rather than being one fixed HLP encoding. Windows builds now decode that path through the active `CP_OEMCP`; non-Windows builds retain an explicit deterministic fallback.

Build-fix 35 fixes two resize/zoom defects exposed by the Calculator help screenshots. Signed WinHelp line advances now scale with the viewer's text zoom independently from physical device DPI, so 150%-200% text no longer keeps the compact 100%-scale line pitch. Main-window resize handling now follows the content host after wxWidgets has applied the frame sizers, so maximize-then-restore immediately reflows text to the restored viewport and repaints the complete cream page/background. The visible browse strip no longer contains Contents or Back/Forward-history buttons; those commands remain available from the Navigate menu and keyboard shortcuts. The About dialog is reduced to basic application/toolkit identification.

Build-fix 36 aligns that reduced browse strip with the document itself. When the navigation pane is visible, the top row reserves the same left column as the pane and centres the controls over the right-hand help surface; when the pane is hidden, the alignment gutter disappears and the strip recentres over the expanded help surface. Because the black/cream page frame uses symmetric horizontal insets inside `content_host`, the strip is consequently centred on the visible text background rather than on the complete application window.

Build-fix 38 incorporates the supplied HLP application icon into the project. The source tree now carries the original PNG plus a generated multi-size Windows ICO under `viewer/assets/`, the `viewer` crate embeds that ICO into Windows executables through a small `build.rs` resource step, and third-party attribution is recorded in `docs/THIRD_PARTY_ASSETS.md`.

Build-fix 40 completes the Windows icon path: after constructing the main wxWidgets frame, the viewer explicitly loads both large and small variants from the executable's embedded icon resource and assigns them to the native window. This fixes the generic toolkit icon that could still appear in the title bar even though Explorer already saw the embedded executable icon.

Build-fix 39 finishes the five-item residual executable audit against the exact 285,696-byte KB917607 WinHlp32 reference. Johab charset `0x82` is Windows CP1361 and now has a deterministic decoder; OEM charset `0xFF` is host-selected GDI behavior and Windows builds mirror it through the active `CP_OEMCP`. The reference Unicode path uses `MultiByteToWideChar` followed by `TextOutW` rather than a private shaping engine, so Windows non-ANSI/default runs now preserve the authored face/charset pair into GDI. Character command `0x85` is implemented as the glyphless signed horizontal line-origin override used by alignment. The remaining structurally accepted hotspot envelope values are verified action-inert in the KB917607 click dispatcher. Arbitrary hosted controls are verified to start at a two-device-inch creation rectangle and then negotiate their final runtime size through private message `0x706B` or `GetWindowRect`; safe mode deliberately does not execute those document-supplied controls and uses the verified initial rectangle as its placeholder. The consolidated manual is retitled **Microsoft WinHelp (.HLP) Internal Format — Reference Manual** and records the new address-level findings.

Build-fix 41 makes bookmarks persistent and gives the Bookmarks pane compact adjacent **+ / -** controls. Bookmarks are saved immediately beside the executable as `<program-name>.bookmarks` (normally `hlp-viewer.bookmarks`) and can reopen their source HLP after restarting the viewer.

Build-fix 42 fixes the remaining zoom-sensitive alignment defect in CALC.HLP's Related Topics row. The blank 12x12 stock ALink button is a hosted `!label,macro` BUTTON rather than a bitmap; it now shares the adjacent text baseline during retained line finalization, so zooming the font no longer leaves the square pinned to the top of the line.

Build-fix 43 improves format-family diagnostics for misleading `.HLP` extensions. Files with the legacy Microsoft `LN 02` signature (`0x00024E4C`), including MS-DOS/QBasic help databases, are now identified as a separate unsupported help/index family instead of being described merely as a corrupt Windows WinHelp file. The classic WinHelp magic remains strictly `0x00035F3F`.

Version 0.7.1 is a compatibility hardening release for real-world WinHelp hyperlinks. LinkData1 `0xE0`/`0xE1` hotspots remain physical `TOPICOFFSET` jumps, while `0xE2`/`0xE3`/`0xE6`/`0xE7` are represented separately as context-hash jumps and resolved through the HLP `|CONTEXT` table. This fixes links such as the Calculator help `operadores` hyperlink that previously surfaced a huge bogus `Unresolved internal TOPICOFFSET` value. A physical-offset fallback is retained for unusual producers. The correction applies to main-topic navigation, popup/secondary target metadata, diagnostics, and destination-title tooltips; build-fix 16 presents every resolved target in the single main help surface.

Build-fix 20 corrects the popup-hover half of that restoration. Ordinary text/image hotspots still expose the resolved destination title, but popup-marked hotspots now expose the **actual visible text of the resolved popup topic** instead of an internal fallback label such as `Popup: Topic 6`. The preview is extracted from the same formatting-decoded presentation used by the renderer and retains paragraph/line-break structure. Activating popup/secondary targets still follows build-fix 16's single-main-surface policy and does not create detached frames.

Build-fix 17 re-audits the remaining formatting assumptions against the exact 285,696-byte KB917607 WinHlp32 reference. It removes the incorrect `VariableField`/`DType` interpretation of character bytes `0x20`/`0x21`, implements the actual zero-width signed `0x85` control, renders compact `0x03`/`0x22` graphics at top level and recursively in tables, safely represents `0x05`/`0x24` hosted controls without executing authored native code, preserves unknown hotspot families on their exact Microsoft structural boundaries, and treats border styles 5-7 as reserved/no-paint. The same audit corrects the font model: all generations use 11-byte descriptors, face-name slots are 20 or 32 bytes, `|SYSTEM` record 11 supplies per-face charset bytes, and record 9 supplies the locale gate. Build-fix 34 completes the common portable Windows charset families and DBCS decoding plus CJK wrapping; native glyph shaping remains host-renderer behavior.

Build-fix 2 also preserves popup/body text when a real-world HLP places display records outside the visual ranges advertised by its topic header. Such records were already retained for plain-text/search purposes but were accidentally omitted from `TopicPresentation`; displayable unclassified records are now recovered into the scrolling body in TOPICPOS order with a diagnostic warning.

Version 0.7.0 introduced the bounded parser and default-deny execution layer for classic WinHelp macros. CONFIG records, per-topic macros, and macro hotspots continue to pass through the same typed safety policy; safe viewer-local navigation/UI operations are supported while process/shell execution, DLL routine registration, host interaction, unknown operations, malformed programs, and unsupported legacy UI mutation remain blocked.

## Milestone 0.6.2

Version 0.6.2 originally added a native browser-style toolbar above the help document. Current build-fix 36 keeps the navigation commands but removes the visible **Contents** and **Back/Forward-history** buttons: Contents/Back/Forward remain in the Navigate menu and on their keyboard shortcuts. The visible in-window strip now contains **◀/▶** physical topic navigation, conditional **⇤/⇥** authored browse buttons, **☰** for the navigation pane, and **−/+** for text zoom (70%-200% in 10% steps). The strip is horizontally centred over the cream help-page region rather than over the complete frame, and recentres automatically when the navigation pane is hidden or shown. **View > Navigation Pane (F9)** still shows or hides the complete Contents / Index / Search / Bookmarks / History side panel. Help text starts at the requested 110% zoom, and the viewer uses a pale `RGB 255,255,228` help-content background inside a black bordered page set on a light gray host area. Build-fix 9 follows the verified Microsoft KB917607 WinHlp32 reference exactly for the old `RGB(1,1,0)` descriptor sentinel: only that value inherits the active text/page colour; nearby dark values and legitimate authored purple/blue remain untouched. Build-fix 10 extends that direct binary reference to paragraph and table formatting: DPI/144 indents and spacing, default/custom tabs, deferred right/center tab alignment, signed line spacing, no-wrap paragraphs, two-thirds-height small caps, Microsoft border style/clearance behaviour, corrected table width/gap headers, exact type-0 versus absolute table scaling, and independent per-column vertical flow. Build-fix 11 remains the current UI/font baseline. Build-fix 12 completes the traced table-cell framing for Windows 3.0 `0x04` and Windows 3.1+ `0x23` tables by decoding each signed-column + bounded nested TOPICLINK record exactly as WinHlp32 does, and fixes Contents synchronization so a hidden wxWidgets root is never expanded indirectly by `ensure_visible()`. Build-fix 13 follows WinHlp32's recursive dispatcher all the way through nested `0x04`/`0x23` cells: nested tables are retained as a real cell tree, receive their parent column's origin/width, maintain their own independent column cursors, and return their maximum height to advance only the containing parent column. Build-fix 14 confirms from the Microsoft GDI call graph that WinHelp tables are layout-only and do not imply a visible grid; authored paragraph borders provide any visible rules. It also makes destination-title tooltips and popup-topic windows properly transient: clicks outside dismiss them and main-topic navigation closes any active popup. Build-fix 15 fixes two retained-layout artifacts without changing parser semantics: hanging/negative first-line indentation now survives final alignment on the first visual line only, and border-only rule paragraphs no longer receive a fabricated 16-pixel blank text row. The `.CNT` sidecar remains the authoritative Contents hierarchy whenever present. Build-fix 16 removes detached popup/secondary topic frames entirely: every resolved destination opens in the single main help surface, compact Related Topics separators render as one aligned rule with deliberate spacing below it, and the visible browse toolbox uses explicit symmetric margins and regular pair/group gaps. Per-run authored text backgrounds remain opaque, raw half-point font precision survives through zoom, and native measurement/painting stay synchronized so wrapping and hotspot geometry remain aligned.

## Milestone 0.6.1

Version 0.6.1 originally added hyperlink hover context across the main viewer and auxiliary topic surfaces. Build-fix 20 now distinguishes the two useful hover payloads correctly under build-fix 16's single-surface navigation policy: ordinary text/graphical hotspots show the resolved destination title, while popup-marked hotspots show the resolved popup topic's visible help text. Every activated destination still opens in the main help surface rather than a floating window.

The flattened source layout and compact `%LOCALAPPDATA%\hv` external cache introduced by 0.6.0-buildfix3 are retained.

Version 0.6.0 turns the viewer into a practical documentation browser rather than only a topic renderer. It discovers and parses authored WinHelp `.CNT` sidecars, reconstructs the hierarchical Contents tree, decodes the HLP keyword B+ tree/data families, builds a deterministic in-memory full-text index, and exposes Contents / Index / Search / Bookmarks / History in one native wxDragon notebook pane. The graphics and navigation behavior from 0.5.1 remains intact.

Relative `.CNT` `:Index` and `:Link` help files are opened one hop for integrated keyword/full-text lookup, with a 32-file cap on sidecar-driven filesystem expansion. Absolute/UNC catalog references are not opened automatically; explicit user-activated cross-file links keep their existing behavior. Multi-topic keywords use a native chooser and every result retains the source HLP path, so cross-file selections feed the same Back/Forward machinery as ordinary hyperlinks. Missing sidecars, malformed optional keyword tables, and unavailable linked manuals are non-fatal navigation warnings. Recursive `.CNT` `:Include` expansion is intentionally not followed in 0.6.0; it is reported as a warning rather than silently loading an unbounded sidecar graph.

Implemented through this milestone:

- same-basename or `|SYSTEM`-named `.CNT` discovery plus CP1252 parsing of `:Title`, `:Base`, `:Index`, `:Link`, books, topics, external HLP targets, and named windows;
- native hierarchical Contents tree, synchronized to the active topic when an authored same-file target can be resolved, plus explicit **Hierarchical view** / **Show all** controls; missing `.CNT` data is reported instead of being silently replaced by a flat topic list;
- WinHelp `|?WBTREE` / `|?WDATA` keyword-table decoding, including multiple topic targets per keyword and inert macro-only targets;
- a pre-folded in-memory search index that ranks title and authored-keyword matches above body-text matches;
- one native notebook pane containing Contents, Index, Search, Bookmarks, and History;
- incremental Index/Search filtering, cross-file aggregation through one-hop `.CNT` `:Index`/`:Link` references, and native multi-topic selection;
- persistent file-qualified bookmarks with compact `+` / `-` controls, stored beside the executable in `<program-name>.bookmarks`, plus a visible browser-history list backed by `NavigationLocation` values;
- real raster image decoding for topic commands `0x86`..`0x88`, including indexed internal `|bmN` streams and embedded `*wd` graphics objects;
- WinHelp logical graphics-stream alternatives with raw, RLE, LZ77, and LZ77-then-RLE packing;
- 1/4/8/16/24/32-bit DIB decoding, DIB palettes, bottom-up scan-line conversion, and flagged palette transparency; portable 1/16/24/32-bit DDB records are also accepted;
- native wxDragon bitmap painting from engine-owned RGBA data, with proportional shrink-to-fit when an image is wider than its topic viewport;
- Windows WMF alternatives (`type 0x08`) decoded with the same WinHelp packing modes and rendered to RGBA through a narrow GDI adapter; malformed WMFs remain non-fatal placeholders;
- SHG/MRB-style graphical hotspot tables decoded into scaled retained hit-test rectangles, including popup, jump, named-window, and inert macro targets;
- authored `bmc` inline placement plus `bml`/`bmr` left/right floating placement with paragraph-local text wrapping and correct release below each float;
- non-fatal fallback placeholders for palette-dependent 4/8-bit DDBs, embedded element type `0x05`, and malformed/unsupported graphics records;
- shared pixel storage for repeated indexed images plus explicit graphics dimension, palette, decompression, and allocation limits;
- direct opening now starts at the first displayable presentation topic instead of implicitly executing the authored Contents target; Navigate > Contents still uses the HLP's explicit Contents mapping;
- retained text is measured by wxWidgets with the same family-aware native font object used for painting, so spaces, line heights, and run positions follow native Windows metrics rather than the previous heuristic estimator;
- Left and Right are explicitly bound to the previous/next physical presentation index (for example Topic 1/77 -> Topic 2/77), independent of the HLP's optional authored browse sequence;
- Navigate now exposes Previous Topic (`Left`) and Next Topic (`Right`) instead of Browse Previous/Next (`Ctrl+PageUp`/`Ctrl+PageDown`);
- the main scrolling topic canvas is explicitly focusable, receives focus after loading or changing a topic, and regains focus when the user clicks a main topic canvas;
- Alt+Left and Alt+Right are explicitly bound to Back/Forward history on the main frame and topic surfaces;
- handled arrow events are consumed so wxWidgets cannot reinterpret them as scrolling/navigation;
- all 0.1-0.3 HLP container, `|SYSTEM`, LZ77/phrase, `|TOPIC`, `|FONT`, LinkData1, table, retained-layout, fixed-region, and native wxDragon rendering support;
- Windows 3.0 `|TOMAP` topic-number lookup, including the special index topic at element zero;
- Windows 3.1+/95 `|CONTEXT` context-hash B+ tree parsing;
- `|CTXOMAP` numeric map-ID lookup;
- HCW `|TopicId` symbolic context-name metadata and `|Viola` default-window assignments;
- reconstruction of WinHelp `TOPICOFFSET` anchors from physical topic-block number plus display-record `TopicLength` character counts;
- internal topic jumps plus popup-marked topic targets, all displayed in the single main help surface;
- external/cross-file HLP jumps with paths resolved relative to the currently loaded HLP;
- hover previews for text links and graphical/image hotspots, including cross-file targets: ordinary jumps show destination titles while popup-marked links show the popup topic body; activation remains in the single main surface;
- external hotspot opcodes retain WinHelp low-bit semantics for destination decoding (`0xEA`/`0xEE` popup-marked, `0xEB`/`0xEF` ordinary navigation) while build-fix 16 presents both kinds in the main viewer;
- browser-style Back/Forward history, including restoration across HLP files;
- authored browse-sequence parsing/resolution for HC31+/HCW and HC30, used by safe `Next`/`Prev` and `BrowseButtons` macro handling while viewer Previous/Next remains physical topic navigation;
- Contents navigation from the SYSTEM contents offset or HC30 index topic;
- HC31/HCW `[WINDOWS]` records, including captions, dimensions, region colours, and window names, retained as compatibility metadata;
- explicit named/numbered secondary-window targets, HCW `|Viola` default-window assignments, popup hotspots, and popup macros resolve normally but are redirected into the single main viewer surface;
- no native floating topic frames are created; Back/Forward history remains the navigation model for every resolved destination;
- modern Windows font substitution preserves WinHelp family intent: Roman text uses Times New Roman, Swiss text uses Microsoft Sans Serif, Modern/fixed-pitch text uses Consolas, Script text uses Segoe Script, and symbol/decorative faces are preserved when glyph identity matters;
- macro strings are parsed by a default-deny policy; allow-listed viewer-local navigation/UI macros execute, while unsafe/unknown/unsupported macros remain inert and are logged;
- one executable: `hlp-viewer.exe`.

## Single executable, diagnostic mode, and scripted HTML export

Normal interactive use:

```bat
build\hlp-viewer.exe
```

Open a file directly:

```bat
build\hlp-viewer.exe manual.hlp
```

Inspect the same file through the parser without initializing wxDragon:

```bat
build\hlp-viewer.exe --dump-file manual.hlp
```

Detailed record/navigation diagnostics:

```bat
build\hlp-viewer.exe --dump-file manual.hlp --verbose
```

Export directly to self-contained HTML without opening the interface:

```bat
build\hlp-viewer.exe --export-html manual.hlp
```

The omitted target defaults to `manual.html` beside the source. To choose the destination explicitly:

```bat
build\hlp-viewer.exe --export-html manual.hlp D:\Exports\manual.html
```

This mode uses the same exporter as **File > Export to HTML...**, including hierarchical `.CNT`/`.GID` Contents when available, links/tooltips, cross-document embedding, formatting, Index/Search catalogs, and the gray/cream WinHelp shell. It does not initialize wxDragon. A successful export writes the resulting path to stdout, making it easy to call from batch/PowerShell scripts; failures return a non-zero process status.

On Windows the application is still linked as a GUI-subsystem program, so ordinary launches do not create a console window. Diagnostic/export/help/version modes attempt to attach to the parent console before their first output operation. That tiny platform bridge is isolated inside `hlp-viewer/src/support.rs`; the entire `hlp` engine crate remains safe Rust.

## Workspace

```text
crates/
  hlp/          HLP parser + document/navigation model + retained layout
  hlp-viewer/   Native wxDragon UI + integrated --dump-file / --export-html entry points
  wmf-render/   Narrow Windows-only legacy-WMF -> RGBA GDI adapter
```

Only `hlp-viewer` depends on wxDragon. The `hlp` crate can be tested without compiling wxWidgets. The small `wmf-render` package exists only to isolate the Windows GDI unsafe boundary from the otherwise safe `hlp` engine. The consolidated tree contains 22 Rust source files total; most are focused binary-format modules rather than architectural layers.

## Windows 11 build requirements

Install:

1. Rust stable with the MSVC toolchain;
2. Visual Studio 2022 Build Tools or Visual Studio with **Desktop development with C++**;
3. a Windows SDK;
4. CMake;
5. Ninja.

Then run:

```bat
build_hlp.bat
```

The 0.7.1 build script intentionally runs tests only for the `hlp` engine crate before performing a single release build of `hlp-viewer`. This avoids compiling wxDragon/wxWidgets once for a debug test profile and then again for release.

Build-fix 3 sets Cargo's target directory to `%LOCALAPPDATA%\hv` by default (or `%TEMP%\hv` if `LOCALAPPDATA` is unavailable). wxDragon itself still creates fixed CMake subdirectories internally, but they now start from a very short root instead of inheriting the project extraction path. To choose another cache root, set `HLP_VIEWER_TARGET_DIR` before running the build script.

### Extracting the KB917607 WinHlp32 reference

`tools\extract_winhlp32_kb917607.bat` is the preferred Windows-only reference utility. It expands an x64 Windows 8.1 KB917607 MSU, locates the PA30 WinHlp32 delta, reconstructs the target through Windows `msdelta.dll`, and accepts the result only when it is 285,696 bytes with SHA-256 `8496f19bc1d898180b57aac61326bdfcf5a48f760811283bbd604aa7a2c13f85`.

```bat
tools\extract_winhlp32_kb917607.bat Windows8.1-KB917607-x64.msu
```

The PowerShell extractor is retained as an alternative/reference implementation. The extracted Microsoft binary is a comparison artifact only; it is not required to build or run this viewer and is intentionally not included in the source archive.

### Cleaning

For ordinary iteration:

```bat
clean_source.bat
```

This removes only `build\` and **preserves the external Cargo target/cache directory**, including the expensive wxDragon/wxWidgets native build cache.

To force a completely clean rebuild:

```bat
clean_all.bat
```

That removes `build\`, the external Cargo/native cache, and any legacy in-tree `target\` directory, so wxDragon/wxWidgets must compile again.

To discard all generated build files while keeping only the finished executable:

```bat
clean_tmp_files.bat
```

This preserves `build\hlp-viewer.exe`, removes the external Cargo/wxDragon cache plus any legacy in-tree `target\` tree, and removes any other files under `build\`. If the packaged executable is missing, the script can recover the release executable from either cache location before cleaning.

## Navigation notes

The viewer keeps WinHelp's two location systems separate:

- `TOPICPOS` locates transformed records in `|TOPIC`;
- `TOPICOFFSET` is the legacy block/character cursor used by contexts, hotspots, browse links, and history.

Cross-file hotspot filenames are treated as paths relative to the HLP containing the link unless already absolute. No executable/DLL macro action is used to resolve links.

Popup and secondary topics are rendered with the same retained layout engine as the main window. Since 0.4.5, popups are transient native owned frames: they open at the clicked hotspot, carry no taskbar entry, close on Escape or after losing activation, and replace themselves when a popup link inside a popup is followed. Secondary windows remain persistent, honor parsed caption/geometry/topmost/maximize/auto-height metadata where available, and can navigate their own document surface.

The viewer deliberately modernizes ordinary HLP typefaces without flattening their semantic family. On Windows, Roman runs request Times New Roman, Swiss runs request Microsoft Sans Serif, Modern/fixed-pitch runs request Consolas, and Script runs request Segoe Script through wxDragon/wxWidgets. Bold, italic, underline, strikeout, authored point size, colour, and fixed-pitch intent still come from the HLP. Original face names remain retained, and known symbol/dingbat plus decorative faces are preserved because substituting those can change glyph meaning.

## Safety model

Classic HLP files are untrusted binary input. The project therefore:

- uses no unsafe Rust in the `hlp` engine crate;
- isolates legacy WMF playback in the tiny `wmf-render` platform adapter, where GDI handles/pointers have a documented lifetime and bounded render target;
- isolates the separate `AttachConsole` Windows FFI call in the application shell and documents its safety contract;
- bounds-checks file offsets, lengths, B+ tree pages, navigation entry counts, phrase indices, transformed positions, table/tab counts, picture payloads, and hotspot structures;
- detects directory/TOPICLINK/B+ tree leaf cycles and caps pathological expansions;
- treats unknown formatting opcodes as recoverable record-local warnings;
- never executes macro operations outside the explicit viewer-local allow-list; shell/process execution, DLL registration, and unknown operations remain blocked.

## Format references

The HLP on-disk format was never completely published by Microsoft. This is a fresh Rust implementation guided by the long-standing reverse-engineered HelpDeco format documentation and cross-checked where useful against other open WinHelp implementations. `docs/FORMAT_NOTES.md` records compact parser assumptions. `docs/WINHLP32_FORMATTING_REFERENCE.md` preserves the detailed formatting audit against the user-extracted, hash-verified Microsoft KB917607 runtime. Build-fix 37 additionally provides `docs/MICROSOFT_WINHELP_INTERNAL_FORMAT_REFERENCE.md` and its DOCX edition as the consolidated reference manual: container/named streams, `|SYSTEM`, phrase compression, TOPIC/TOPICLINK, formatting, fonts/charsets/DPI, recursive tables, graphics/WMF, hotspots/navigation, `.CNT`/keywords, hosted controls, safe macros, corrections to older interpretations, confidence labels, quick-reference tables, and the executable-address appendix.

## Next milestone

With safe macro compatibility implemented in **0.7.0** and context-hash hyperlink compatibility hardened in **0.7.1**, version 0.8 moves to printing, text selection/copy, accessibility/keyboard polish, DPI-change reflow, persistent settings, and packaging.

## Third-party assets

The application icon is credited in [`docs/THIRD_PARTY_ASSETS.md`](docs/THIRD_PARTY_ASSETS.md).
