# Rust HLP Viewer implementation roadmap

## Design invariants

1. **wxDragon is the default desktop library**, but never leaks into the `hlp` engine crate.
2. `.HLP` files are interpreted directly; no `winhlp32.exe`, WebView2, browser engine, or HTML conversion is required. HTML export is an optional one-way output and is never a rendering dependency.
3. Binary offsets/sizes are untrusted and parsing stays bounds checked.
4. WinHelp macros remain data until a specifically supported operation can be represented by an allow-listed typed command.
5. Layout is retained and deterministic before wxDragon paints it.
6. `TOPICPOS` and `TOPICOFFSET` remain distinct typed coordinate systems.
7. User-facing release builds contain one application executable; parser diagnostics are a launch mode, not a second program.
8. HTML export translates the decoded WinHelp semantic model to browser-native document flow; ordinary prose is never corrected by moving or scaling individual browser-shaped words.

## 0.1.0 - Container foundation

- [x] Initial parser/render/UI separation established; consolidated to two crates in 0.4.2.
- [x] HLP header, internal `FILEHEADER`, B+ tree directory, internal streams.
- [x] `|SYSTEM`, generation/compression metadata, title/copyright/CNT/charset.
- [x] CONFIG macro retention without execution.
- [x] Native wxDragon shell and initial diagnostic utility.

## 0.2.0 - Topic decoder

- [x] HC30/HC31+/HCW physical `|TOPIC` blocks and transformed TOPICPOS space.
- [x] Bounded WinHelp LZ77 and cross-block TOPICLINK reads.
- [x] Classic HC30/HC31/MVB phrases and WinHelp 4 Hall phrases.
- [x] HC30 relative and HC31+ absolute link traversal.
- [x] Topic headers/titles/macros.
- [x] Fixed-vs-scrolling record classification and plain-text reconstruction.
- [ ] Broader legal/user-supplied real-HLP regression corpus.

## 0.3.0 - First native renderer

- [x] Parse old and modern `|FONT` tables and historical metric variants.
- [x] Decode variable-length display/table paragraph metadata from LinkData1.
- [x] Decode font changes, line/tab commands, picture placeholders, macros, internal/external hotspots.
- [x] Preserve table column metadata and paragraph/cell association.
- [x] Recover safely from unknown per-record formatting commands.
- [x] Produce retained GUI-independent `LayoutBox` geometry.
- [x] Wrap text, tabs, indentation, paragraph spacing/alignment, tables, and borders.
- [x] Match the verified Microsoft KB917607 paragraph path for DPI/144 metrics, default/custom tabs, right/center tab alignment, signed line spacing, no-wrap, and alignment value 3.
- [x] Match the classic Microsoft font path for exact RGB(1,1,0) colour inheritance, opaque run backgrounds, half-point sizes, and two-thirds-height small caps.
- [x] Match the verified Microsoft border style code and per-side 5/6/7-pixel content clearance, including double and shadow borders.
- [x] Match Microsoft type-0x04/0x23 table geometry: 32-column cap, type-0 minimum width and 32767-unit proportional scaling, nonzero absolute DPI/144 metrics, width/gap ordering, and independent per-column vertical flow.
- [x] Decode Microsoft compact table-cell framing: signed column + bounded nested TOPICLINK record, including old/modern display cells, fixed `0x02`/`0x21` topic-header size forms, exact payload boundaries, and `-1` termination.
- [x] Retain/render recursively nested `0x04`/`0x23` table records through the same cell-tree/layout path, including parent-cell width/origin propagation and returned-height advancement.
- [x] Implement the compact non-display cell families proven by KB917607: `0x03`/`0x22` graphics, `0x05`/`0x24` hosted-control placeholders, and old `0x06` no-render dispatch.
- [x] Identify paragraph bit 13 as the Microsoft right-to-left flag and apply its confirmed right-side first-line-indent behavior.
- [x] Decode legacy Windows SBCS/DBCS charsets (including Hebrew/Arabic and the common Greek/Cyrillic/CJK families), reproduce the KB917607 locale/face-charset run-reordering path, and retain native glyph shaping.
- [x] Preserve the final two raw border bytes as render-inert metadata after tracing the reference clearance and painter consumers; do not invent semantics.
- [x] Keep fixed/non-scrolling and scrolling regions separate.
- [x] Native wxDragon PaintDC topic surfaces and scrolling body.
- [x] Hyperlink rectangle hit testing.
- [x] HTML export translates decoded `FormattedRecord` / `ParagraphFormat` semantics directly to browser-native paragraphs, inline runs, anchors, tabs, tables, pictures, and controls; ordinary prose uses browser flow with no retained-word repositioning or glyph fitting.
- [x] Deterministic renderer tests independent of wxWidgets.
- [ ] Cache wxDragon native fonts across paint events.
- [x] Substitute modern platform text faces while retaining HLP size/style/color/fixed-pitch semantics.
- [ ] Validate visual metrics against a broad real-HLP screenshot corpus.

## 0.4.1 - Hypertext/navigation

- [x] Parse HC30 `|TOMAP`.
- [x] Parse HC31+/HCW `|CONTEXT` and WinHelp context hashes.
- [x] Parse `|CTXOMAP`, `|TopicId`, and `|Viola`.
- [x] Parse standard HC31/HCW SYSTEM WINDOW definitions.
- [x] Reconstruct `TOPICOFFSET` anchors from topic blocks/TopicLength fields.
- [x] Resolve TOPICOFFSET jump targets to reconstructed topics.
- [x] Context-ID/context-name APIs in the document layer.
- [x] Internal jump links.
- [x] Popup-topic links.
- [x] Browser-style Back/Forward history.
- [x] History restoration across HLP files.
- [x] Browse previous/next sequences for HC30 and HC31+.
- [x] Contents-topic resolution for HC30 and HC31+.
- [x] Cross-file HLP links with explicit relative-path rules.
- [x] External popup-vs-jump opcode distinction.
- [x] Named/numbered/default secondary help windows.
- [x] Merge diagnostic utility into `hlp-viewer.exe --dump-file`.
- [x] `--verbose`, `--help`, `--version`, and positional `file.hlp` startup modes.
- [x] Avoid wxDragon initialization on command-line diagnostic paths.
- [x] Preserve the Cargo/wxWidgets cache during normal cleaning.
- [x] Refine popup auto-dismiss/focus behavior with owned transient frames, activation loss, Escape dismissal, and hotspot-relative placement.
- [x] Apply parsed WINDOW position, maximize, topmost, caption, region-color, and auto-height behavior to native secondary windows.
- [x] Make hyperlinks inside popup and secondary windows fully interactive, including cross-file and explicit-window targets.
- [ ] Validate legacy WINDOW coordinate scaling and edge-of-screen popup placement against a broad real-HLP corpus.

## 0.4.2 - Architecture consolidation

- [x] Merge parser, document/navigation, and retained-layout packages into one GUI-independent `hlp` crate.
- [x] Keep `hlp-viewer` as the only wxDragon-dependent crate.
- [x] Preserve `cargo test -p hlp` as the fast non-wxWidgets test path.
- [x] Keep a single application executable and integrated `--dump-file` mode.
- [x] Consolidate viewer CLI/console/dump helpers without growing `main.rs`.
- [x] Preserve all 0.4.1 runtime behavior while removing Cargo-package plumbing.

## 0.4.3 - Viewer regression fixes

- [x] Separate direct-file startup from the explicit WinHelp Contents command.
- [x] Use wxWidgets native text extents for retained layout in the GUI while retaining deterministic headless metrics for engine tests.
- [x] Restore Alt+Left / Alt+Right Back/Forward handling independently of menu accelerator parsing.
- [ ] Validate native wrapping/spacing against additional real HLP files and DPI settings.

## 0.4.4 - Direct topic keyboard navigation

- [x] Make plain Left/Right select the physically adjacent decoded topic index rather than the authored browse sequence.
- [x] Preserve Alt+Left/Alt+Right as Back/Forward history.
- [x] Bind native key handling to the frame and focused topic surfaces.
- [x] Consume handled arrow events so wxWidgets does not also scroll/navigate.

## 0.4.5 - Navigation focus/menu correction

- [x] Make the main topic canvases explicitly focusable.
- [x] Restore focus to the scrolling topic surface after document/topic changes and main-surface clicks.
- [x] Replace viewer-facing Browse Previous/Next with physical Previous Topic/Next Topic commands.
- [x] Remove Ctrl+PageUp/Ctrl+PageDown from the Navigate menu.
- [x] Retain authored browse-sequence parsing and resolution inside `hlp` for compatibility and safe macro browse commands.

## 0.5.0 - Raster graphics

- [x] Decode indexed `|bmN` and embedded WinHelp logical graphics objects.
- [x] Decode DIB bitmap alternatives at 1/4/8/16/24/32 bpp and portable DDB variants.
- [x] Support raw, RLE, LZ77, and LZ77-then-RLE WinHelp graphics packing.
- [x] Preserve palette transparency and convert decoded pictures to GUI-independent RGBA.
- [x] Render raster pictures natively through wxDragon and proportionally shrink oversized images.
- [x] Reuse indexed decoded pixel buffers and enforce per-image decode/allocation bounds.

## 0.5.1 - Graphics completion

- [x] Decode and rasterize legacy WMF alternatives on Windows through an isolated GDI adapter.
- [x] Parse SHG/MRB graphical hotspot tables and make scaled image regions clickable.
- [x] Resolve graphical popup/jump/named-window targets through existing context/window navigation.
- [x] Keep graphical macro targets inert under the same blocked-macro policy as text hotspots.
- [x] Implement `bml`/`bmr` left/right paragraph floats with text wrap and full-width release below the image.
- [x] Correct text external-link opcode parity to the WinHelp low-bit popup/jump rule.
- [ ] Add a bounded total-document/native-bitmap cache only if real-world profiling justifies it.
- [ ] Validate bitmap, WMF, image-hotspot and float placement against a broader legal/user-supplied HLP screenshot corpus.

## 0.6.0 - Contents, index, and search

- [x] `.CNT` parser, `|SYSTEM`/same-basename discovery, hierarchy, and `:Base`/`:Index`/`:Link` metadata; build-fix 46 adds same-basename `.GID` fallback from cached `|CntText`/`|CntJump`/`|Flags` data.
- [x] Native hierarchical Contents tree with active-topic synchronization; build-fix 45 makes physical topic order an explicit **Show all** mode, and build-fix 46 preserves hierarchy from a usable GID when the CNT is missing.
- [x] Author-defined keyword index (`|?WBTREE`/`|?WDATA` families), including multiple topic targets.
- [x] Incremental deterministic in-memory full-text index over title, keyword, and decoded topic body text.
- [x] Unified Contents / Index / Search navigation pane.
- [x] One-hop cross-file Index/Search catalogs for `.CNT` `:Index`/`:Link` help files and build-fix 46 GID `|FILES` equivalents.
- [x] Session bookmarks and visible Back/current/Forward history UI.

## 0.6.1 - Hyperlink destination tooltips (completed)

- [x] Native tooltip hover handling on fixed and scrolling main-topic regions.
- [x] Destination-title resolution for same-file text hyperlinks.
- [x] Cross-file title resolution through the existing relative HLP-link resolver.
- [x] Popup destinations explicitly labeled as popup topics.
- [x] Graphical/image hotspots use the same retained hit-test rectangles and tooltip resolver.
- [x] Popup and secondary help windows expose identical tooltip behavior.
- [x] Ordinary text and blocked executable macro hotspots remain tooltip-free.
- [x] Tooltip state changes only when the hovered hotspot changes and is cleared on pointer leave.

## 0.6.2 - Browsing toolbar and navigation pane control

- [x] Native wxDragon/wxWidgets browsing controls with a compact in-window strip.
- [x] Keep Back/Forward history and Contents commands in the Navigate menu/keyboard paths; build-fix 35 removes their redundant visible buttons.
- [x] Physical Previous/Next topic commands reuse the existing menu/navigation paths and remain visible on the strip.
- [x] Visible command availability follows document presence and first/last physical topic boundaries.
- [x] Navigation control shows/hides the entire discovery side panel.
- [x] `View > Navigation Pane` with `F9` exposes the same side-panel visibility command.
- [x] Topic layout reflows when the side panel is hidden or restored, without changing HLP navigation state.

## 0.7.0 - Safe macro compatibility (completed)

- [x] Macro tokenizer/parser and typed `HelpMacro` AST with bounded program/string/argument/nesting limits.
- [x] Allow-listed navigation/UI macros dispatched through existing main/popup/secondary navigation paths.
- [x] SYSTEM CONFIG, per-topic, and hotspot macro execution under one default-deny policy.
- [x] Authored `BrowseButtons` toolbar controls kept distinct from physical Previous/Next topic navigation.
- [x] Explicit blocked result for process/shell execution, DLL invocation, host interaction, malformed invocations, and unknown unsafe operations.
- [x] Shared command budget to stop cyclic macro navigation and bounded diagnostic retention.
- [x] Runtime macro diagnostic log plus `--dump-file --verbose` ALLOW/BLOCK classification.
- [x] Popup colour macro constrained to viewer-owned popup rendering.

## 0.7.1 - Context-hash hotspot compatibility (current)

- [x] Distinguish physical `TOPICOFFSET` text hotspots (`0xE0`/`0xE1`) from context-hash hotspot families (`0xE2`/`0xE3`/`0xE6`/`0xE7`).
- [x] Resolve context-hash hotspots through `|CONTEXT` before navigation.
- [x] Preserve a TOPICOFFSET fallback for unusual/nonstandard producers.
- [x] Apply the fix consistently to main windows, popups, secondary windows, status descriptions, and destination tooltips.
- [x] Add parser regression coverage for visible and non-emphasized context-hash links.
- [x] Recover displayable records outside inconsistent topic-header visual ranges into the scrolling presentation so correctly resolved popup topics cannot become blank solely because their records were classified as `unclassified`.

## 0.8.0 - Printing and polish

- [x] Native Windows printing with retained topic formatting and topic-range selection.
- [x] Text selection/copy, native clipboard Copy/Paste for query fields, and Select All.
- [ ] Keyboard navigation/accessibility labels.
- [ ] DPI-change reflow, font-object caching, and broader fallback/accessibility polish.
- [ ] Persistent settings and packaging.

## 1.0 compatibility target

A standalone viewer that opens a broad set of Windows 3.x/95 HLP files and preserves their documentation behavior without Microsoft's removed WinHelp runtime. A separate WinHelp API compatibility shim can then translate legacy application requests into the same internal navigation model rather than duplicating HLP logic.
### Build-fix 14 transient UI and table-visibility validation

- [x] Confirm from Microsoft WinHlp32 that table records provide geometry/flow only and do not synthesize a visible grid.
- [x] Preserve paragraph-border rendering as the sole source of table-area rules unless a cell's paragraph explicitly carries borders.
- [x] Dismiss destination-title hover tooltips on click and invalidate their cache across topic changes.
- [x] Track one active transient popup-topic frame and close it on outside main-window clicks or main-topic navigation.
- [x] Keep secondary help windows persistent.


### Build-fix 15 paragraph-rule fidelity

- [x] Preserve negative/hanging `first_line_indent` through final line alignment instead of clamping it back to the ordinary paragraph left edge.
- [x] Apply `first_line_indent` only to the first visual line; wrapped continuation lines use the ordinary paragraph indent.
- [x] Stop border-only paragraphs from acquiring a synthetic 16-pixel blank text row.
- [x] Keep authored top/bottom rules as paragraph borders rather than replacing them with synthetic table grid lines.
- [x] Keep discovered `.CNT` contents authoritative when present; otherwise accept a verified same-basename `.GID` cached hierarchy. Physical topic order is available only through explicit **Show all**.
- [x] Add headless regressions for hanging indentation and zero-text border-rule height.

### Build-fix 16 single-surface UI and Related Topics separator

- [x] Remove native floating popup/secondary topic frames from all reachable viewer behavior.
- [x] Route popup hotspots, external popup opcodes, secondary-window destinations, `.CNT` window qualifiers, and popup macros into the single main help surface while preserving destination resolution and navigation history.
- [x] Render compact border-only top+bottom separator paragraphs as one horizontal rule rather than two visible edges.
- [x] Align that separator's left edge to the following unindented `Related Topics` heading, preserve its authored right-side inset, and reserve a 12-pixel gap below the rule.
- [x] Give the visible browse toolbox explicit 5-pixel vertical margins, 4-pixel pair gaps, and 10-pixel group gaps with consistent control widths.
- [x] Extend headless layout coverage for separator geometry/alignment while preserving build-fix 15 hanging-indent coverage.

### Build-fix 17 executable-format audit and compact special renderers

- [x] Remove the false `0x20`/`0x21` VariableField/DType character-command interpretation and the unverified `0x8B`/`0x8C` character branches after direct KB917607 scanner tracing.
- [x] Decode real character control `0x85` and, after build-fix 39 tracing, apply its signed WORD as the glyphless transient horizontal line-origin override used by alignment.
- [x] Render compact/top-level `0x03`/`0x22` graphics through the existing indexed/embedded graphics pipeline.
- [x] Retain `0x05`/`0x24` hosted-control metadata without executing native authored code; provide a bounded placeholder and preserve surrounding layout.
- [x] Represent old `0x06` explicitly as the no-render compact dispatcher case.
- [x] Preserve the complete `C0..CF` / `E0..EF` hotspot envelopes on WinHlp32's exact fixed/variable boundaries; build-fix 39 additionally verifies that residual variants have no activation-dispatch branch in the audited runtime and are intentionally inert.
- [x] Treat paragraph-border trailing bytes and paragraph flag bit 0 as retained render-inert metadata in the traced path; keep styles 5-7 reserved with zero clearance and no invented paint.
- [x] Remove the speculative MVB 42-byte style/character-map descriptor model: KB917607 uses 11-byte descriptors for every generation and 20/32-byte face slots.
- [x] Parse `|SYSTEM` locale/per-face charset metadata, decode the common Windows SBCS/DBCS families, and reproduce charset-run reordering; native glyph shaping remains host-renderer behavior.
- [x] Reproduce deterministic GDI-style face/locale charset inference for the common legacy Windows families when record 11 is absent/default, plus CJK no-space wrapping.
- [x] Characterize and implement Johab (`0x82`) as Windows CP1361; classify `OEM_CHARSET` (`0xFF`) as deliberately host-selected through GDI rather than a fixed HLP code page, using active `CP_OEMCP` on Windows.
- [x] Characterize the downstream semantic effect of `0x85`: it overwrites the transient horizontal line origin and therefore participates in remaining-space/alignment calculations without emitting a glyph.
- [x] Resolve the structurally supported uncommon hotspot variants for the audited KB917607 runtime: the activation dispatcher has no branch for them, so they are verified inert rather than unknown navigation actions.
- [x] Trace authored hosted-control sizing: arbitrary controls are created at `2*LOGPIXELSX` by `2*LOGPIXELSY`, then negotiate final dimensions through private message `0x706B` or fall back to `GetWindowRect`; safe mode mirrors the verified initial rectangle without loading document-supplied native controls.


### Build-fix 18 tooltip restoration without floating windows

- [x] Restore destination-title native tooltips on the main fixed and scrolling help canvases from the pre-build-fix-16 implementation.
- [x] Restore authored popup distinction in hover/status metadata (`Popup: <title>` / `Popup link: ...`).
- [x] Keep popup/secondary activation routed into the single main help surface; do not restore auxiliary `Frame` creation.
- [x] Preserve internal TOPICOFFSET, context-hash, and cross-file tooltip destination resolution; keep macro hotspots tooltip-free.
- [x] Preserve tooltip cache-generation invalidation and click-to-clear behavior across topic/document navigation.
- [x] Leave build-fix 17's executable-derived character-command semantics unchanged.
- [x] Add pure regression coverage for ordinary and popup destination-tooltip labels.

### Build-fix 20 popup-content hover correction

- [x] Replace synthetic `Popup: <topic title>` hover labels with the resolved popup topic's actual visible text.
- [x] Derive popup hover content from formatting-decoded `TopicPresentation` records so paragraph boundaries, line breaks, tabs, and formatting fallbacks track the renderer's source text.
- [x] Preserve ordinary destination-title hover behavior and internal/context-hash/cross-file resolution.
- [x] Fall back to the destination title only for genuinely textless popup topics.
- [x] Keep macro hotspots tooltip-free and preserve build-fix 16 single-surface click routing.
- [x] Add regression tests for popup-body selection, CR/LF normalization, presentation extraction, and empty-body fallback.

### Build-fix 21 inline hosted controls and recent documents

- [x] Re-trace CALC.HLP's `Related Topics` object against the retained 285,696-byte KB917607 WinHlp32 reference rather than assuming inline `0x86`/`0x87`/`0x88` means picture.
- [x] Dispatch the nested inline compact TOPICLINK by record type: graphics `0x03`/`0x22`, hosted/custom windows `0x05`/`0x24`, and old no-render `0x06`.
- [x] Retain CALC's exact hosted descriptor and reproduce the verified final 12x12 empty-label BUTTON geometry without executing the authored macro.
- [x] Preserve ordinary inline-picture decoding and modern compact TopicLength framing with dedicated regressions.
- [x] Add **File > Recent Documents** with a five-document MRU list.
- [x] Persist the MRU list in `hlp-viewer.cfg` beside the running viewer executable, with no per-user config-directory fallback.
- [x] Add parser/layout/config regression coverage and keep malformed or missing config non-fatal.

### Build-fix 22 portable executable-local configuration

- [x] Store `hlp-viewer.cfg` in the same directory as the running viewer executable on every platform.
- [x] Remove `%LOCALAPPDATA%`, `%APPDATA%`, XDG, home-directory, and working-directory fallbacks for recent-document persistence.
- [x] Fail configuration load/save non-fatally when the executable location cannot be resolved or its directory is not writable.
- [x] Add regression coverage tying `config_path()` directly to `std::env::current_exe().parent()`.

### Build-fix 23 MRU compile correction

- [x] Explicitly type the recent-document parser accumulator as `Vec<PathBuf>` so `Path` is not inferred as an unsized vector element through `same_path`.
- [x] Remove the unused `PanelStyle` import from the viewer front end.
- [x] Preserve build-fix 22 executable-local config persistence and all existing rendering/navigation behavior unchanged.

### Build-fix 24 table-cell empty-record alignment

- [x] Re-trace WinHlp32's empty display-record fast path at `0x415B44..0x415BA1`.
- [x] Treat completely empty, unbordered table-cell display paragraphs as zero-height structural fillers rather than synthetic blank text lines.
- [x] Preserve independent per-column cursor advancement instead of introducing a non-native shared row model.
- [x] Add an 8-pixel gap only when a rule-only display record is immediately followed by a table.
- [x] Add headless regressions for column-baseline alignment and post-rule table spacing.


### Build-fix 25 mixed-font baseline alignment

- [x] Align mixed-height text runs on one retained baseline instead of top-aligning every run at `line.y`.
- [x] Keep pictures, hosted controls, borders, wrapping, indents, and paragraph spacing outside the adjustment.
- [x] Preserve RTL run reordering before the vertical baseline adjustment.
- [x] Add regression coverage for a smaller bullet-like glyph beside larger body text.

### Build-fix 26 refined bullet/text vertical alignment

- [x] Replace full bottom-alignment of mixed-height text runs with an ascent-based retained baseline approximation.
- [x] Keep the correction limited to text runs so pictures/hosted controls and paragraph geometry do not move.
- [x] Update the headless regression to assert the refined bullet/body relationship.

### Build-fix 27 list-marker alignment tuning

- [x] Increase the retained ascent estimate from 3/4 height to 5/6 height for mixed-height text baseline alignment.
- [x] Keep the tuning limited to text runs so non-text layout geometry remains unchanged.
- [x] Update the headless regression to assert the tuned bullet/body relationship under the retained metric model.

### Build-fix 28 native font-baseline retention

- [x] Extend retained text metrics with an explicit baseline offset instead of inferring vertical alignment from cell height.
- [x] Preserve wxWidgets/GDI descent from `get_full_text_extent()` and derive the native baseline as text height minus descent.
- [x] Carry that baseline in each retained text box and align mixed-face runs from the real metric during line finalization.
- [x] Add an equal-height/different-baseline regression matching the CALC.HLP failure mode that made build-fix 25-27 ineffective.

### Build-fix 29 inline bitmap marker baseline

- [x] Decode the supplied CALC.HLP marker records far enough to prove the square/triangle bullets are `0x86` -> `0x22` indexed graphics (`|bm0` 3x7 and `|bm1` 4x8), not text runs.
- [x] Include inline pictures in line baseline finalization using the picture bottom as the object baseline.
- [x] Shift transparent picture-hotspot overlays by exactly the same vertical delta as their owning picture.
- [x] Keep floating pictures outside this inline-baseline adjustment.
- [x] Add an integration regression reproducing a 3x7 CALC-style marker beside body text with an explicit baseline.


### Build-fix 31 Related Topics rendered-rule anchoring

- [x] Remove build-fix 30's ALink-only `left_indent` / `first_line_indent` zeroing and restore ordinary signed paragraph-metric handling.
- [x] Retain the actual rendered `Border` x-coordinate for a preceding rule-only top-level record.
- [x] After ordinary layout, translate the stock 12x12 ALink button and text on its visual line to that saved rule edge.
- [x] Keep the existing 4-pixel vertical separation and safe ALink macro dispatch unchanged.
- [x] Regress against a non-margin rule x-coordinate and separately prove authored ALink indents survive when no rule anchor is present.


### Build-fix 32 native DPI and fractional font sizing

- [x] Carry independent horizontal/vertical DPI through retained layout rather than assuming one 96-DPI axis.
- [x] Use horizontal device DPI for paragraph indents, tabs, table geometry, and fallback text width.
- [x] Use vertical device DPI for paragraph/line spacing and fallback font height.
- [x] Read the Windows canvas `LOGPIXELSX` / `LOGPIXELSY` and construct the retained layout with those values.
- [x] Keep authored/zoomed HLP font sizes in twips through native Windows font creation, avoiding early whole-point rounding.
- [x] Use one GDI `LOGFONTW` definition for both text measurement and text painting so wrapping/baselines/hotspots follow the font actually drawn.
- [x] Preserve wxDragon whole-point font creation only as the portable/failure fallback and retain `LayoutEngine::new(dpi)` for square-DPI callers/tests.
- [x] Apply the same device-DPI context to authored bitmap physical-resolution and WMF natural-size conversion (build-fix 33).

### Build-fix 33 graphics physical sizing

- [x] Preserve nonzero bitmap x/y resolution metadata from type `0x05`/`0x06` alternatives.
- [x] Compute authored bitmap natural dimensions as pixel extent times target device DPI divided by authored axis resolution.
- [x] Convert WMF physical mapping modes with independent target x/y DPI instead of a fixed 96-DPI display assumption.
- [x] Keep bounded WMF decoding/rasterization independent from retained display geometry and reuse the existing picture/hotspot proportional-scaling path.
- [x] Add asymmetric-DPI regressions for bitmap and WMF natural sizing.

### Build-fix 34 international charset compatibility

- [x] Decode common Windows SBCS families: Central/Eastern European, Cyrillic, Greek, Turkish, Vietnamese, Baltic and Thai, retaining the existing Western/Hebrew/Arabic paths.
- [x] Decode the common Windows DBCS families used by Japanese, Korean, Simplified Chinese and Traditional Chinese help files.
- [x] Apply explicit per-face record-11 charset metadata first; infer common legacy charsets from face name and `LANGID` only for absent/default metadata.
- [x] Add CJK break opportunities between ideographic/kana/hangul units while keeping Latin words intact and simple opening/closing punctuation attached.
- [x] Add parser/decoder/layout regressions for SBCS, Shift-JIS, GBK, Big5, locale inference and no-space CJK wrapping.
- [x] Characterize Johab as Windows CP1361 and implement a deterministic decoder; document historical OEM selection as host-GDI-defined and use the active Windows `CP_OEMCP` rather than aliasing it to an unrelated fixed code page.

### Build-fix 39 residual executable audit

- [x] Recovered and hash-verified the 285,696-byte KB917607 `winhlp32.exe` reference used by build-fix 17.
- [x] Implement Windows CP1361/Johab decoding for charset `0x82`; retain OEM charset `0xFF` as a documented host-GDI boundary.
- [x] Preserve authored non-ANSI/default face names on Windows so legacy charset runs reach `CreateFontIndirectW` with the same face/charset pair instead of being pre-substituted by the viewer's modern Western-font policy.
- [x] Implement the verified `0x85` horizontal line-origin state change.
- [x] Reclassify structurally accepted residual hotspot opcodes as KB917607-inert after tracing the click dispatcher.
- [x] Replace the invented generic hosted-control placeholder dimensions with the reference's two-device-inch pre-negotiation creation rectangle while retaining the default-deny native-control policy.
- [x] Retitle and revise the consolidated format manual with the new confidence classifications and executable addresses.

### Build-fix 35 zoom/restore/UI cleanup

- [x] Carry viewer text zoom separately from device DPI and scale signed `spacing_lines` advances so enlarged native glyphs retain proportional vertical pitch.
- [x] Reflow on the content host's actual size event after wxWidgets sizer propagation, fixing stale maximized widths after restore.
- [x] Explicitly invalidate the page host/border/background when resize does not otherwise require a retained-layout rebuild.
- [x] Remove Contents and Back/Forward-history buttons from the visible browse strip and hidden toolbar while preserving menu/keyboard navigation.
- [x] Remove the macro milestone/diagnostics prose from the About dialog.

### Build-fix 30 Related Topics / ALink support

- [x] Align the standard Related Topics ALink control row to the authored separator's left edge rather than preserving its negative hanging indent.
- [x] Add a 4-pixel post-separator gap before the Related Topics row.
- [x] Allow-list classic `ALink` / `AL` with one string argument under the existing default-deny macro policy.
- [x] Resolve semicolon-delimited ALink names exactly against `|AWBTREE` / `|AWDATA`, deduplicate resolved TOPICOFFSET targets, and present multiple targets in a native Topics Found chooser.
- [x] Make built-in `!label,macro` hosted buttons clickable viewer-local macro hotspots while retaining the same security dispatcher.
- [x] Add parser/index/layout regressions for CALC-style associative links and Related Topics geometry.


### Build-fix 48 — printing

Implemented native Windows printing for the current topic through File > Print... / Ctrl+P, with printer-width word wrapping, automatic page breaks, cancellation handling, and a portable non-Windows fallback. A future fidelity pass may route the full retained visual layout (pictures, borders, tables, exact fonts) to printer device contexts.

### Build-fix 49 — printing compile fix

Build-fix 49 corrects the build-fix 48 native Windows printing implementation for the workspace's strict unsafe-code lint. The Win32 printer FFI remains isolated and explicitly permitted at the smallest practical item scopes; the crate-wide policy remains unchanged. Print error/information dialogs now build the wxDragon `MessageDialog` before showing it modally.

### Build-fix 56 — formatted printing and topic ranges

Completed the printing fidelity pass that build-fix 48 left open. The printer backend now runs the formatting-decoded `TopicPresentation` through `LayoutEngine` using the selected printer HDC's native DPI/text metrics, then paints retained text styles, authored colours/backgrounds, pictures, borders, table/paragraph geometry, and safe embedded-control placeholders directly to paginated printer pages. File > Print also gains current-topic / topic-range / all-topics selection; range syntax is one-based and accepts comma/semicolon-separated inclusive ranges with overlap de-duplication and validation.

### Build-fix 59 — explicit Contents inline overflow tip

- [x] Stop relying on the TreeCtrl backend to display a clipped-label tooltip automatically; the build-fix 58 Windows runtime capture showed that no reveal appeared.
- [x] Detach the tree view's automatically-created tooltip association with `TVM_SETTOOLTIPS` so there can be only one Contents overflow tip.
- [x] Reuse the same pre-coloured row-anchored `NativeInlineOverflowTooltip` used by Index/Search/Bookmarks/History.
- [x] Use TreeCtrl hit testing plus the text-only item rectangle to show the complete label only when it is horizontally clipped, on the same line as the row.
- [x] Preserve the existing `RGB(249,249,158)` (`#F9F99E`) information palette and all formatted-printing/topic-range behavior.

### Build-fix 58 — single inline navigation overflow tips

- [x] Remove the build-fix 57 second wxToolTip from Windows Contents and keep the native TreeCtrl clipped-label tip as the reference behavior.
- [x] Give Index, Search, Bookmarks, and History one shared native tracking-tooltip implementation positioned at the hovered row origin rather than at the mouse cursor.
- [x] Keep overflow reveals single-line, conditional on actual clipping, and pre-coloured to the existing WinHelp information palette without recolour/retry workers.
- [x] Preserve the portable non-Windows fallback and all formatted-printing/topic-range behavior.

### Build-fix 67 — custom positioned overflow popup

- [x] Replace the Windows navigation `tooltips_class32` overflow reveal with a viewer-owned popup window whose screen position is fully explicit.
- [x] Paint the popup with the source control's actual native font, RGB(249,249,158) background, black text, and a one-pixel border.
- [x] Align the popup's *text origin* to the cropped label text origin instead of aligning the tooltip window rectangle or mouse cursor.
- [x] Use one shared ListBox geometry path for Index, Search, Bookmarks, and History and the TreeCtrl text-only bounds for Contents.
- [x] Make the popup no-activate and hit-test transparent so it cannot steal focus/mouse ownership from the row it overlays.
- [x] Preserve overflow-only visibility, UTF-16/GDI clipping measurement, the 350 ms initial hover delay, and the portable wxToolTip fallback.

### Build-fix 63 — ordinary overflow-tooltip lifecycle

- [x] Replace the tracking-tooltip path with `TTF_IDISHWND | TTF_SUBCLASS` so Windows owns hover timing and cursor-relative placement.
- [x] Share one cached `OverflowTooltip` driver across Contents, Index, Search, Bookmarks, and History, with a wx fallback if native creation fails.
- [x] Measure ListBox labels with the real control font through `WM_GETFONT` + GDI and retain wxDragon extent only as a fallback; measurement failure reveals rather than suppresses.
- [x] Use the TreeCtrl text-only bounding rectangle for indentation-aware Contents clipping and keep `TVM_SETTOOLTIPS` detachment to prevent duplicate native tips.
- [x] Remove row anchoring, tracking activation/position state, and the synthetic mouse-leave workaround.

### Build-fix 62 — synthetic mouse-leave correction

- [x] Keep the build-fix 61 shared tracking-tooltip registration and activation contract intact.
- [x] Recognize that a same-row tracking tooltip overlays the TreeCtrl/ListBox and can trigger `WM_MOUSELEAVE` without the physical cursor leaving the navigation widget.
- [x] Gate Windows tooltip dismissal on the real cursor position transformed into the control client rectangle.
- [x] Apply the correction once in the shared ListBox/TreeCtrl overflow behavior without restoring duplicate wxToolTip paths or per-tab implementations.

### Build-fix 61 — shared tracking-tooltip contract correction

- [x] Restore the containing window as `TOOLINFO.hwnd` and keep the child TreeCtrl/ListBox HWND in `uId` under `TTF_IDISHWND`, matching Microsoft's child-control tooltip contract.
- [x] Activate tracking tooltips before sending `TTM_TRACKPOSITION`, matching the documented common-control sequence.
- [x] Register the hidden tooltip with a stable UTF-16 placeholder string, then update the same retained buffer before activation.
- [x] Keep one shared implementation for Contents, Index, Search, Bookmarks, and History; preserve same-row placement and RGB(249,249,158).

### Build-fix 60 — shared navigation tooltip registration fix

- [x] Register the shared Windows overflow tracking tooltip as a window tool with `TTF_IDISHWND`.
- [x] Use the actual TreeCtrl/ListBox HWND as `TOOLINFO.uId`, matching the Win32 contract for window-backed tools.
- [x] Own each tracking tooltip from the navigation control itself so `TTF_TRANSPARENT` forwards mouse messages back to the correct control rather than its container panel.
- [x] Keep one common `NativeInlineOverflowTooltip` path for Contents, Index, Search, Bookmarks, and History; keep empty wxToolTip clearing only in the non-Windows fallback.
- [x] Preserve row-aligned single-line placement, overflow-only visibility, `#F9F99E` colour, formatted printing, and HLP hotspot-tooltip behavior.

### Build-fix 57 — navigation overflow tooltips

- [x] Apply overflow-only hover tooltips to Contents, Index, Search, Bookmarks, and History rather than special-casing Bookmarks.
- [x] Use TreeCtrl hit testing plus the visible label origin for clipped hierarchical Contents rows.
- [x] Share the existing native Windows list-box row hit test across Index/Search/Bookmarks/History, preserving correct rows after vertical scrolling.
- [x] Keep tooltips absent for labels that fit, and clear stale tooltip text whenever a navigation model is rebuilt.
### Build-fix 50 — resizable navigation and bookmark overflow tooltips

- [x] Replace the fixed-width navigation/body sizer boundary with a native vertical wxDragon splitter using live sash updates.
- [x] Keep navigation tabs and document browse controls in separate vertical columns so toolbar centering follows the right pane automatically.
- [x] Preserve F9/navigation-toggle behavior while remembering and restoring the last dragged pane width.
- [x] Route macro-driven Index, Search, Bookmarks and History visibility through the same splitter-aware helper.
- [x] Show the full bookmark label as a tooltip only when the hovered row is clipped; use native Windows list-box hit testing so vertically scrolled rows resolve correctly.
- [x] Preserve the classic tooltip palette already used by help hotspots.
### Build-fix 51 — WinHlp information-yellow match

Completed:

- sampled the retained WinHlp reference capture: topic page `RGB(255,255,228)` and popup/tooltip interior `RGB(255,255,225)`;
- unified native tooltip and legacy popup-note fallback painting on `RGB(255,255,225)` (`#FFFFE1`);
- removed the previous `RGB(255,255,184)` saturated popup fallback;
- retained black information text and build-fix 50 navigation behavior unchanged.


### Build-fix 52 — darker WinHelp tooltip yellow

Completed:

- changed the shared tooltip/popup information surface to the user-sampled `RGB(249,249,158)` (`#F9F99E`);
- preserved the normal topic page at `RGB(255,255,228)` (`#FFFFE4`);
- kept native hover tooltips and legacy popup-note rendering on the same shared colour constant, with black text unchanged.

### Build-fix 53 — lazy native tooltip palette application

Completed:

- confirmed from the user capture that the page was actually `RGB(255,255,228)` while the visible tooltip remained Windows' `RGB(255,255,225)`, proving build-fix 52's `#F9F99E` constant was not reaching the native control;
- traced the failure to wxWidgets' lazy creation of the Windows `tooltips_class32` HWND after `set_tooltip()` returns;
- retained the requested `RGB(249,249,158)` (`#F9F99E`) tooltip colour and black text;
- added a bounded retry worker for dynamic HLP hotspot tooltips so the palette is applied after the actual native tooltip window exists;
- cancel superseded retry workers with a generation token when the hovered hotspot changes.
### Build-fix 54 — flicker-free native tooltip palette application

- [x] Make the Windows palette helper return whether it actually found and styled a thread-owned `tooltips_class32` HWND.
- [x] Cancel any older retry generation before the new synchronous palette attempt.
- [x] Stop the lazy-creation retry loop immediately after the first successful palette application.
- [x] Remove repeated `TTM_UPDATE` repaint forcing, which was unnecessary once the target HWND had been found and caused visible redraw pulses.
- [x] Preserve the requested `RGB(249,249,158)` (`#F9F99E`) tooltip background, black text, and unchanged `RGB(255,255,228)` (`#FFFFE4`) topic page.
### Build-fix 55 — pre-coloured native hotspot tooltip

- [x] Remove the post-creation hotspot palette retry worker entirely.
- [x] Pre-create one hidden Windows `tooltips_class32` control for each topic canvas before hover begins.
- [x] Disable its visual theme and apply `RGB(249,249,158)` (`#F9F99E`) plus black text while the tooltip is still invisible.
- [x] Register the topic canvas through `TTF_IDISHWND | TTF_SUBCLASS` so normal Windows hover timing is retained without delayed recolouring.
- [x] Keep the UTF-16 tooltip text buffer alive for as long as the native control can reference it, and use `TTM_POP` on hotspot exit/target changes to prevent stale text.
- [x] Retain the wxWidgets path only as a defensive fallback if native tooltip creation fails.

### Build-fix 68 — navigation-root preservation across external HLP jumps

- Main-window state now distinguishes the active topic document from the explicitly opened navigation root.
- Manual/startup opens replace both; cross-document jumps replace only the active topic document.
- Contents, Index, Search, related-catalog loading, Show all, and the Contents command remain rooted in the original HLP.
- History and rendering continue to track the actual active external document, so Back/Forward and chained relative links still work.
- This prevents referenced files such as COMMON.HLP from taking over WORDPAD.HLP's discovery structure.
## Optional HTML export (build-fix 69; hierarchy fidelity in 71; scripted mode in 72; semantic browser-native topic rendering in 77)

- [x] Add **File > Export to HTML...** without introducing a browser dependency into the native viewer or `hlp` engine.
- [x] Add `--export-html <source.hlp> [target.html]` as a pre-wxDragon automation mode that calls the same exporter and defaults `source.hlp` to `source.html`.
- [x] Reconcile natural browser glyph widths without distortion and add word-boundary continuation wrapping when natural text exceeds the retained line width; keep authored no-wrap overflow intact.
- [x] Distinguish semantic paragraphs, automatic layout wraps, authored hard breaks, and single-tab/list segments so browser reflow removes redundant fallback wraps instead of stacking extra continuation lines; propagate signed vertical deltas to all retained objects.
- [x] Render short isolated topic/section headings in exported HTML with a bold browser face and measure that bold face before final wrapping.
- [x] Render exported text hotspots with the classic green underlined link treatment while retaining hotspot geometry, tooltip metadata, and safe action dispatch.
- [x] Prevent browser/GDI metric drift from overlapping adjacent words without deforming glyphs: export retained baselines, keep browser shaping natural, and reposition later same-line tokens while preserving large authored positioning anchors.
- [x] Export retained topic geometry, text styling/colours, pictures, borders, hotspots, and safe hosted-control placeholders into one self-contained HTML file.
- [x] Recreate Contents / Show all / Index / Search / Bookmarks / History and navigation controls in the exported shell. Preserve usable root `.CNT`/`.GID` Contents as a collapsible authored tree; never relabel physical topic order as hierarchical Contents.
- [x] Preserve the build-fix 68 navigation-root model across embedded cross-document topics.
- [x] Translate only typed allow-listed WinHelp macros to browser-local operations; retain arbitrary controls/macros as inert or unavailable.
- [x] Bound recursive collection to relative linked HLP files and refuse automatic absolute/UNC traversal.
- [x] Keep the native gray/cream/information-yellow palette and document browser-vs-GDI fidelity boundaries.
