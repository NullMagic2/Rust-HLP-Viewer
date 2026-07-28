## 1.0 - 2026-07-27

- Uses colour-neutral grayscale antialiasing for Windows topic text. The retained GDI font path previously left `LOGFONTW.lfQuality` at `DEFAULT_QUALITY`, allowing ClearType subpixel rendering; on cream/yellow WinHelp backgrounds this produced conspicuous orange/blue fringes around glyphs in the supplied HelpScribble file. `ANTIALIASED_QUALITY` keeps the exact retained font size and GDI metrics while matching the smooth appearance of ordinary viewer text.
- Treats HelpScribble's paired black-background font descriptors as inherited only when an otherwise identical `RGB(1,1,0)` sentinel descriptor exists, removing compiler-generated black rectangles without weakening authored black backgrounds.
- Allows the constrained HelpScribble `ExecFile` Internet-link form to open only `http://` and `https://` URLs in the system browser; arbitrary executables, files and unsafe shell actions remain blocked.
- Bumps the workspace/application version to 1.0.0 / 1.0.

## v0.7.1-buildfix81

- Accepts pathnames containing spaces on the command line. A Windows shell splits an unquoted `D:\Rusty HLP viewer\Backups\HLP file examples\CALC.HLP` into six separate arguments, so `--export-html` saw a source, a target, and four extra pathnames and refused with `only one HLP pathname may be opened at startup`. The tokens that follow a path option are now rejoined with the single space that separated them.
- Finds the source/target boundary by asking the filesystem: the shortest leading run of tokens that names a real file is the source, and whatever follows it is the target. When no run matches - a mistyped path, or a target that does not exist yet - the authored `.hlp`/`.mvb` extension terminates the source instead, so the reported failure is `could not open <full path>` rather than a usage message. Correctly quoted pathnames still arrive as one argument and are used unchanged.
- Applies the same rejoining to `--dump-file` and to the positional GUI pathname. Two arguments that both name real files are still rejected as two documents rather than silently merged into one nonsense path.
- Adds the `--export-html=<source>` inline form for symmetry with `--dump-file=<file>`, and both inline forms now also accept a rejoined remainder. `--help` documents pathname quoting and the reassembly rule.

## v0.7.1-buildfix80

- Stops a hosted control from hanging in the paragraph margin. WinHelp's negative first-line indent is authored for a *text* marker - the marker sits in the hanging area and the prose that follows lines up with the paragraph's left indent - but applied to the Related Topics / `ALink` button it pulled the whole control box to the left of the rules and body text around it, by exactly the authored hanging distance. A paragraph whose first visible object is a hosted control now keeps that control on the paragraph's left edge, so it lines up with the separator rules its neighbours draw. Authored bullet and list *bitmaps* are deliberately excluded and keep hanging, since those are real markers.

## v0.7.1-buildfix79

- Adds a single exported type-scale constant, `EXPORT_BASE_FONT_PX`, defaulting to **14 px for a 10 pt authored font**. WinHlp32 draws at the host display DPI, so the verified 96 DPI reference conversion puts 10 pt at 13.333 px, which reads slightly small in a browser. Every authored `|FONT` size is now multiplied by one factor derived from that constant, so all relative size relationships in the document stay exactly as authored and the scale is tuned in one place. Paragraph indents, tab stops, spacing, and table geometry deliberately keep the unscaled 96 DPI reference conversion.
- Drops standard WinHelp buttons - Related Topics / `ALink` among them - below the surrounding text baseline so the control lines up with the rule its authoring paragraph draws beside it, instead of riding high against it. The amount is the `--control-drop` custom property (4 px by default) and applies to both labelled buttons and the classic small empty-label control.

## v0.7.1-buildfix78

- Fixes the exported topic's lost character formatting. Every run's declarations were written into a double-quoted HTML `style` attribute while the CSS font family was itself double-quoted (`font-family:"Segoe UI",...`), so the browser terminated the attribute at that quote and silently discarded **every following declaration**: authored point size, bold weight, italic, underline, strikeout, small caps, and explicit colours. Exported topics therefore rendered as one uniform browser-default face, and only the class-driven link colour and topic-title emphasis survived. Font families are now single-quoted and complete declaration lists are attribute-escaped, so decoded `|FONT` attributes reach the page as authored.
- Emits bold, italic, underline, and strikeout explicitly per run and states `font-synthesis: weight style small-caps`, so a face without a real bold/italic/small-capital cut is synthesized instead of being drawn as plain text.
- Renders the HC30 small-caps attribute properly. Authored all-capital runs such as `NUM LOCK` keep WinHlp32's verified two-thirds cell height (`0x411a59..0x411a6c`); runs that still contain lower-case characters keep their authored size and receive real CSS small-capital shaping, which is what the attribute means typographically.
- Makes the exported topic surface fluid. The topic view and each region were frozen at the export-time layout width (860 px by default) with a matching page minimum width, so a narrower window - or any text-zoom step, since CSS `zoom` multiplies that frozen width - clipped the topic instead of re-wrapping it. The authored width is now retained only as `--hlp-authored-width` for objects that carry absolute WinHelp geometry, and the shell keeps the surface sized to the visible page across window resizes, navigation-pane drags, pane toggling, and zoom changes. Only the container is measured; no glyph, word, or paragraph is measured or repositioned.
- Restores authored vertical rhythm. WinHelp adds a paragraph's space-below to the next paragraph's space-above, but adjacent CSS margins collapse to the larger of the two, so exported paragraphs sat closer together than the native viewer draws them. The exporter now carries the owed space forward into the next paragraph's top margin with a zero bottom margin, and regions and table cells each start a fresh vertical flow.
- Gives authored blank paragraphs a line box again. An empty `<p>` has no line box at all, so WinHelp's blank spacer paragraphs vanished from the export and pulled the surrounding text together; they now occupy exactly one line of the topic's own font. Regions also inherit descriptor 0 - the font WinHlp32 selects once per topic render - so an unstyled line has the authored height rather than the browser default.
- Slightly opens up default line spacing (`line-height:1.35`) for paragraphs with no authored line-spacing value, keeps explicit signed line spacing authoritative, and lets the navigation tab strip wrap so History is no longer scrolled out of view in a narrow pane.

## v0.7.1-buildfix77

- Replaces the HTML export's retained-token replay as the normal topic renderer with a **semantic HTML translation of the decoded WinHelp model**. Ordinary display records now become block-flow paragraphs and inline font/hotspot runs; the browser, rather than JavaScript, performs ordinary word wrapping and font shaping. This removes the metric-reconciliation loop that caused the build-fix 76 overlap regression.
- Translates verified `ParagraphFormat` fields directly: signed left/right/first-line indents and above/below spacing use the same `raw * 96 / 144` conversion as WinHlp32; paragraph alignment, RTL, no-wrap, authored line breaks, line spacing, and paragraph-border sides/styles become CSS semantics. Automatic line wraps are no longer exported as fixed source geometry at all.
- Translates WinHelp tabs into CSS-grid segments. The authored/custom tab positions and the reference 72-unit (48 px at 96 DPI) default tab interval determine grid columns, so numbered steps and bullet paragraphs receive a natural hanging indent and continuation lines remain inside the prose column instead of being manually repositioned.
- Translates decoded `|FONT` descriptors into ordinary inline spans/anchors with natural browser shaping, preserving size, weight/bold, italic, underline, strikeout, small-caps intent, foreground/background inheritance, family policy, and charset-aware face selection. Topic-title paragraphs are explicitly bold when their visible text matches the decoded topic title.
- Emits text hotspots as real HTML anchors and keeps them green/underlined; picture hotspots remain transparent anchors over their image. Standard hosted `!label,macro` buttons, including Related Topics/ALink, remain actionable through the existing allow-listed action dispatcher.
- Maps decoded WinHelp tables to independent HTML/CSS columns rather than synthetic rows. This follows the verified table model: each signed column has its own vertical cell flow, authored gap/width metrics use the same type-0 relative vs nonzero absolute conversion, and nested tables recurse through the same semantic renderer.
- The old retained-box helper code remains isolated for compatibility with prior tests, but normal exported topic templates no longer invoke `LayoutEngine` at all. Linked-HLP discovery now walks decoded paragraphs/tables/pictures directly as well, so HTML generation no longer depends on approximate headless font metrics. The semantic regions are also excluded from the legacy browser geometry-reconciliation pass.

## v0.7.1-buildfix76

- Replaces the HTML export's line-by-line corrective wrapping with **semantic paragraph reflow**. Each retained text token now carries paragraph, automatic-line, hard-break, tab-segment, no-wrap, and paragraph-edge metadata from the core `LayoutEngine`.
- Browser export is therefore free to discard only WinHelp's metric-dependent automatic wraps and recompute them with the browser's naturally shaped font widths. Explicit `Inline::LineBreak` boundaries remain hard breaks, and words move to a continuation line only when the next visible word would actually cross the authored paragraph right edge.
- This removes the redundant/empty-looking continuation rows seen when the deterministic fallback layout had already wrapped a paragraph and the browser added another wrap before that retained continuation. Reflow can now either add **or remove** an automatic line; later text, bullets/markers, rules, pictures/hotspots, controls, and paragraph borders receive the resulting signed vertical-flow delta.
- Preserves hanging/list indentation by treating the prose after a single tab as its own flow segment. Multi-tab, RTL, no-wrap, picture/control, and Control85 paragraphs deliberately stay on retained geometry rather than risking semantic damage.
- Adds exported heading emphasis: short, isolated non-interactive heading paragraphs (including ordinary topic headings and section labels such as “Dicas”) are rendered bold without altering link styling. Heading classification occurs before the browser's final width measurement so wrapping accounts for the bold face.
- Natural glyph shaping, green underlined hyperlinks, Related Topics/ALink behavior, hierarchical Contents, and `--export-html` remain unchanged.

## v0.7.1-buildfix75

- Fixes HTML-export vertical-flow drift after build-fix 74 corrective word wrapping. Build-fix 74 shifted later **text** baselines when a browser-only continuation line was inserted, but left non-text retained objects at their original y coordinates. That could leave list-marker bitmaps, pictures/hotspots, paragraph separators, and the stock Related Topics button one line above their associated text.
- The browser reconciliation pass now caches and restores the retained geometry of **every** direct layout box, not just text. Each inserted continuation line records a vertical-flow event; ordinary retained objects below that source baseline receive the same accumulated displacement as later text.
- Paragraph/table border boxes participate structurally rather than being blindly translated: a border below a wrap moves down, while a border that encloses the wrapped source baseline grows by the inserted line height so its top stays anchored and its bottom continues to surround the paragraph. This prevents rules from striking through moved text.
- Inline/list-marker pictures, their transparent hotspot overlays, hosted-control placeholders, and standard WinHelp buttons now stay vertically synchronized with naturally wrapped prose. The fix is idempotent across the initial layout pass and the later `document.fonts.ready` pass.
- Natural glyph rendering, word-boundary wrapping, green underlined hyperlinks, Related Topics/ALink behavior, hierarchical Contents, and scripted `--export-html` remain unchanged.

## v0.7.1-buildfix74

- Fixes the remaining HTML-export line overflow introduced by natural browser font metrics. The exporter now keeps every authored WinHelp line break, measures naturally shaped browser tokens, and inserts an additional continuation line only when the next visible word would cross the retained topic-region edge.
- Reflow remains word-boundary based: glyphs are never compressed, stretched, or split merely to satisfy the headless fallback metrics. Existing no-wrap/intentional-overflow lines are detected from retained geometry and left unwrapped.
- Added continuation-line vertical accounting. Newly wrapped lines shift later retained baselines down and grow the region height, so following paragraphs, rules, controls, and the Related Topics area do not collide with reflowed prose. Tabs, table columns, and other large authored positioning gaps remain independent anchors.
- Restores classic WinHelp hotspot visibility in exported HTML. Interactive text is explicitly dark green (`#008000`) and underlined, with a slightly darker hover/focus state, while preserving the existing action rectangle, tooltip metadata, and natural glyph rendering.
- The same wrapping and hotspot styling applies to the main topic, popup topics, and secondary-window topics, including scripted `--export-html` output.

## v0.7.1-buildfix73

- Removes build-fix 70's HTML glyph squeezing. The browser no longer changes `letter-spacing`, disables kerning/ligatures, or applies `scaleX` merely to force naturally rendered glyphs into the headless layout engine's approximate retained token widths.
- Exports each text box with its retained absolute baseline and reconciles browser/native metric differences by **position**, not glyph shape. Visible text keeps the browser's natural font shaping; subsequent boxes on the same baseline move right only when the preceding natural glyph run would otherwise overlap them.
- Preserves explicit large positioning gaps as segment anchors, so tabs, independently positioned labels, and table columns do not drift because of text earlier on the same baseline. Whitespace-only boxes retain their authored width.
- Expands an outer text/hotspot/background box when naturally rendered glyphs are wider than its retained width, keeping interactive hit areas and explicit backgrounds aligned with what the user actually sees without stretching the text.
- Re-runs natural placement after `document.fonts.ready` when supported, covering browsers that finalize a local fallback face asynchronously. Main topics, popups, and secondary-window surfaces all use the same path.
- Revalidated the exported **Related Topics** path. The classic stock control decoded by the layout engine is an `ALink` standard button; HTML export keeps its macro hotspot, resolves exact semicolon-delimited names through the HLP `A` keyword table, opens one match directly, and presents the existing Topics Found chooser when several topics match.

## v0.7.1-buildfix72

- Adds a scriptable `--export-html <source.hlp> [target.html]` launch mode. It is dispatched before wxDragon/wxWidgets initialization, so batch files, CI jobs, and conversion scripts can export help systems without ever creating the native interface.
- Reuses the exact `html_export` module and `HelpDocument`/`LayoutEngine` path used by **File > Export to HTML...**; the command-line mode does not introduce a second exporter or a simplified conversion path. Formatting, hyperlinks, tooltip metadata, pictures, safe macro actions, cross-document embedding, and build-fix 71 hierarchical Contents therefore remain identical to GUI-triggered exports.
- The source HLP becomes both the navigation root and initial active document. `.CNT`/`.GID` linked Index/Search catalogs are loaded through the same bounded relative-path policy as the native viewer before export.
- When the target is omitted, `manual.hlp` deterministically writes `manual.html` beside the source. An explicit target is accepted as the second argument and is normalized to an `.html` extension by the shared exporter. Existing files are overwritten, which makes the mode suitable for unattended scripts.
- Successful command-line exports print only the resulting HTML pathname to stdout; non-fatal unresolved-link counts go to stderr, parser/export failures return exit code 1, and command-line syntax failures retain exit code 2.
- Adds CLI parser regression tests for explicit/default targets, missing source, and incompatible `--dump-file` combinations.

## v0.7.1-buildfix71

- Preserves authored Contents hierarchy in exported HTML whenever the navigation-root HLP has usable `.CNT` or compiled `.GID` Contents data. The exporter no longer renders hierarchical mode as one permanently expanded flat list with indentation only.
- Reconstructs the same parent/child tree used by the native wxDragon `TreeCtrl`: each row is attached to the nearest preceding lower-level entry, top-level rows stay visible, and nested authored books/topics start collapsed.
- Adds explicit expand/collapse controls and lets a targetless book title toggle its branch. Clickable topic rows continue to navigate normally, including authored cross-document Contents targets.
- Synchronizes the exported tree with root-document navigation exactly like the native viewer. When the current topic belongs to the navigation-root HLP and has an authored Contents row, its ancestor branches are expanded automatically and the row is selected. Cross-document topics leave the original tree/selection alone instead of making the referenced HLP appear to own the pane.
- Keeps **Show all** as the separate physical-topic list. When neither `.CNT` nor usable `.GID` hierarchy exists, hierarchical mode still reports that fact instead of silently substituting Show all.

## v0.7.1-buildfix70

- Fixes the HTML export's overlapping/merged words. Build-fix 69 positioned every text token with retained WinHelp geometry, but then allowed the browser to paint each token at its own natural font width. Even a small browser-vs-retained width difference therefore extended one absolutely positioned word into the next token's retained space.
- Splits each exported text object into an outer retained geometry/hotspot/background box and an inner `text-glyphs` layer. The outer box is never resized or transformed, so link hit areas, authored backgrounds, paragraph geometry, wrapping decisions, and following-token origins remain exactly where the exporter placed them.
- On topic activation the HTML viewer measures the browser's actual shaped glyph width and fits only the inner glyph layer to the exporter's retained token width. Small differences are corrected with character tracking; a horizontal scale is used only for any residual mismatch that exceeds the bounded tracking correction.
- Disables browser kerning and standard/contextual ligatures for retained HLP text so Chromium/Firefox-style shaping does not silently introduce spacing behavior that classic GDI `TextOut` did not use.
- The correction runs identically for the main topic surface, exported popups, and secondary-window surfaces, and is independent of CSS zoom/browser page scaling.

## v0.7.1-buildfix69

- Adds **File > Export to HTML...** through a dedicated `viewer/src/html_export.rs` module. The exporter consumes the same decoded `HelpDocument` presentations and retained `LayoutEngine` boxes as the native renderer; it does not introduce a second HLP parser or make HTML part of the desktop rendering path.
- Produces one self-contained interactive HTML file with no external scripts, stylesheets, images, fonts, or server dependency. Retained text formatting, explicit colours/backgrounds, paragraph/table geometry, borders, decoded RGBA pictures, picture hotspots, safe built-in WinHelp buttons, and inert hosted-control placeholders are represented directly in the export.
- Recreates the viewer shell with the same `RGB(212,212,212)` gray content host and `RGB(255,255,228)` cream help page, plus the current `RGB(249,249,158)` WinHelp information yellow. The HTML viewer includes Contents / Index / Search / Bookmarks / History, best-effort persistent browser-local bookmarks, a draggable navigation splitter, Back/Forward, physical Previous/Next, authored browse buttons, navigation-pane toggle, display zoom, and print styling.
- Preserves build-fix 68's navigation-root separation. An export begun while WORDPAD is displaying a linked COMMON topic can start on that COMMON topic while Contents / Show all / Index / Search remain owned by WORDPAD and its authored catalogs. Relative links inside an embedded cross-document HLP continue to resolve against that HLP.
- Recursively embeds bounded **relative** HLP references discovered from Contents, `:Index`/`:Link`, external hotspots, and allow-listed CONFIG/topic macros. Absolute/UNC targets are never traversed automatically; unresolved/unembedded targets become safe unavailable actions.
- Translates only the existing typed `SafeHelpMacro` allow-list to local JavaScript operations. Arbitrary macro text, hosted controls, executables, DLLs, ActiveX, and shell commands are not executed. Hotspot hover text retains the build-fix 20 destination-title/popup-body behavior, while activation retains build-fix 16's single-main-topic-surface policy.
- Index export now merges keyword rows case-insensitively and de-duplicates by `(document, topic)` destination before action registration.
- Documents the export architecture, fidelity boundaries, navigation-root semantics, palette, and security model in `docs/HTML_EXPORT.md`.

## v0.7.1-buildfix68

- Keeps the navigation pane anchored to the HLP that was explicitly opened, even when a topic jump loads another HLP into the main document surface. Cross-document navigation now changes only the active topic document; Contents, Index, and Search continue to represent the original navigation root.
- Adds a separate `navigation_document` to main-viewer state. Manual/startup opens replace this root and rebuild its linked Index/Search catalog; cross-file jumps, Back/Forward restores, and external Contents targets leave it untouched.
- Contents rows are resolved against the original HLP even while an external topic is active. `Show all` and the Contents command therefore return to/navigate within the original document instead of accidentally interpreting its row indices against the referenced HLP.
- Index and Search remain sourced from the original HLP plus only the related catalogs authored by that original document. Loading an ordinary external target such as `WORDPAD.HLP -> COMMON.HLP` no longer swaps the pane to COMMON.HLP's keyword/search structure.
- Cross-document topics still use their own rendering, fonts, browse sequence, windows, macros, title, history location, and subsequent relative external-link resolution. Only the discovery/navigation structure stays rooted in the originally opened help file.

## v0.7.1-buildfix67

- Replaces the navigation overflow reveal's `tooltips_class32` placement path with a true viewer-owned Windows popup. The popup has its own registered window class, custom paint routine, and explicit screen-space position, so Windows no longer gets a second chance to move the reveal beside the cursor.
- The popup's painted text origin is exactly the cropped control-label origin. Index / Search / Bookmarks / History share the same `LB_GETITEMRECT` + native-font vertical-centering geometry; Contents uses the TreeCtrl text-only item bounds. The popup subtracts only its own known border/padding from that origin, making the full label overlay the clipped line.
- The custom surface uses the source control's actual `WM_GETFONT` font, the existing RGB(249,249,158) WinHelp information background, black text, and a one-pixel black frame. It is `WS_EX_NOACTIVATE`, tool-window/topmost, and hit-test transparent so it neither steals focus nor becomes the mouse target.
- Keeps the shared 350 ms initial hover delay with a small show-only timer; the timer does not discover, recolour, or reposition another native tooltip. Moving an already-visible reveal updates its text/position directly.
- The same `OverflowTooltip` driver still covers Contents, Index, Search, Bookmarks, and History, and the wxToolTip path remains only as a creation-failure/non-Windows fallback. Clipping measurement remains UTF-16/GDI-first.

## v0.7.1-buildfix66

- Retained build-fix 63's ordinary common-control tooltip but attempted to align it after it became visible by reading its real window rectangle, converting it to the actual text rectangle, and shifting it with a short-lived timer.
- Restored physical-cursor leave protection for an in-place reveal. This approach is superseded by build-fix 67's viewer-owned popup, which no longer depends on another tooltip window's final placement.

## v0.7.1-buildfix65

- Attempted to reposition the ordinary common-control overflow tooltip from its owner-side `TTN_SHOW` notification. This approach is superseded by build-fix 67.

## v0.7.1-buildfix64

- Changes only the navigation overflow-tooltip placement; build-fix 63's ordinary `TTF_IDISHWND | TTF_SUBCLASS` lifecycle, hover delays, clipping tests, UTF-16/GDI measurement, palette, and shared Contents / Index / Search / Bookmarks / History driver are otherwise unchanged.
- The native tooltip window is subclassed only to correct its final `WM_WINDOWPOSCHANGING` coordinates. `TTM_ADJUSTRECT` converts the hovered label's desired text origin into the corresponding tooltip-window origin, so the tooltip text overlays the cropped line instead of appearing beside the cursor.
- ListBox anchors come from `LB_GETITEMRECT` plus the native label inset/vertical centring; Contents uses the TreeCtrl's existing text-only item bounds directly.

## v0.7.1-buildfix63

- Replaces the fragile row-anchored tracking-tooltip implementation for navigation overflow reveals with the same ordinary `TTF_IDISHWND | TTF_SUBCLASS` native tooltip contract already proven by hotspot previews. Windows now owns tooltip timing, placement, show, and hide behavior instead of the viewer driving `TTM_TRACKACTIVATE` / `TTM_TRACKPOSITION` from mouse-motion events.
- Adds one shared `OverflowTooltip` state driver for Contents, Index, Search, Bookmarks, and History. It caches the hovered reveal text and updates the native/wx tooltip only when the row changes, preventing continuous mouse motion from restarting the native hover timer. If the Windows common-control tooltip cannot be created, the driver falls back to `set_tooltip` rather than silently losing the reveal.
- Native navigation tooltip timing is 350 ms initial, 80 ms reshow, and 30 s auto-pop. The existing RGB(249,249,158) / black WinHelp information palette is applied before the tool is registered.
- ListBox clipping now prefers UTF-16 GDI measurement with the control's actual `WM_GETFONT` font (`SelectObject` + `GetTextExtentPoint32W`) and uses wxDragon measurement only as a fallback. A non-empty label that cannot be measured reliably is treated as clipped, so accented text can no longer be suppressed by a zero/invalid extent.
- Contents clipping now uses the TreeCtrl's own text-only bounding rectangle width, preserving correct indentation-aware geometry at every hierarchy level. The tree's built-in native tooltip remains detached with `TVM_SETTOOLTIPS` to prevent duplicates.
- Removes the obsolete row-rectangle anchoring helper, tracking activation/position state, and the synthetic `WM_MOUSELEAVE` workaround that was only necessary for the old overlay geometry.

## v0.7.1-buildfix62

- Fixes the remaining shared Windows navigation overflow-tooltip disappearance by addressing the overlay geometry itself rather than changing TOOLINFO registration again.
- The row-aligned tracking tooltip intentionally covers the clipped TreeCtrl/ListBox row. Windows can therefore emit `WM_MOUSELEAVE` for the underlying control as soon as the tooltip appears even though the physical cursor is still inside that control; build-fix 61 treated that synthetic transition as a real leave and immediately deactivated the tooltip.
- Windows leave handlers now verify the physical cursor against the actual TreeCtrl/ListBox client rectangle with `GetCursorPos`, `ScreenToClient`, and `GetClientRect`; the tooltip is hidden only when the cursor has genuinely left the widget.
- The build-fix 61 shared registration/activation contract is retained unchanged: one `NativeInlineOverflowTooltip` path for Contents, Index, Search, Bookmarks, and History, `TTF_IDISHWND | TTF_TRACK | TTF_ABSOLUTE | TTF_TRANSPARENT`, row-aligned placement, and RGB(249,249,158) / #F9F99E.
- No polling, delayed recolouring, duplicate wxToolTip path, or per-tab special case is reintroduced.

## v0.7.1-buildfix61

- Repairs the shared Windows navigation tracking tooltip after build-fix 60 accidentally made every overflow reveal disappear.
- `TOOLINFO.hwnd` is again the containing/owner window while `uId` is the actual TreeCtrl/ListBox HWND under `TTF_IDISHWND`, matching Microsoft's child-control tooltip contract and the viewer's already-working hotspot tooltip path.
- Tracking activation now follows the documented order: update text, `TTM_TRACKACTIVATE(TRUE)`, then `TTM_TRACKPOSITION`. Build-fix 60 sent the absolute position while the tracking tool was still inactive, which some common-control implementations ignore.
- The shared tracking tooltip registers with a stable non-empty UTF-16 buffer before later text updates.
- Contents, Index, Search, Bookmarks, and History still share one implementation, one clipping rule family, same-row placement, and RGB(249,249,158) / #F9F99E.

## v0.7.1-buildfix60

- Fixes the shared Windows navigation overflow-tooltip registration bug instead of patching individual tabs. The tracking tooltip now follows Microsoft's window-tool contract: `TTF_IDISHWND | TTF_TRACK | TTF_ABSOLUTE | TTF_TRANSPARENT`, with `uId` set to the actual TreeCtrl/ListBox HWND.
- The tracking tooltip is owned by the navigation control itself rather than its containing panel. With `TTF_TRANSPARENT`, mouse events over the inline reveal are therefore forwarded back to the TreeCtrl/ListBox that owns the row instead of being diverted to the panel.
- Removes the empty Windows `set_tooltip("")` path entirely from navigation binding/rebuild code so no wxToolTip object is created alongside the explicit native tracking control; those clears remain only in the non-Windows fallback.
- Contents, Index, Search, Bookmarks, and History continue to use the same `NativeInlineOverflowTooltip` implementation, the same row-clipping rules, and the same `RGB(249,249,158)` (`#F9F99E`) palette.
- Build-fix 56 formatted printing/topic ranges and build-fix 55 hotspot-tooltip behavior remain unchanged.

## v0.7.1-buildfix59

- Fixes the missing Contents overflow reveal exposed by the build-fix 58 runtime capture. A Windows TreeCtrl may own a tooltip control without automatically displaying the desired clipped-label reveal in this wxDragon configuration, so Contents no longer relies on that backend behavior.
- Detaches the TreeCtrl's built-in tooltip association with `TVM_SETTOOLTIPS` before installing the viewer's own overflow tip, preventing duplicate native tips.
- Contents now uses the same `NativeInlineOverflowTooltip` tracking control as Index, Search, Bookmarks, and History. Tree hit testing and the text-only item rectangle determine clipping and anchor the full label at the row's own top-left position.
- All five navigation item views therefore use one explicit Windows mechanism: single-line, row-aligned, `RGB(249,249,158)` (`#F9F99E`) with black text, and hidden when the label fits or the pointer leaves the row.
- Build-fix 56 formatted printing/topic ranges and build-fix 55 hotspot-tooltip behavior remain unchanged.

## v0.7.1-buildfix58

- Removes the duplicate navigation tooltip introduced by build-fix 57 on Windows. Contents now relies on the native TreeCtrl clipped-label tip only; no second cursor-positioned wxToolTip is layered over it.
- Index, Search, Bookmarks, and History replace `set_tooltip()` overflow tips with one pre-coloured native tracking tooltip per ListBox, anchored to the hovered row's top-left corner so the full label appears on the same line as the clipped text.
- The tracked ListBox tip stays single-line, preserves `RGB(249,249,158)` (`#F9F99E`) with black text, moves in place between rows, and hides immediately when the pointer leaves or the hovered row no longer overflows.
- Non-Windows builds retain the portable wxWidgets overflow fallback. Build-fix 56 formatted printing/topic ranges and build-fix 55 hotspot-tooltip behavior are unchanged.

## v0.7.1-buildfix57

- Generalizes the existing overflow-only bookmark tooltip behavior to every navigation item view: Contents, Index, Search, Bookmarks, and History.
- Contents uses wxDragon TreeCtrl hit testing plus the current text bounding rectangle so horizontally clipped tree labels expose their complete text on hover, including after horizontal/vertical scrolling.
- Index, Search, Bookmarks, and History share one ListBox overflow binder; Windows continues to use native `LB_ITEMFROMPOINT` row hit testing so the hovered row remains correct after vertical scrolling.
- Tooltips remain conditional: rows whose complete label fits in the visible client area do not show an overflow tooltip. Dynamic list/tree rebuilds clear any stale tooltip text.
- Retains the requested WinHelp tooltip palette and all build-fix 56 formatted-printing/topic-range behavior unchanged.

## v0.7.1-buildfix56

- Replaces build-fix 48's reconstructed plain-text printing path with printer-device retained layout. Printed topic bodies now preserve authored font mapping and size, bold/italic/underline/strikeout/small-caps semantics, foreground and explicit background colours, paragraph/table geometry, pictures, borders, and safe embedded-control placeholders.
- Adds a **Print Topics** chooser before the native Windows printer dialog: current topic, an explicit topic range, or all topics. Range syntax accepts inclusive one-based forms such as `3-8` and `1-3, 7, 10-12`; overlapping entries are de-duplicated and invalid/out-of-range values are rejected before opening the printer dialog.
- Lays out text directly against the selected printer HDC at printer DPI and paginates retained boxes by clipping/translation. Each selected topic begins on a fresh printer page; navigation chrome and selection highlighting remain excluded.
- Keeps build-fix 55's dedicated pre-coloured native hotspot tooltip unchanged.

## v0.7.1-buildfix55

- Eliminates the remaining one-frame Windows hotspot-tooltip blink instead of trying to repaint a lazily-created wxToolTip after it becomes visible.
- HLP hotspot previews now use one dedicated native `tooltips_class32` control per topic canvas. The HWND is created hidden, has visual theming disabled, and receives `RGB(249,249,158)` (`#F9F99E`) plus black text before any tool is registered.
- Registers the canvas with `TTF_IDISHWND | TTF_SUBCLASS`, so the pre-coloured tooltip control receives the ordinary Windows hover timing/mouse relay without a palette retry thread.
- Hover changes only replace the tooltip text; leaving a hotspot sends `TTM_POP` and clears the retained UTF-16 text buffer. The previous 10-ms/1.5-s lazy-HWND polling path is removed entirely.
- If native tooltip creation ever fails, the old wxWidgets tooltip path remains as a defensive fallback. Static toolbar/list tooltips are otherwise unchanged.

## v0.7.1-buildfix54

- Removes the mild Windows hotspot-tooltip flicker introduced by build-fix 53's repeated palette repaint loop.
- `apply_windows_tooltip_palette` now reports whether a matching thread-owned `tooltips_class32` HWND was actually found and styled.
- The lazy-creation retry worker stops immediately after the first successful palette application instead of continuing for the full retry window.
- Removes the repeated `TTM_UPDATE` repaint forcing; `RGB(249,249,158)` (`#F9F99E`) with black text is still applied through `TTM_SETWINDOWTHEME`, `TTM_SETTIPBKCOLOR`, and `TTM_SETTIPTEXTCOLOR`.
- Advances the retry generation before the synchronous attempt so an older worker is cancelled even when the next tooltip can be styled immediately.

## v0.7.1-buildfix53

- Fixes dynamic hotspot tooltip colouring on Windows when wxWidgets creates `tooltips_class32` lazily after `set_tooltip()` returns.
- Keeps the requested information background at `RGB(249,249,158)` (`#F9F99E`) with black text.
- Adds a bounded 1.5-second, 10-ms palette retry window after the hovered hotspot changes, so the Win32 palette messages reach the real native tooltip HWND once it exists. A generation token cancels an older retry worker as soon as another hotspot replaces it.
- The normal topic page remains `RGB(255,255,228)` (`#FFFFE4`).

## v0.7.1-buildfix52

- Changes the shared WinHelp tooltip/popup information background to `RGB(249,249,158)` (`#F9F99E`), as explicitly sampled from the requested reference.
- The normal topic page remains `RGB(255,255,228)` (`#FFFFE4`), so information surfaces now have a clearly visible darker-yellow contrast.
- Native Windows hover tooltips and the legacy popup-note fallback still share one `WINHELP_INFO_BACKGROUND` constant; tooltip text remains black.

## v0.7.1-buildfix51

- Matched the WinHlp reference information-surface colour exactly: popup/tooltip yellow is now `RGB(255,255,225)` (`#FFFFE1`) while the normal help-page background remains `RGB(255,255,228)` (`#FFFFE4`).
- Removed the older over-saturated popup fallback `RGB(255,255,184)` and made native hover tooltips plus legacy popup-note rendering share one `WINHELP_INFO_BACKGROUND` constant so the two paths cannot diverge.
- Black tooltip text and all build-fix 50 navigation/bookmark behavior are unchanged.

## v0.7.1-buildfix50

- Replaces the fixed 300-pixel navigation/content boundary with a native wxDragon splitter. The Contents / Index / Search / Bookmarks / History column can now be resized by dragging the divider, with a 180-pixel minimum pane width.
- Keeps the browse strip structurally attached to the document column, so its centering follows the resized document surface instead of depending on a fixed-width alignment spacer.
- Preserves View > Navigation Pane / F9: hiding remembers the current sash width and showing restores it. Macro-driven Index, Search, Bookmarks and History activation uses the same splitter-aware visibility path.
- Adds overflow-only bookmark tooltips. On Windows the hovered native list-box row is hit-tested even after scrolling; a tooltip appears only when the bookmark label is wider than the visible list client area and uses the existing classic WinHelp tooltip palette.
- Retains build-fix 49's printing compilation corrections and build-fix 48 printing behavior unchanged.

## v0.7.1-buildfix49

- Fixes Windows printing compilation with the workspace `-D unsafe-code` policy by applying local `#[allow(unsafe_code)]` only to the two Win32 extern blocks and three printing backend functions.
- Fixes the two print-related message dialogs by calling `.build()` before `.show_modal()`, consistent with wxDragon 0.9.17's builder API.
- No printing behavior or HLP parsing semantics are otherwise changed from build-fix 48.

## v0.7.1-buildfix48

- Adds **File > Print... (Ctrl+P)** for the current HLP topic.
- Windows builds use the native system Print dialog and printer device context.
- Topic text is word-wrapped to the printer's printable width and paginated automatically; the navigation pane and on-screen selection highlighting are not printed.
- Printing is read-only and does not mutate HLP/CNT/GID state.
- Non-Windows builds remain compilable and report that the print backend is currently Windows-only.

# Changelog

## 0.7.1 build-fix 47 - 2026-07-26

- Added mouse-drag text selection for retained fixed and scrolling topic regions, with selection painting that preserves the existing native-font metrics and read-only topic model.
- Added **Edit > Copy** (`Ctrl+C`) for selected topic text or selected Index/Search query text, backed by the native wxWidgets clipboard.
- Added **Edit > Paste** (`Ctrl+V`) for the focused Index or Search query field and **Edit > Select All** (`Ctrl+A`) for either the active topic region or focused query field. Paste into a topic remains intentionally unavailable because help topics are read-only.
- Changed topic hotspot activation to mouse-up and suppress activation after a selection drag so hyperlink clicks and text selection coexist.
- Preserved plain Left/Right physical topic navigation and Alt+Left/Alt+Right Back/Forward history navigation.

## 0.7.1 build-fix 46 - 2026-07-26

- Added read-only WinHelp 4.x `.GID` Contents support without adding another source module: the existing `hlp/src/contents.rs` now falls back to a same-basename case-insensitive `.GID` only when the authored `.CNT` is unavailable/unreadable.
- Decoded GID `|CntText` / `|CntJump` `Lz` B+tree leaves as `u32 key + NUL-terminated Windows-1252 text`. Ordinary `|CntText` keys provide ordered row titles; key `70000` supplies the cached Contents title and key `70001` supplies the cached base HLP/window. Matching `|CntJump` keys provide clickable targets, so non-jump book rows remain non-clickable.
- Decoded the supplied Windows 95 WordPad `|Flags` Contents tail conservatively: byte `0x0C` followed by one node byte per ordinary `|CntText` row, with the high nibble used as hierarchy level. The 35 resulting levels match all 35 numbered `WORDPAD.CNT` rows exactly. GIDs that do not have this verified tail are rejected as a hierarchy source instead of being guessed flat.
- Decoded GID `|FILES` `L4z` leaves (`u32 key`, cached metadata dword, NUL-terminated text) for one-hop Index/Search catalogs. Absolute cached Win9x paths are reduced to their final filename before passing through the viewer's existing relative-path and automatic-catalog security rules; key `10000` (cached CNT pathname) is not treated as a help link.
- Updated the Contents UI to describe `.CNT`/`.GID` hierarchy sources and to report hierarchy unavailable only when neither usable source exists. **Show all** remains the explicit physical-topic diagnostic view.
- Added focused parser regressions for GID keyed text, hierarchy-tail validation, `|FILES` decoding/path portability, and retained the existing CNT parser behavior. The project still does not generate or update `.GID` files.

## 0.7.1 build-fix 45 - 2026-07-26

- Restored the classic WinHelp-era information-tooltip palette on Windows. Native wxWidgets tooltips now use Microsoft's classic `InfoWindow` background `RGB(255,255,225)` (`#FFFFE1`) with black `InfoText`, visibly darker than the viewer's `RGB(255,255,228)` help-page cream. The tooltip control's visual-style subclass is cleared before applying `TTM_SETTIPBKCOLOR` / `TTM_SETTIPTEXTCOLOR`, because themed common controls otherwise ignore those color messages.
- Reworked the Contents tab into two explicit modes. **Hierarchical view** is now the default and renders the authored `.CNT` book/topic levels without flattening them; **Show all** lists every decoded HLP topic in physical presentation order for diagnostics and discovery.
- Removed the old silent flat-topic fallback from the hierarchical Contents mode. When the HLP has no discoverable `.CNT` sidecar, the hierarchy view says that authored hierarchical contents are unavailable and points the user to **Show all** instead of presenting the physical topic list as though it were the original contents structure.
- Manual/new-document opens reset the Contents tab to **Hierarchical view**; ordinary in-document navigation preserves the selected mode. Both modes continue to attach the existing typed navigation metadata to clickable topic rows.

## 0.7.1 build-fix 44 - 2026-07-26

- Fixed the build-fix 43 invalid-magic regression test on current Rust toolchains by explicitly typing its synthetic `0x12345678` magic value as `u32` before calling `to_le_bytes()`. The value is compared with the `u32` HLP container magic and is serialized as the same four-byte header width used by the parser.
- Runtime HLP parsing and diagnostics are unchanged; this is a compile-only test correction.

## 0.7.1 build-fix 43 - 2026-07-26

- Classified the `0x00024E4C` / `LN 02` signature separately from corrupt Windows WinHelp input. This signature belongs to a different legacy Microsoft help/index family used by files such as MS-DOS/QBasic `HELP.HLP`; the viewer now reports that distinction explicitly with an `Unsupported HLP family` dialog instead of only saying that the WinHelp magic is invalid.
- Kept the actual Windows WinHelp container check strict at `0x00035F3F`; build-fix 43 does **not** pretend that the structurally unrelated `LN 02` format is a second WinHelp container generation.
- Added parser regression coverage proving that `LN 02` receives the classified unsupported-family diagnostic while arbitrary bad magic still produces the ordinary `InvalidMagic` error.

## 0.7.1 build-fix 42 - 2026-07-26

- Fixed zoom-dependent vertical misalignment of CALC.HLP's blank 12x12 Related Topics ALink button. The stock `!label,macro` BUTTON now participates in the same retained baseline finalization as adjacent text and inline pictures, so its bottom edge follows the measured text baseline at every zoom level.
- Kept arbitrary authored hosted-control placeholders out of this baseline rule because their native child-window geometry is runtime-negotiated rather than equivalent to the verified stock WinHlp32 button form.
- Added regression coverage for small, normal, and enlarged text metrics and updated the existing Related Topics rule-gap test to distinguish the line's top position from the button's baseline-relative position.

## 0.7.1 build-fix 41 - 2026-07-26

- Replaced the Bookmarks pane's full-width `Add Current Topic` button with compact adjacent `+` and `-` controls. `+` bookmarks the current topic and `-` removes the selected bookmark.
- Changed bookmarks from session-only state to portable persistent storage beside the executable. The filename is derived from the executable stem, so the normal build writes `hlp-viewer.bookmarks` and a renamed portable executable writes `<program-name>.bookmarks`.
- The bookmark file retains the display label, source HLP path, topic index, optional TOPICOFFSET, and optional window name with escaped tab/newline/backslash fields, allowing file-qualified bookmarks to survive restarts without adding a serialization dependency.
- Persisted bookmarks are loaded at startup, and a bookmark can now reopen its HLP even when no document is currently loaded. Add/remove operations write the complete bookmark set immediately; storage failures are reported in the status bar without discarding the in-memory change.

## 0.7.1 build-fix 40 - 2026-07-26

- Applied the existing HLP application icon to the live native Windows frame as well as the executable resource. The main wxWidgets `HWND` now receives explicit large and small `WM_SETICON` icons loaded from embedded resource 1, fixing the generic title-bar icon visible even after build-fix 38 embedded the `.ico` into `hlp-viewer.exe`.
- Loads separate icon handles at the current Windows large/small system icon metrics from the multi-size `viewer/assets/hlp.ico`; no external runtime asset file is required. The two handles remain valid for the lifetime of the one main frame.
- Added the `Win32_System_LibraryLoader` and `Win32_UI_WindowsAndMessaging` windows-sys feature gates needed for the native resource/window calls. Non-Windows behavior is unchanged.

## 0.7.1 build-fix 39 - 2026-07-26

- Recovered and re-verified the exact 285,696-byte KB917607 `winhlp32.exe` reference (`SHA-256 8496f19bc1d898180b57aac61326bdfcf5a48f760811283bbd604aa7a2c13f85`) and traced the five residual compatibility questions left after build-fix 17. The Microsoft executable remains external to the source archive.
- Closed `JOHAB_CHARSET` (`0x82`) as Windows CP1361 and added a deterministic compact CP1361 decoder/table plus regression vectors. `OEM_CHARSET` (`0xFF`) is now explicitly classified as host-GDI-selected behavior rather than a missing fixed HLP mapping; Windows builds decode it through the active `CP_OEMCP`, with a documented deterministic fallback off Windows.
- Traced the reference Unicode draw path through `GetTextCharset` / `TranslateCharsetInfo`, `MultiByteToWideChar`, and `TextOutW`. On Windows, non-ANSI/default legacy runs now preserve the authored face name and charset into the existing GDI `LOGFONTW` backend instead of applying the viewer's modern Western-face substitution first.
- Resolved character command `0x85`: the signed WORD overwrites WinHlp32 render-state `+0x38`, the horizontal line origin used by line finalization/alignment. Retained layout now applies the same glyphless x-origin reset.
- Traced hotspot activation at `0x429C13..0x429E24`. The residual structurally accepted `C0..CF` / `E0..EF` envelope values outside the decoded macro/internal/external families have no click-action branch in this KB917607 runtime and are now reported as verified inert rather than semantically unresolved.
- Traced authored hosted-control sizing. Arbitrary controls are initially created at exactly `2*LOGPIXELSX` by `2*LOGPIXELSY`; final size is control-negotiated through private message `0x706B`, falling back to `GetWindowRect`. Safe mode still refuses to load HLP-supplied native controls but now uses the verified two-device-inch creation rectangle instead of the invented 180x36 placeholder.
- Retitled the consolidated document to **Microsoft WinHelp (.HLP) Internal Format — Reference Manual** and revised its confidence matrix, residual-gap analysis, quick references, and executable-address appendix accordingly.

## 0.7.1 build-fix 38 - 2026-07-26

- Added the supplied HLP application icon to the source tree as `viewer/assets/hlp.png` and a generated multi-resolution `viewer/assets/hlp.ico` for Windows packaging.
- Added `viewer/build.rs` and the `winres` build dependency so Windows builds embed the ICO into the generated `hlp-viewer.exe`. This gives the executable a project-specific Explorer/taskbar/window icon instead of the default toolchain icon.
- Added `docs/THIRD_PARTY_ASSETS.md` recording the requested credit for the icon source: <https://www.flaticon.com/free-icon/hlp_8263260>.
- Updated the README to document the new application-icon asset and where attribution is stored.

## 0.7.1 build-fix 37 - 2026-07-26

- Added `docs/MICROSOFT_WINHELP_INTERNAL_FORMAT_REFERENCE.md` and a polished DOCX edition as a consolidated reverse-engineering manual covering the HLP container, named streams, `|SYSTEM`, phrase compression, `|TOPIC`/TOPICLINK, paragraph and character streams, fonts/charsets/DPI, recursive tables, bitmap/DDB/DIB/WMF graphics, hotspots, navigation metadata, `.CNT`, keyword tables, hosted controls, safe macros, and executable-address findings.
- Added explicit **Verified / Strong inference / Unresolved** confidence labels and a dedicated **Corrections to received WinHelp lore** section, including the verified rejection of `0x20`/`0x21` as character commands and the universal 11-byte `|FONT` descriptor model.
- Added quick-reference tables and an address appendix tied specifically to the retained 285,696-byte KB917607 `winhlp32.exe` reference (`8496f19b…13f85`), while keeping Microsoft binaries external to the source tree.

## 0.7.1 build-fix 36 - 2026-07-26

- Centered the visible browsing strip over the help document rather than over the complete application frame. The toolbar row now reserves the same left-hand width as the Contents/Index/Search/Bookmarks/History pane, so the remaining Previous/Next, authored browse, navigation-toggle and zoom controls are horizontally centred on the cream help-page region.
- The alignment gutter follows navigation-pane visibility: hiding the pane removes the gutter and recentres the controls over the newly widened help page; reopening the pane restores the matching offset. The cream page's symmetric frame inset does not require a second correction because its centre is the same as `content_host`'s centre.

## 0.7.1 build-fix 35 - 2026-07-26

- Fixed zoomed line crowding. The native text measurer already enlarged glyph cells, but signed WinHelp `spacing_lines` advances remained at their unzoomed device-pixel value. The layout engine now carries the viewer text zoom separately from device DPI and scales only the authored line-advance metric, preserving 100% WinHlp32 semantics while keeping 150%-200% text vertically proportional.
- Fixed maximize/restore reflow and background clipping by moving resize handling from the frame's early size event to the content host's post-sizer size event. Restored viewport width is now authoritative before retained layout is rebuilt, and newly exposed page/border/background regions are explicitly invalidated.
- Removed the visible **Contents**, **Back-history**, and **Forward-history** buttons, including their hidden toolbar entries and custom browse-strip widget state. Contents and history navigation remain available through the Navigate menu and existing keyboard shortcuts; the visible strip now starts with physical Previous/Next.
- Simplified the About dialog to the application name, one-line description, and GUI toolkit, removing the milestone/default-deny macro and diagnostics paragraphs requested by the UI cleanup.

## 0.7.1 build-fix 34 - 2026-07-26

- Completed the portable legacy charset path used by retained LinkData2 text. In addition to the existing Western/Hebrew/Arabic decoding, the viewer now handles Windows Central/Eastern European, Cyrillic, Greek, Turkish, Vietnamese, Baltic and Thai charsets plus the major Japanese, Korean, Simplified Chinese and Traditional Chinese DBCS families.
- Reproduced the verified WinHlp32 charset-selection precedence more closely: an explicit non-default `|SYSTEM` record-11 per-face charset wins; absent/default charset metadata falls back to deterministic legacy face-name and `LANGID` inference for the common historical Windows families. Symbol/Wingdings remain symbol charset.
- Added CJK-aware retained wrapping. Japanese/Chinese/Korean text may break between ideographic/kana/hangul units without requiring ASCII spaces, while ordinary Latin runs stay grouped and common opening/closing punctuation is kept with its neighbouring unit.
- Added parser-to-layout regressions for Greek/Cyrillic/Central-European text, Shift-JIS, GBK and Big5 decoding, locale/face inference precedence, and a real Shift-JIS LinkData2 formatting path. At build-fix 34 time, Johab and host-dependent OEM charset inference remained deliberately unresolved rather than guessed; build-fix 39 closes/reclassifies those cases.

## 0.7.1 build-fix 33 - 2026-07-26

- Implemented authored bitmap physical-resolution sizing from the verified KB917607 graphics path. For bitmap alternatives carrying nonzero x/y resolution fields, retained layout now computes natural size independently per axis as `pixels * device_dpi / authored_resolution`; zero-resolution bitmap records retain raw-pixel sizing.
- Made WMF natural dimensions device-DPI aware. Physical mapping modes now convert logical extents using the layout engine's horizontal/vertical DPI instead of a fixed 96-DPI constant; pixel mapping mode remains one logical unit per device pixel.
- Kept WMF rasterization itself on the existing bounded 96-DPI RGBA compatibility surface. Only retained display geometry changes, so the normal picture painter/hotspot scaling path resamples that safe raster to the WinHlp32-equivalent natural size.
- Added headless regressions for asymmetric target DPI, authored bitmap resolution, and a 0.01-mm WMF mapping mode.

## 0.7.1 build-fix 32 - 2026-07-26

- Removed the viewer's whole-point font-size truncation on Windows. Retained HLP sizes stay in twentieths of a point through zoom and are converted directly to a negative `LOGFONTW.lfHeight` using the canvas vertical DPI; measurement and painting use the same GDI font definition.
- Split retained layout DPI into independent x/y axes. Horizontal paragraph metrics, tab/table geometry, and fallback text width use horizontal DPI; vertical paragraph/line spacing and fallback font height use vertical DPI. `LayoutEngine::new(dpi)` remains as the square-DPI compatibility constructor, with `LayoutEngine::with_dpi(dpi_x, dpi_y)` added for native front ends.
- The Windows viewer now reads `LOGPIXELSX`/`LOGPIXELSY` from the actual topic canvas and supplies those values to retained layout. The wxDragon integer-point font path remains a portable fallback if the native GDI context/font cannot be created.
- Added pure regressions proving 8.5-point/110% zoom survives as 187 twips before device conversion, that vertical font height changes with device DPI, and that paragraph x/y metrics use their correct device axes.
- Build-fix 33 subsequently applies the same per-axis device-DPI context to bitmap authored-resolution and WMF natural-size conversion.

## 0.7.1 build-fix 31 - 2026-07-25

- Fixed the remaining CALC.HLP Related Topics horizontal misalignment without discarding authored paragraph metadata. Build-fix 30 had forced stock ALink rows to `left_indent = 0` and `first_line_indent = 0`; build-fix 31 restores the normal signed DPI/144 indent path.
- Re-checked the retained 285,696-byte KB917607 `winhlp32.exe` (SHA-256 `8496f19bc1d898180b57aac61326bdfcf5a48f760811283bbd604aa7a2c13f85`). The hosted-object dispatcher at `0x419281` calls factory `0x4240F4` for the leading-`!` form, while the empty-label branch stores `0x000C000C` and calls `MoveWindow` at `0x424593`, confirming the stock control's 12x12 geometry but not a page-margin horizontal override.
- The top-level record walker now remembers the actual rendered x-coordinate of a rule-only record's `Border`. When the following record contains the stock ALink button, the already-laid-out button and text boxes on that visual line are translated by the exact delta to the saved rule edge. This follows authored border geometry whether the edge is the region margin or an indented border.
- Retained the build-fix 30 4-pixel vertical gap and safe/clickable `ALink` implementation unchanged.
- Replaced the misleading margin-normalized regression with a deliberately indented rule whose rendered x is not `PAGE_MARGIN`, and added a second regression proving a standalone ALink row still preserves authored left and first-line indents.

## 0.7.1 build-fix 30 - 2026-07-25

- Aligned CALC.HLP's classic Related Topics control row with the authored double-rule separator. The standard `!label,macro` ALink row no longer preserves its negative hanging indent: the 12x12 blank button now starts at the same left edge as the separator, and the complete row is moved 4 pixels farther below the rule.
- Implemented the safe WinHelp `ALink` / `AL` macro. Semicolon-delimited names are looked up exactly in the authored `|AWBTREE` / `|AWDATA` associative-link table, resolved through the existing TOPICOFFSET anchors, deduplicated in authored order, and shown in a native **Topics Found** chooser when more than one topic matches. A single match navigates directly; no match remains non-fatal.
- Verified the supplied CALC.HLP contains `A_CALC_LIST_EQUIV` and `A_CALC_KEYB_SEQ` in its A-table, exactly matching the retained `AL("A_CALC_LIST_EQUIV;A_CALC_KEYB_SEQ")` descriptor.
- Made retained built-in `!label,macro` BUTTON placeholders real viewer-local macro hotspots. Clicking the blank square itself now invokes the same bounded/default-deny macro dispatcher as an authored macro hotspot; unsafe button macros remain blocked.
- Added regression coverage for exact semicolon A-table lookup, ALink macro allow-listing, Related Topics left/gap geometry, and clickability of the stock hosted button.

## 0.7.1 build-fix 29 - 2026-07-25

- Corrected the root cause of CALC.HLP list-marker vertical alignment. The square and triangle markers are **not text glyphs**: direct decoding of the supplied CALC.HLP shows the paragraph `0x86` compact commands contain nested `0x22` graphics. The square marker resolves to indexed `|bm0` (3x7 pixels) and the triangle marker to `|bm1` (4x8 pixels).
- This explains why build-fix 25-28 could leave the screenshot unchanged: those changes aligned text runs, while the visible marker remained an independently positioned inline picture.
- Extended line baseline finalization to include inline pictures. Text uses its retained font baseline; an inline picture uses its bottom edge (`height`) as its object baseline, matching the KB917607 inline-object path around `0x416A73..0x416B7A`, where emitted object records receive the common line metric from the object's bottom relative to their y origin.
- Transparent `PictureHotspot` overlays now receive the same vertical shift as their owning inline image, preserving hit-test registration after baseline alignment. Floating pictures remain excluded because they are outside the line's retained box slice.
- Added an integration regression using a 3x7 indexed inline marker beside 20-pixel text with a 15-pixel baseline; it proves that the picture bottom and text baseline coincide and that the picture hotspot follows the same shift.
- Build-fix 28's real text-baseline retention remains useful for genuinely mixed-font text lines, but it is no longer claimed to be the CALC list-marker fix.

## 0.7.1 build-fix 28 - 2026-07-25

- Added real font-baseline retention for mixed-font text runs, replacing the build-fix 25-27 height-only heuristic. This infrastructure is retained, but build-fix 29 subsequently proved that CALC.HLP's visible square/triangle list markers are inline bitmaps rather than text glyphs.
- `TextMetrics` now retains a baseline offset. The native viewer derives it from `get_full_text_extent()` as `text height - descent`; retained text boxes carry that baseline through layout and line finalization aligns the actual baseline offsets.
- The GUI-independent fallback still synthesizes a bounded baseline when no native metric is available, but the normal Windows viewer path no longer guesses from box height.
- Added a regression in which two text runs deliberately have the same 20-pixel cell height but different 10/16-pixel baselines, proving native baseline alignment works for genuinely mixed-font text. Build-fix 29 adds the separate inline-picture regression needed for CALC.HLP.
- Line height is expanded when a baseline shift makes a text cell extend below the original line box, preventing the correction from overlapping the following visual line. Paragraph-authored spacing, hanging indents, wrapping, pictures, hosted controls, RTL ordering, and hotspot semantics remain unchanged.

## 0.7.1 build-fix 27 - 2026-07-25

- Tuned the mixed-font bullet/text vertical alignment again. Build-fix 26's ascent estimate was still slightly too conservative for CALC.HLP under the viewer's native wxWidgets text metrics, leaving the body text a little too low beside the small square marker.
- The retained baseline approximation now uses a 5/6-height ascent estimate instead of 3/4-height. This lowers the small bullet run a little more so the first text line sits visually higher and closer to the marker, matching the expected WinHelp appearance better.
- The correction remains limited to mixed-height text runs; paragraph spacing, hanging indents, wrapping, pictures, hosted controls, and hotspot geometry are unchanged.
- Updated the headless regression to assert the refined 5/6-ascent alignment for an 8 px bullet-like run beside 20 px body text.

## 0.7.1 build-fix 26 - 2026-07-25

- Refined the mixed-font bullet alignment introduced in build-fix 25. A full bottom-alignment of text boxes pushed CALC.HLP's small square list markers slightly too low relative to the body text.
- The retained line finalizer now approximates a shared baseline from each run's ascent (3/4 of measured text height) rather than from the full text-box bottom. This keeps the body text slightly higher while preserving the intended alignment with the bullet marker.
- Same-height text, pictures, hosted controls, wrapping, indents, paragraph spacing, and hotspot geometry remain unchanged.
- Updated the headless regression to assert the refined ascent-based alignment for an 8 px bullet-like run beside 20 px body text.

## 0.7.1 build-fix 25 - 2026-07-25

- Corrected the small square bullets used by CALC.HLP `Dicas` paragraphs. The bullet run uses a smaller measured text height than the neighbouring body-text run; retained layout previously placed both at the same top y-coordinate, so the bullet appeared too high.
- Text runs on the same visual line are now bottom/baseline aligned after horizontal alignment and RTL run ordering. Same-height text is unchanged, while smaller font/symbol runs are lowered to share the surrounding text baseline.
- Pictures, hosted controls, borders, wrapping, indents, and paragraph spacing are not moved by the baseline correction.
- Added a headless regression with an 8-pixel bullet-like run beside a 20-pixel body run, proving both retained text boxes finish on the same baseline.

## 0.7.1 build-fix 24 - 2026-07-25

- Corrected the staggered Calculator `Equivalentes de teclado` table. CALC.HLP uses empty compact display cells as structural fillers between independently flowing table columns; the retained layout was incorrectly assigning those fillers a synthetic blank-text height, so later cells in some columns started one line below their peers.
- Re-traced the verified 285,696-byte KB917607 WinHlp32 display path. At `0x415B44..0x415BA1`, an empty LinkData2 string followed immediately by `0xFF` is consumed through a fast path that returns before paragraph spacing/line-height advancement. Parsed table-cell display paragraphs matching that zero-content case now contribute zero height while bordered paragraphs, explicit line breaks/tabs, pictures, and hosted controls remain layout-bearing.
- Preserved WinHlp32's independent per-column table cursors (`0x4151DC..0x4151E9`); the fix removes false height from empty cells rather than forcing table rows onto a synthetic shared-row model.
- Added an 8-pixel gap when an authored rule-only display record is immediately followed by a table, giving the first table line breathing room below the second horizontal rule without adding spacing between subsequent table records.
- Added headless regressions proving that an empty filler cell cannot stagger a later column and that the requested post-rule gap occurs only at the display-to-table boundary.

## 0.7.1 build-fix 23 - 2026-07-25

- Fixed the recent-document configuration parser compile failure by explicitly typing its accumulator as `Vec<PathBuf>`. This prevents Rust from inferring the unsized `Path` type through the `same_path(&Path, &Path)` predicate and eliminates the reported E0277/E0308/E0599 errors.
- Removed the stale unused `PanelStyle` import from `viewer/src/main.rs`.
- No recent-document behavior, configuration location, WinHelp parsing, rendering, or navigation semantics were changed; this is a build-only correction to build-fix 22.

## 0.7.1 build-fix 22 - 2026-07-25

- Changed recent-document persistence so `hlp-viewer.cfg` is stored **beside the running viewer executable** on every platform.
- Removed the build-fix 21 `%LOCALAPPDATA%\hv`, `%APPDATA%`, XDG, home-directory, and current-working-directory fallbacks for this configuration file.
- `config_path()` now derives the directory from `std::env::current_exe()` and returns an I/O error rather than silently writing somewhere else if the executable location cannot be resolved. Existing configuration failures remain non-fatal to document viewing.
- Added regression coverage proving the configuration path is exactly `<current executable directory>\hlp-viewer.cfg`.

## 0.7.1 build-fix 21 - 2026-07-25

- Re-audited the Calculator `Related Topics` inline object against the retained 285,696-byte KB917607 `winhlp32.exe` and corrected the stale assumption that character commands `0x86`/`0x87`/`0x88` always contain pictures. They contain a compact TOPICLINK whose own record type selects the renderer.
- Confirmed that the object shown as `[embedded picture]` in CALC.HLP is an old `0x05` hosted/custom-window record, not a `0x03`/`0x22` graphic. Its retained descriptor is `!,AL("A_CALC_LIST_EQUIV;A_CALC_KEYB_SEQ")`.
- Followed WinHlp32 hosted-control renderer `0x419281` into factory `0x4240F4`: the leading `!` selects the built-in `BUTTON` class, the text before the comma is the button label, and the text after it is the authored macro. CALC's label is empty; WinHlp32 creates the child at 30x12 and then immediately resizes that empty-label form to its final **12x12** layout geometry.
- Generalized inline compact-object parsing so old/modern graphics (`0x03`/`0x22`) still use the picture pipeline, hosted controls (`0x05`/`0x24`) use the safe retained-control pipeline, `0x06` remains no-render, and bounded unsupported compact records are consumed without desynchronizing following text.
- Render CALC's verified leading-`!` built-in form as the final 12x12 classic raised button instead of a fabricated picture placeholder. Non-empty `!label,macro` controls retain a bounded standard-button substitute; the authored macro remains metadata and is never executed by this safe visual path.
- Added **File > Recent Documents** with up to five successfully opened HLP files. Opening a file moves it to the top of the MRU list and immediately refreshes the native File menu.
- Added persistent `hlp-viewer.cfg` storage for the MRU list. Windows uses `%LOCALAPPDATA%\hv\hlp-viewer.cfg` (falling back to `%APPDATA%`); other platforms use the XDG/user config directory when available. Missing configuration is treated as a normal first run.
- Added regression coverage for the exact CALC hosted-button descriptor, modern `0x24` inline framing, ordinary inline graphics, native blank-button geometry, and MRU parse/record/serialization behavior.

## 0.7.1 build-fix 20 - 2026-07-25

- Corrected popup-hover semantics. Build-fix 18 had restored only a destination label such as `Popup: Topic 6`; popup-marked hotspots now show the **actual visible text of the resolved popup topic** in the native hover tooltip.
- Extract popup tooltip text from the formatting-decoded `TopicPresentation` used by the renderer, preserving paragraph boundaries, explicit line breaks, and tabs while ignoring non-text objects rather than inventing descriptions for them.
- Keep ordinary navigation hotspots unchanged: they still show the resolved destination topic title on hover. Internal TOPICOFFSET, context-hash, and cross-file popup targets all use the same popup-body preview path.
- Empty/textless popup topics fall back to the resolved destination title so the native tooltip is never blank. Macro hotspots remain tooltip-free under the existing execution policy.
- Kept build-fix 16's single-surface click-routing policy unchanged; this correction changes hover content only and does not recreate detached popup/secondary frames.
- Added regression coverage proving that popup hover selects body text, normalizes CR/LF, falls back safely for empty bodies, and derives text from the decoded presentation rather than the synthetic `Topic N` label.

## 0.7.1 build-fix 19 - 2026-07-25

- Fixed the recursive table-picture walker after `TableCellContent::Table` moved to `Box<FormattedTable>`: the document resolver now explicitly dereferences the boxed nested table and recurses through `nested.cells.as_mut_slice()`.
- Removed the now-unused `FormattedTable` import from `hlp/src/document.rs`.
- Removed the stale unused `content_width` local from `hlp/src/layout.rs`; this is warning cleanup only and does not change layout calculations.
- No formatting, tooltip, popup, navigation, decoding, or rendering semantics were changed by this build-only correction.

## 0.7.1 build-fix 18 - 2026-07-25

- Restored the pre-build-fix-16 destination-title hover presentation on the **main fixed and scrolling help canvases**. Text links and graphical/image hotspots once again resolve their destination topic title into the native hover tooltip.
- Restored the authored popup distinction as hover/status metadata: popup-marked destinations show `Popup: <topic title>` in the tooltip and `Popup link: ...` in the status bar, while ordinary destinations retain the plain topic title / `Topic link: ...` presentation.
- Kept build-fix 16's single-surface navigation policy intact. Hover metadata does not create UI surfaces; activating popup-marked links, secondary-window targets, `.CNT` window qualifiers, or default-window assignments still routes through the main viewer, and no popup/secondary `Frame` is constructed.
- Preserved cross-file and context-hash destination-title resolution, tooltip generation invalidation across navigation, click-to-clear behavior, and the policy that macro hotspots remain tooltip-free.
- Left build-fix 17's executable-derived character-command parser unchanged. In particular, `0x8B`/`0x8C` remain rejected in the audited KB917607 scanner rather than being reintroduced merely to mask a viewer-layer regression.
- Added pure regression tests for ordinary versus popup tooltip labels so future single-surface UI work cannot silently erase the popup-aware hover presentation again.

## 0.7.1 build-fix 17 - 2026-07-25

- Re-audited the exact 285,696-byte KB917607 `winhlp32.exe` reference and corrected a major character-stream misconception: bytes `0x20` and `0x21` are TOPICLINK record types, not inline `VariableField`/`DType` commands. The Microsoft scanner rejects `0x20`, `0x21`, `0x8B`, and `0x8C` as character commands, so the Rust decoder no longer consumes fabricated payload bytes for them.
- Implemented the real previously-missing character command `0x85`. WinHlp32 consumes a signed WORD and stores it in transient layout state without emitting a glyph; the viewer now retains it losslessly as a zero-width `Control85` marker.
- Completed the compact non-text dispatcher families identified in the Microsoft executable. Old/modern `0x03`/`0x22` graphics reuse the existing indexed/embedded WinHelp graphics decoder both at top level and recursively inside table cells; `0x05`/`0x24` hosted/custom-window records retain their six-byte prefix and descriptor but never execute authored native controls, instead receiving a bounded viewer placeholder; old `0x06` is explicitly retained as a no-render record.
- Corrected compact-picture selector semantics from the executable: selector zero is an indexed `|bmN` resource using a signed WORD index (negative values are rejected), while every nonzero selector denotes an embedded logical graphics stream.
- Distinguished layout-safe diagnostics from structurally unsafe decode failures. Exact bounded omissions such as disabled hosted controls and unresolved hotspot action variants no longer force the complete formatted record to be replaced by plain text.
- Hardened the full structural hotspot envelope accepted by WinHlp32. Unknown fixed `C0`-family variants consume their exact four-byte payload; unknown variable `C8`-family variants consume their exact WORD-sized payload, preserving following text/layout while disabling only the unresolved link action.
- Corrected known `0xC8`/`0xCC` macro-hotspot framing: the following WORD is the payload length itself, not a total record size including the three-byte command header. The old `length - 3` interpretation could leave three macro bytes behind to be misread as formatting commands.
- Reclassified the two trailing paragraph-border bytes and paragraph flag bit 0 after tracing their consumers: both are still retained losslessly, but no visual read was found in the verified renderer path. Border styles 5-7 likewise have zero clearance and no defined style setup; the viewer now leaves those reserved values unpainted instead of inventing a normal border.
- Corrected the speculative modern/MVB font model from the executable. WinHlp32 always indexes 11-byte descriptors; only the fixed face-name slot changes from 20 bytes to 32 bytes for minor version 33. The old 42-byte descriptor/style-directory/character-map interpretation has been removed rather than preserved as false compatibility.
- Cross-checked that corrected font model against the retained 1995 Calculator HLP: minor version 33 uses two 32-byte face slots followed by nine 11-byte descriptors, and its `|SYSTEM` records expose locale `0x0416` plus a per-face charset-byte table.
- Reinterpreted modern `|SYSTEM` records 9 and 11 from their actual consumers. Record 9 is a ten-byte locale record whose final WORD gates Arabic/Hebrew reordering; record 11 is a per-face GDI charset byte table indexed through each font descriptor's face index, not one global 16-bit charset value.
- Added Windows-1255 and Windows-1256 LinkData2 decoding for explicit charset `0xB1`/`0xB2`, carried that charset into retained text styles, and reproduced the reference's face-charset run reordering for Arabic/Hebrew locales while leaving glyph shaping to the native text painter.
- At build-fix 17 time, GDI-style charset inference and broader DBCS/code-page decoding remained explicit gaps; build-fix 34 subsequently implements the common deterministic families, leaving rare Johab/OEM selection and platform-native shaping as the remaining edge cases.

## 0.7.1 build-fix 16 - 2026-07-25

- Removed native floating help-topic windows from the viewer. Popup hotspots, secondary-window targets, `.CNT` window qualifiers, default topic-window assignments, and popup macros still resolve their intended destination, but every resolved topic is now installed in the single main help surface. This preserves link functionality and Back/Forward history without creating detached frames.
- Removed the visible distinction between popup and ordinary topic destinations in hover/status text, since the viewer no longer presents a separate popup surface.
- Normalized compact border-only top+bottom separator paragraphs used before headings such as `Tópicos relacionados`: they now paint as one horizontal rule, align to the same content edge as the following unindented heading, and reserve a deliberate 12-pixel post-rule gap.
- Reworked the visible browse toolbox spacing. The Contents button is slightly wider, compact buttons use a consistent width, logical button pairs have 4-pixel internal gaps, groups have 10-pixel gaps, and the bar has explicit 5-pixel top/bottom margins instead of platform-dependent padding.
- Extended the retained-layout separator regression to assert the new 12-pixel separator height and left-edge alignment while preserving the authored right-side inset.
- Preserved build-fix 15 hanging-indent behavior and `.CNT` hierarchy selection unchanged.

## 0.7.1 build-fix 15 - 2026-07-25

- Corrected hanging first-line indentation in retained paragraph layout. Negative `first_line_indent` values were initially applied when the line was created, but `finish_line()` then clamped every positioned box back to the unindented paragraph edge. The clamp now uses the line's actual starting x-coordinate, so an authored outdent survives on the first visual line while wrapped continuation lines return to the ordinary left indent.
- Removed the synthetic 16-pixel text-line fallback from border-only paragraphs. Empty authored rule/container paragraphs now derive their height from their real border clearances instead of inventing a blank text row between top and bottom rules. This specifically collapses the excessive gap between the two authored horizontal rules above `Tópicos relacionados` in CALC.HLP without deleting either rule or synthesizing table grid lines.
- Added headless regressions proving that a negative first-line indent is preserved only on the first wrapped line and that a normal top+bottom border-only paragraph is 10 pixels high at the default DPI (5-pixel Microsoft clearance on each side, no fake 16-pixel line).
- Kept the existing `.CNT` sidecar as the authoritative Contents hierarchy whenever one is present. Physical decoded-topic order remains only the fallback for HLP files with no discovered `.CNT`; no flattening of authored books/topics was introduced.
- Preserved build-fix 14's transient tooltip/popup behavior and build-fix 13's recursive table implementation unchanged.

## 0.7.1 build-fix 14 - 2026-07-25

- Clarified the Microsoft table rendering model from the verified KB917607 `winhlp32.exe`: `0x414F66` is a geometry/dispatch routine only. It computes column x/width values and recursively dispatches cell TOPICLINK records but does not issue GDI line/rectangle drawing calls. A WinHelp table therefore does **not** imply a visible grid. In the Calculator keyboard-equivalents topic, the four aligned text flows are the table; the long horizontal rules are authored paragraph borders inside cells, not synthetic table borders.
- Kept build-fix 13's recursive table implementation unchanged rather than adding non-native grid lines. This preserves WinHlp32 fidelity for table type 0 proportional geometry, nonzero absolute geometry, independent column cursors, and recursively nested cells.
- Made destination-title hover tooltips transient on click. Main-topic clicks immediately clear both native canvas tooltips and invalidate the hover cache; auxiliary-topic clicks do the same for their canvas.
- Added a generation counter to tooltip hit caches so changing topic/document cannot leave a stale native tooltip or prevent the same logical hotspot from producing a fresh tooltip in the new topic.
- Added explicit main-window outside-click dismissal for transient WinHelp popup-topic windows. Clicks on the navigation tree/lists/search fields, browsing controls, document chrome, toolbar, or frame close the active popup while preserving the original control event via `Skip`.
- Track the active popup in `ViewerState`. Opening a new popup replaces the old one, and the popup close handler clears the tracked handle. Secondary WinHelp windows remain persistent and are never affected by transient-popup dismissal.
- Main-topic/document/history navigation now dismisses both hover tooltips and the active transient popup before changing the displayed topic, including Contents/Index/Search/Bookmarks/History routes.

## 0.7.1 build-fix 13 - 2026-07-25

- Implemented recursive nested-table retention and layout from the verified Microsoft KB917607 `winhlp32.exe`, extending build-fix 12 rather than replacing its table framing.
- Re-traced the recursive call chain: table walker `0x414F66` calls generic compact-record dispatcher `0x417578` for each cell at `0x4151B6`; dispatcher types `0x04`/`0x23` call straight back into `0x414F66` at `0x4175E3..0x4175F1`. Nested tables therefore use the same parser/layout routine as top-level tables.
- Retain every table as a real cell tree. `TableCellContent::Display` points to one bounded range in the owning record's flat paragraph store, while `TableCellContent::Table` owns another `FormattedTable` recursively. This avoids duplicating mutable pictures/hotspots while preserving exact nesting.
- Match WinHlp32's recursive geometry: a nested table receives its parent cell's x/y origin and column width; its returned height is added only to that parent column's cumulative vertical cursor. The Microsoft parent performs that update at `0x4151DC..0x4151E9`; the Rust layout now does the same at every nesting depth.
- Preserve type-0 minimum-width/proportional geometry and nonzero absolute geometry independently at each nested level, so a type-0 child can expand beyond the immediate parent width exactly as the reference calculation permits.
- Keep one shared LinkData2 string stream and running font selection through recursive dispatch. Text after a nested table therefore consumes the next string/font state after the entire nested subtree, matching the executable's depth-first dispatch order.
- Added a 64-level defensive recursion cap for hostile files. The reference executable recursively re-enters its dispatcher without a format-level depth field; the cap is an engine safety bound and turns an over-deep subtree into the existing bounded diagnostic/fallback path.
- Added parser regression coverage for `0x23 -> 0x23 -> 0x20` recursion followed by a sibling outer display cell, plus old-generation `0x04 -> 0x04 -> 0x01` recursion with no modern TopicLength field. These prove exact compact-payload return boundaries and shared LinkData2 ordering. Added retained-layout coverage proving a nested table's maximum child-column height advances only its containing parent column.
- Uncommon non-display compact cell renderer families remain bounded diagnostics; this build specifically completes recursive `0x04`/`0x23` table support.

## 0.7.1 build-fix 12 - 2026-07-25

- Rebased exclusively on the user-supplied build-fix 11 source. Build-fix 11 is now the implementation baseline; none of the build-fix 10 package was used as a replacement tree.
- Fixed the wxWidgets hidden-root assertion shown in `treectrl.cpp`: topic/Contents synchronization no longer calls `TreeCtrl::ensure_visible()` on a tree created with `HideRoot`. The viewer now expands only real authored book ancestors and calls `scroll_to()` for the selected item, so wxMSW never receives an expand request for its virtual hidden root.
- Reverse-engineered table-cell framing directly from the manifest-verified Microsoft KB917607 `winhlp32.exe`. The table walker at `0x414F66` reads a signed 16-bit column index, dispatches a complete nested compact TOPICLINK record through `0x417578`, advances by the exact compact-header plus payload size decoded by `0x412884`, and terminates on column `-1`.
- Added Windows 3.0 table record type `0x04` alongside the Windows 3.1+ `0x23` table record. Both now reach the retained table parser/layout path, with old `0x01` and modern `0x20` display cells decoded inside them.
- Removed the old fixed five-byte table-cell prelude guess. Table cells now consume their actual bounded nested record header, preventing the nested type/size bytes from being misread as paragraph metadata.
- Implemented the compact-header generations used by the Microsoft decoder: ordinary `0x01..0x06` / `0x20..0x24` records use the compressed signed payload length (and modern compressed TopicLength), while `0x02` / `0x21` use the fixed-width DWORD-size header selected by `0x412884`. Every nested payload is bounded before decoding so malformed cell sizes cannot run into the following cell.
- Preserved build-fix 10/11's Microsoft table geometry: 32-column cap, type-0 minimum width and two-stage 32767-unit proportional conversion, absolute DPI/144 nonzero types, width-before-gap records, and independent per-column vertical flow.
- Added headless regression fixtures for a modern `0x23` table containing `0x20` display cells and a Windows 3.0 `0x04` table containing an old `0x01` display cell. Unsupported compact cell families remain bounded and diagnostic rather than guessed; recursively nested table retention/rendering remains a separate hardening item.

## 0.7.1 build-fix 11 - 2026-07-25

- Fixed spurious bold body text. The selected font is running state in WinHlp32, held in the global at `0x43C2C4`, initialised once per topic render at `0x41B05D` and written only by character opcode `0x80` at `0x41AB8C`; neither paragraph terminator (`0xFF` at `0x41ABEB`, the `0x81`/`0x82`/`0x83` path at `0x41AB7A`) clears it. `parse_character_stream` reset the selection to descriptor 0 for every paragraph, so each paragraph that reused the previous paragraph's font inherited the file's first descriptor - usually the bold heading face. The selection is now threaded through paragraphs and across the records of one region via the new `FormattedRecord::decode_with_font`. Traced addresses are recorded in `docs/WINHLP32_FORMATTING_REFERENCE.md`.
- Stopped per-run opaque text output from shaving glyphs. Anti-aliased glyph edges extend about a pixel past their character cell, so drawing every run with an opaque background erased the right edge of the run before it, which read as clipped letters and broken anti-aliasing. The run background is now filled only when the descriptor asks for a colour different from the page, and the text itself is drawn transparently over it; authored highlight backgrounds are unchanged.
- Substituted legacy raster faces with modern outline equivalents on Windows. `Helv`, `MS Sans Serif`, `System`, `Small Fonts` and the `MS Shell Dlg` aliases now resolve to Segoe UI, `Tms Rmn`/`MS Serif` to Times New Roman, and `Courier`/`Terminal`/`Fixedsys` to Consolas. GDI cannot anti-alias a bitmap strike and scales missing sizes by pixel replication. Outline faces chosen by the author (Arial, Times New Roman, Verdana, Tahoma, Courier New) are left untouched.
- Removed the help-file title row from the Contents pane (`TR_HIDE_ROOT`), matching WinHelp's Contents tab. MSW's `TVM_ENSUREVISIBLE` horizontally scrolls a tree by one indent level to bring a selected child's label flush with the client edge, which sheared the leading characters off the root row. `wxTR_LINES_AT_ROOT` is kept alongside it because MSW's common control only draws expand buttons on top-level items when `TVS_LINESATROOT` is set, and authored `.cnt` books become top-level rows once the root is hidden. The two `expand()` calls on the root are gone - `wxTreeCtrl::DoExpand` asserts on a hidden root - and the "Open an HLP file" placeholder is now a real child row, because wxWidgets discards a hidden root's label.
- Saturated the popup background to `RGB(255, 255, 184)`. Same hue as before, taken to full HSL saturation so popups read as yellow rather than olive.
- Centred the browsing bar by giving it stretch spacers at both ends.
- Added headless regression coverage proving the font selection survives both a paragraph boundary and a record boundary.

## 0.7.1 build-fix 10 - 2026-07-25

- Reverse-engineered paragraph/font/border formatting directly from the manifest-verified Microsoft KB917607 `winhlp32.exe` instead of inferring the remaining fields from third-party implementations. The traced addresses and unresolved fields are documented in `docs/WINHLP32_FORMATTING_REFERENCE.md`.
- Fixed paragraph flag bit 7: it carries a compressed signed default-tab interval and defaults to 72 source units when absent. Consuming this field also prevents later border/tab fields from becoming desynchronized.
- Replaced the old font-generation-dependent paragraph metric conversion with WinHlp32's direct signed `raw * device-DPI / 144` integer conversion. Horizontal indents/tabs use horizontal DPI; vertical spacing uses vertical DPI; negative spacing and negative left/right indents are preserved.
- Corrected the two-bit alignment field so values 1 and 3 are right-aligned and only value 2 is centered.
- Implemented paragraph bit 12 as Microsoft-style no-wrap for word and tab overflow. Identified bit 13 as the right-to-left paragraph flag: the Microsoft path applies first-line indentation from the right and enters Hebrew/Arabic run reordering for charsets `0xB1`/`0xB2`. Build-fix 10 applies the confirmed right-side first-line indent but leaves full bidi reordering as an explicit follow-up.
- Implemented signed line-spacing semantics: positive values are a minimum line advance, negative values are an exact line advance, and zero uses the natural measured line extent.
- Implemented deferred right/center tab layout: text after a right tab ends on the stop, text after a center tab is centered on it, and absent custom stops advance by the authored/default tab interval.
- Implemented classic small-caps sizing exactly as the old WinHlp32 font builder does: attribute `0x20` uses a two-thirds-height font. The classic `0x10` double-underline flag remains parsed but is not synthesized because Microsoft's old 11-byte font path does not consume it.
- Reinterpreted the border high bits as one three-bit style code rather than independent booleans. Added normal, thick, double, shadow, style-4, and reserved handling; Microsoft-style 5/6/7-pixel text clearance is now applied per border side, so borders affect layout as well as painting.
- Stopped treating the final two bytes of the three-byte border record as an authored pen width. They are retained raw until their complete mode-dependent semantics are established.
- Corrected the paragraph prelude to consume WinHlp32's variable two/four-byte compressed signed long instead of blindly skipping two bytes; long-form paragraph records no longer desynchronize their id/flags and optional fields.
- Corrected WinHelp's extended compressed signed-long bias from the previous `0x04000000` equivalent to Microsoft's exact `(DWORD >> 1) - 0x40000000` rule at `0x4129e8`; extended paragraph/picture values now decode correctly.
- Reverse-engineered type-`0x23` table layout from WinHlp32: column count is capped at 32; type 0 alone carries a minimum-width word; each unsigned column record is `width` then `gap-before`; type 0 uses the 32767-unit proportional reference span while nonzero types use absolute DPI/144 metrics.
- Replaced the HTML-like row approximation with WinHlp32's independent per-column vertical flow, so a tall cell advances only its own column and overall table height is the maximum column cursor.
- Added headless regression coverage for default tabs, alignment/no-wrap flags, signed line spacing, right/center tabs, small-caps height, DPI/144 paragraph metrics, border style decoding/clearance, long-form paragraph preludes, table header ordering, table conversion modes, and independent column flow.

## 0.7.1 build-fix 9 - 2026-07-25

- Rebased the presentation fixes on the user-extracted, manifest-verified Microsoft KB917607 `winhlp32.exe` reference (285,696 bytes; SHA-256 `8496f19bc1d898180b57aac61326bdfcf5a48f760811283bbd604aa7a2c13f85`).
- Replaced build-fix 8's broad `RGB <= 8` near-black normalization with WinHlp32's exact colour sentinel rule: descriptor `RGB(1,1,0)` / COLORREF `0x00000101` inherits the current text colour, while every nearby dark or purple authored colour remains untouched.
- Retain and paint each `|FONT` descriptor's background colour. Exact sentinel backgrounds inherit the actual main/popup/secondary page colour; authored backgrounds use opaque native text output, matching WinHlp32's GDI text path.
- Preserve raw twentieth-point font sizes through retained layout and apply zoom before final native point rounding. This keeps HC30 half-point sizes such as 7.5 pt from being rounded prematurely and improves WinHlp32-compatible wrapping and vertical geometry.
- Restored the requested 110% initial text zoom while keeping 70%-200% 10% controls and synchronized native reflow/hit testing.
- Added regression coverage proving colour inheritance is exact (not a near-black threshold), authored dark/purple colours survive, and foreground/background inheritance state is retained independently.

## 0.7.1 build-fix 8 - 2026-07-25

- Replaced build-fix 5's broad purple/blue plain-text colour heuristic with a narrow canonicalization of compiler-emitted near-black values (`RGB <= 8` on every channel). Authored purple/blue text is now preserved, while the Calculator reference HLP's `RGB 1,1,0` body text renders as exact black.
- Preserve the original WinHelp Roman/Swiss/Modern/Script/Decorative family classification in retained text styles. Windows native substitution now maps Roman text to Times New Roman, Swiss text to Microsoft Sans Serif, Modern/fixed-pitch text to Consolas, Script text to Segoe Script, and keeps symbol/decorative faces when glyph identity matters.
- Restored authored 100% point sizes as the default zoom instead of build-fix 3's forced 110% enlargement. User zoom controls remain available from 70% through 200%.
- Removed the unconditional 16-pixel minimum line advance for populated text lines. Paragraph reflow now advances by the native measured font height, with 16 pixels used only as an empty-line fallback.
- Corrected old-format half-point paragraph metrics to WinHelp's historical half-unit rounding before device conversion, improving indents, spacing, tab geometry, and table-like alignment.
- Added `tools/extract_winhlp32_kb917607.ps1`, which extracts the supplied Windows 8.1 KB917607 MSU on Windows, reconstructs its PA30 `winhlp32.exe` through Microsoft's `msdelta.dll` `ApplyDeltaB`, and verifies the target size/SHA-256 from the update manifest. The Microsoft executable is used only as a local reference and is not redistributed with this source package.

## 0.7.1 build-fix 7 - 2026-07-25

- Reworked the visible browse row into a compact arrow-based control strip instead of wide text buttons: history uses `←`/`→`, physical topic navigation uses `◀`/`▶`, authored browse navigation uses `⇤`/`⇥`, navigation-pane toggle uses `☰`, and zoom remains `−`/`+`.
- Added concise tooltips for every symbolic control and compact minimum widths so the bar reads like navigation chrome instead of a form.
- Authored `BrowseButtons()` controls are now hidden until the HLP actually requests them, then appear in-place and follow the existing enabled-state logic.
- Added small visual gaps between navigation groups while retaining the same command handlers, keyboard shortcuts, and document-state synchronization introduced in build-fix 5/6.

## 0.7.1 build-fix 5 - 2026-07-25

- Replaced the unreliable text-only frame toolbar with an explicit visible in-window browse-button row (`Contents`, `Back`, `Forward`, `Previous`, `Next`, `<<`, `>>`, navigation toggle, `-`, `+`) while keeping the existing command wiring and status logic.
- Made the Contents tree react to normal selection changes as well as activation, so fallback entries such as `Topic 6` / `Topic 7` open when clicked instead of appearing inert.
- Gave popup note windows a darker default paper shade than the main help page, closer to classic WinHelp notes, while still honoring explicit popup/window colours from the HLP when present.
- Added a conservative plain-text colour normalization pass that collapses stray purple/blue non-hotspot text to ordinary black, while preserving green hotspot emphasis and all non-affected colours.

## 0.7.1 build-fix 4 - 2026-07-25

- Kept the native viewer interface from build-fix 3, but framed the help page more professionally: the content host now uses a light gray background while the actual help page sits inside a black 1-pixel border, matching the supplied reference more closely.
- The pale `RGB 255,255,228` WinHelp-style page background remains inside that border, so only the surrounding chrome changed.
- The framed page layout is recomputed when the main window resizes or the navigation pane is shown/hidden, keeping the border and margins stable.

## 0.7.1 build-fix 3 - 2026-07-25

- Enlarged HLP content text slightly by default (110%) while preserving each authored run's relative size, weight, italics, underline, strikeout, colour, and proportional/monospace intent.
- Added text-only `-` and `+` browsing-toolbar controls for 10% zoom steps from 70% through 200%; every zoom change reflows native text metrics so wrapping and hyperlink hitboxes remain aligned.
- Made the frame-owned browsing toolbar explicitly visible and reserved a native toolbar row so it cannot collapse to an effectively hidden height on startup.
- Changed the default help-content surface to the pale WinHelp-style background used by the supplied visual reference (`RGB 255,255,228`), while explicit auxiliary-window/popup colours still take precedence.
- The status bar now shows the current text zoom beside the active topic number.

## 0.7.1 build-fix 2 - 2026-07-25

- Fixed correctly resolved popup topics that rendered blank when their display records fell outside the fixed/scrolling TOPICPOS ranges advertised by the topic header.
- `TopicPresentation` now recovers displayable `unclassified` records into the scrolling body instead of silently dropping them, preserving TOPICPOS order relative to ordinary scrolling records.
- Unknown/non-display housekeeping records remain excluded, and recovered content emits a non-fatal diagnostic warning.
- Added regression coverage for an all-unclassified popup body and ordering between recovered and normally classified body records.

## 0.7.1 - 2026-07-25

- Fixed real-world internal hyperlinks such as the Calculator HLP `operadores` link being misreported as `Unresolved internal TOPICOFFSET 1133400500`.
- Split LinkData1 `0xE0`/`0xE1` physical TOPICOFFSET links from `0xE2`/`0xE3`/`0xE6`/`0xE7` context-hash links.
- Context-hash links now resolve through the document `|CONTEXT` table and then enter the existing main/popup/secondary-window navigation paths.
- Retained a physical-offset fallback for nonstandard producers that encode the latter opcode family as TOPICOFFSETs.
- Updated hover tooltips and diagnostics to understand context-hash destinations.
- Added regression tests for visible and invisible context-hash hotspot decoding.

## 0.7.0 - 2026-07-24

- Added a GUI-independent WinHelp macro parser with a typed `HelpMacro` / `SafeHelpMacro` model, legacy short-name aliases, semicolon-separated programs, legacy quote handling, decimal/hex 32-bit integers, nested-call preservation, and explicit parser resource limits.
- Added a default-deny safety classifier: process/shell operations (`ExecFile`, `ExecProgram`, `ShellExecute`, `ShortCut`, `ControlPanel`), DLL registration (`RegisterRoutine`), host-interaction macros, unknown names, and malformed/invalid invocations are never dispatched.
- Execute allow-listed viewer-local macros from SYSTEM CONFIG records, per-topic macro records, and macro hotspots in the main viewer, popups, and secondary windows.
- Implemented safe navigation/UI commands including About, Back/BackFlush, bookmarks, Contents, Finder/Index, Search, History, FocusWindow where resolvable, JumpContents/JumpContext/JumpHash/JumpID, PopupContext/PopupHash/PopupId, authored Next/Prev, BrowseButtons, and SetPopupColor.
- Added `HelpDocument::topic_index_for_context_hash` so `JumpHash`/`PopupHash` resolve through the same bounded navigation metadata as ordinary links.
- `BrowseButtons()` now adds distinct native Browse Prev/Browse Next toolbar tools backed by authored browse metadata; physical Previous/Next remains unchanged.
- Added a shared 128-command macro execution budget to bound recursive/cyclic topic/config navigation, a 512-entry diagnostic cap, and a 2,048-character cap per retained diagnostic.
- Preserved non-ASCII macro string arguments losslessly and suppress stale source-topic macro execution when a CONFIG macro replaces or closes a popup during startup.
- Added `View > WinHelp Macro Diagnostics...` and enhanced `--dump-file --verbose` to classify macro strings as ALLOW/BLOCK without executing them.
- `SetPopupColor()` affects only viewer-owned popup painting and cannot mutate host/system state.
- Preserved the 0.6.2 browsing toolbar/navigation-pane controls, 0.6.1 destination tooltips, and flattened build/cache layout.

## 0.6.2 - 2026-07-24

- Added a native wxWidgets browsing toolbar with Back, Forward, Contents, Previous, and Next controls, reusing the existing navigation command path rather than duplicating navigation behavior.
- Added a checkable Navigation toolbar control plus `View > Navigation Pane` (`F9`) to show or hide the entire Contents / Index / Search / Bookmarks / History side panel.
- Hiding or restoring the navigation pane immediately reflows the active topic into the newly available width and returns keyboard focus to the topic surface.
- Toolbar Back/Forward availability follows the live history stacks, while Previous/Next follows physical topic boundaries and document-dependent commands remain disabled until a help file is open.
- Preserved the 0.6.1 hyperlink destination tooltips, popup/secondary-window behavior, and the 0.6.0-buildfix3 flattened source/cache layout.

## 0.6.1 - 2026-07-24

- Added native hover tooltips for text hyperlinks and graphical/image hotspots in the main topic surface, popup topics, and secondary help windows.
- Tooltip text resolves the destination topic title from the actual HLP navigation target; cross-file links open the referenced HLP for title resolution using the same relative-path rules as activation.
- Popup destinations are identified as `Popup: <topic title>`; ordinary jump links show the destination title directly.
- Ordinary non-link text and blocked executable macro hotspots do not expose a tooltip, and leaving a hotspot clears the native tooltip immediately.
- Hover resolution is target-change driven, avoiding repeated tooltip replacement and repeated cross-file HLP opens for every mouse-motion event while the pointer remains over one hotspot.
- Preserved the 0.6.0-buildfix3 flattened source layout and compact external wxDragon/Cargo cache strategy.

## 0.6.0 build-fix 3 - 2026-07-24

- Flattened the workspace directories from `crates/hlp`, `crates/hlp-viewer`, and `crates/wmf-render` to `hlp`, `viewer`, and `wmf`; Cargo package names and public crate identities are unchanged.
- Changed the default external Cargo/native cache root from `%LOCALAPPDATA%\RustHlpViewer\cargo-target` to the much shorter `%LOCALAPPDATA%\hv` (with `%TEMP%\hv` fallback).
- Repacked the source ZIP without an additional top-level project directory, so Windows “Extract All” produces one project directory rather than a project directory inside another project directory.
- Kept `HLP_VIEWER_TARGET_DIR` as an override and updated all cleanup scripts to use the same compact cache root.
- No HLP parser, navigation, rendering, search, or UI behavior changed.

## 0.6.0 build-fix 2 - 2026-07-24

- Moved the default Cargo target directory used by `build_hlp.bat` to a short, stable per-user path under `%LOCALAPPDATA%\RustHlpViewer\cargo-target` (with a `%TEMP%` fallback), preventing wxDragon/wxWidgets CMake scratch and generated manifest/resource paths from inheriting a deeply nested extracted-source path.
- Added the optional `HLP_VIEWER_TARGET_DIR` override for users who need to place the project-specific Cargo/native cache elsewhere.
- Updated `clean_source.bat`, `clean_all.bat`, and `clean_tmp_files.bat` for the external short-path cache while retaining cleanup compatibility with legacy in-tree `target\` directories.
- This specifically addresses the Windows CMake compiler-probe failure where path-length warnings were followed by `manifest.rc(3) : error RC2136 : missing '=' in EXSTYLE=<flags>`.
- No Rust source, HLP parsing, navigation, rendering, search, or UI behavior changed.

## 0.6.0 build-fix 1 - 2026-07-24

- Corrected wxDragon `TreeCtrl::get_custom_data` calls to pass `&TreeItemId`, matching wxDragon 0.9.17's item-data API and fixing the release-build `E0277` errors in Contents synchronization/activation.
- Removed six redundant `y = line.y` assignments from paragraph layout; `LineState::y` is already the authoritative constrained vertical position and later transitions derive directly from it.
- Test-gated the formatting-only compressed-unsigned-long helper so normal library builds no longer report it as dead code while its unit coverage remains intact.
- No navigation, graphics, Contents/Index/Search semantics, or file-format behavior changed.

## 0.6.0 - 2026-07-24

- Added `.CNT` sidecar discovery from `|SYSTEM` with same-basename fallback, CP1252 decoding, hierarchical book/topic entries, `:Title`, `:Base`, `:Index`, `:Link`, external-help targets, and named-window targets.
- Added a native wxDragon navigation notebook with Contents, Index, Search, Bookmarks, and History pages; the Contents tree tracks the active same-file topic when its authored target is resolvable.
- Added WinHelp `|?WBTREE` / `|?WDATA` authored-keyword decoding over the generic HLP B+ tree page structure, including multiple TOPICOFFSET targets per keyword and non-fatal malformed optional tables.
- Added a deterministic in-memory search index over decoded title/body text plus authored K-keywords; exact/prefix title and keyword matches rank above body matches.
- Merge Index and Search results across the current document and one-hop `.CNT` `:Index` / `:Link` help files while keeping each result file-qualified for cross-file navigation/history; sidecar-driven linked-file expansion is bounded to 32 unique files and automatic absolute/UNC catalog references are skipped.
- Added native multi-topic keyword selection instead of arbitrarily choosing the first matching topic.
- Added session bookmarks and a visible Back/current/Forward history list using the existing `NavigationLocation` model.
- Optional contents/keyword/linked-file failures remain warnings and do not prevent the base HLP from opening; recursive `.CNT` `:Include` expansion remains explicitly unsupported and warning-only.
- Preserved the 0.4.5 physical Left/Right topic navigation and the complete 0.5.1 raster/WMF/image-hotspot/float graphics path.

## 0.5.1 - 2026-07-24

- Added legacy Windows metafile (`type 0x08`) decoding and Windows GDI rasterization through a dedicated `wmf-render` adapter; the `hlp` parser remains GUI-independent and denies unsafe Rust.
- Decode metafile mapping mode, authored extents, compressed payload size, bitmap/hotspot offsets, and all existing WinHelp graphics packing modes.
- Added graphical hotspot-table parsing for macro (`0xC8`), same-file popup/jump (`0xE6`/`0xE7`), and named-window popup/jump (`0xEE`/`0xEF`) records.
- Resolve graphical context-name targets through the existing navigation metadata and expose image hotspots as retained invisible hit-test boxes, including scale-to-fit coordinate correction.
- Added true `bml` (`0x87`) and `bmr` (`0x88`) paragraph floats: text wraps in the remaining horizontal span while vertically overlapping the image and returns to full width beneath it.
- Corrected ordinary external text-hotspot dispatch to the same WinHelp low-bit rule used by graphical hotspots: even `0xEA`/`0xEE` are popups; odd `0xEB`/`0xEF` are navigation links.
- Corrected type-6 external text-hotspot parsing so the authored secondary-window name is read before the external help filename, matching the WinHelp record layout.
- Kept graphical macro hotspots inert and routed them through the viewer's existing blocked-macro feedback.
- Added headless regression fixtures for graphical hotspot records, image-hotspot scaling/hit testing, metafile extent conversion, and left/right float layout.

## 0.5.0 - 2026-07-24

- Added real raster image rendering for WinHelp topic picture commands (`0x86`..`0x88`) instead of fixed placeholders.
- Resolve indexed images from internal `|bmN` streams and embedded `bmcwd`/`bmlwd`/`bmrwd`-style graphics objects.
- Decode WinHelp bitmap alternatives stored as DIB/DDB records with raw, RLE, LZ77, or LZ77-then-RLE packing.
- Decode 1/4/8/16/24/32-bit DIB pixels, DIB palettes, bottom-up scan lines, and WinHelp's flagged transparent palette entry; portable 1/16/24/32-bit DDB variants are also accepted.
- Preserve decoded pixels in GUI-independent RGBA buffers and render them through native wxDragon bitmaps; images wider than the topic viewport are proportionally reduced.
- Reuse decoded indexed-image buffers across repeated references and enforce dimension, decoded-byte, palette, and picture-payload safety limits.
- Corrected picture-command synchronization: `PictureSize` is a compressed **signed** long, while the type-`0x22` hotspot count remains a compressed unsigned short.
- Keep unsupported WMF-only graphics, unsupported DDB palette cases, embedded element type `0x05`, and unresolved image records as non-fatal placeholders with document warnings.
- Graphical image-hotspot maps and true `bml`/`bmr` floating text wrap remain deferred; ordinary text hyperlinks are unchanged.

## 0.4.5 - 2026-07-24

- Fixed 0.4.4's unreliable plain Left/Right navigation by making the main topic canvases explicitly focusable and restoring focus to the scrolling topic surface after loading or changing topics.
- Clicking either main topic canvas now restores keyboard focus before hotspot hit testing.
- Replaced Navigate > Browse Previous/Next with Navigate > Previous Topic/Next Topic, mapped to the physical decoded topic index and shown as Left/Right.
- Removed Ctrl+PageUp/Ctrl+PageDown from the Navigate menu entirely.
- Kept authored HC30/HC31+/HCW browse metadata and `HelpDocument::browse_previous_index` / `browse_next_index` in the GUI-independent `hlp` engine for compatibility and future safe macro support.
- Preserved Alt+Left/Alt+Right Back/Forward history and Ctrl+Home Contents behavior.

## 0.4.4 - 2026-07-24

- Changed plain Left Arrow / Right Arrow navigation to move directly by the decoded presentation index, so Topic 1/N goes to Topic 2/N regardless of the HLP's optional authored browse sequence.
- Kept Alt+Left / Alt+Right as browser-style Back/Forward history.
- Replaced the history-only key binder with one native `KeyDown` handler attached to the frame, fixed topic canvas, scrolled window, and scrolling topic canvas.
- Consumed handled Left/Right key events instead of skipping them, preventing wxWidgets from performing a second scrolling/navigation action.
- Kept Navigate > Browse Previous/Next as the explicit way to follow the HLP-authored browse sequence.
- Added `clean_tmp_files.bat`, which removes Cargo/wxDragon build artifacts and leaves only `build\hlp-viewer.exe` as generated output.

## 0.4.3 - 2026-07-24

- Fixed direct `.HLP` opening selecting the `[OPTIONS]` Contents target (for example topic 12 in `CALC.HLP`) instead of the first displayable presentation topic. The Navigate > Contents command still resolves the authored Contents target explicitly.
- Added caller-supplied text metrics to the GUI-independent retained layout engine.
- Changed the wxDragon viewer to measure every retained text fragment with `get_full_text_extent()` using the exact native font object later used to paint it, eliminating the spacing drift caused by the old heuristic width estimator.
- Added a regression test proving caller-supplied text widths control whitespace and subsequent token positions.
- Restored browser-style Alt+Left / Alt+Right navigation with explicit key handlers on the frame and topic surfaces instead of relying on menu-label accelerator parsing.
- Kept Segoe UI/Consolas native font substitution, two-crate architecture, single executable, popups/secondary windows, and `--dump-file` behavior unchanged.
- Changed help/version strings to derive the package version where practical, reducing future version-string drift.

## 0.4.2 - 2026-07-24

- Consolidated the former `hlp-format`, `hlp-core`, and `hlp-render` packages into one GUI-independent `hlp` engine crate.
- Reduced the workspace from four Cargo packages to two: `hlp` and `hlp-viewer`.
- Moved loaded-document semantics, TOPICOFFSET resolution, navigation history, retained layout, and hit testing beside the parser instead of forwarding data across package boundaries.
- Kept wxDragon isolated in `hlp-viewer`, so `cargo test -p hlp` still avoids the expensive wxWidgets build.
- Consolidated CLI, console attachment, and dump-mode helpers into one viewer support module.
- Preserved 0.4.1 popup/secondary-window behavior, native-font substitution, and the single `--dump-file` executable design without file-format changes.

## 0.4.1 - 2026-07-24

- Refined WinHelp popups into transient native owned windows with hotspot-relative placement, no taskbar entry, Escape dismissal, and activation-loss dismissal.
- Made popup and secondary-window topic surfaces fully interactive: internal, external, cross-file, popup, explicit-window, and main-window hyperlinks now work from auxiliary windows.
- Kept ordinary navigation inside an existing secondary window unless the link explicitly targets another window; popup-to-popup links replace the current transient popup.
- Applied parsed secondary-window caption, colors, topmost, maximize, geometry, and auto-height metadata where available.
- Added a native Windows font policy: proportional text requests Segoe UI and fixed-pitch text requests Consolas while retaining HLP size, weight, italic, underline, strikeout, color, and pitch intent.
- Preserved original HLP face names in the semantic/render model and exempted symbol/dingbat faces from ordinary typeface substitution.
- Added fixed-pitch classification from HLP pitch/family metadata and retained-layout hotspot-box hit testing for popup positioning.
- Build-fix repack: updated the three complete `SystemInfo` test fixtures for the new `windows` field.
- Removed the redundant stored `DecodedLink.next_raw` field while retaining the local next-link value used during TOPICLINK traversal.
- Merged the former `hlp-dump` binary into `hlp-viewer.exe --dump-file <file.hlp>` with optional `--verbose`.
- Added pre-wxDragon CLI dispatch, positional HLP startup, help/version modes, and a narrow Windows parent-console attachment bridge for diagnostic output.
- Added HC30 `|TOMAP`, HC31+/HCW `|CONTEXT`, `|CTXOMAP`, `|TopicId`, and `|Viola` parsing.
- Added the WinHelp context-name hash and typed context/map/window navigation metadata.
- Added standard HC31/HCW SYSTEM WINDOW parsing for names, captions, geometry, colors, and flags.
- Added TOPICOFFSET reconstruction/resolution from transformed topic blocks and display-record TopicLength fields.
- Added internal topic jumps, popup-topic windows, cross-file jumps, external popup opcode handling, and explicit/default secondary windows.
- Added bounded Back/Forward history that can restore locations across HLP files.
- Added HC30 and HC31+ browse-sequence navigation and generation-aware contents-topic resolution.
- Added safer relative external-HLP path resolution without executing macros.
- Changed the build workflow to test parser/core/renderer crates before a single release wxDragon build, avoiding a redundant native debug/test wxWidgets build.
- Changed `clean_source.bat` to preserve Cargo/wxDragon caches and added `clean_all.bat` for deliberate full rebuilds.

## 0.3.0 - 2026-07-24

- Added HC30 and modern HC31+/HCW `|FONT` parsing with face names, descriptors, style flags, colours, families, and historical metric selection.
- Added semantic LinkData1 paragraph/table decoding with conditional spacing, indentation, alignment, borders, tab stops, and table columns.
- Added synchronized character-command decoding for font changes, line/tab commands, picture metadata, macros, and internal/external hotspots.
- Corrected type-`0x22` picture hotspot counts to use compressed unsigned-short decoding.
- Corrected external-hotspot `SizeOfFollowingStruct` handling so subsequent formatting commands stay synchronized.
- Added non-fatal formatting issues and per-record plain-text fallback in `hlp`.
- Added GUI-independent retained layout for wrapping, alignment, tables, borders, picture placeholders, and hotspot rectangles.
- Added fixed-vs-scrolling native topic canvases using wxDragon `PaintDC` and `wxScrolledWindow`.
- Added native font/colour painting, Previous/Next topic inspection, resize reflow, and hotspot hit testing.
- Extended `hlp-dump` with font and semantic formatting diagnostics.
- Added targeted formatting/layout regression tests while preserving the no-macro-execution policy.
- Hardened rectangle/font layout arithmetic against overflow and corrected Unicode tokenization plus first-line-indent alignment.

## 0.2.0 - 2026-07-24

- Added physical `|TOPIC` block decoding for HC30/HC31+/HCW block sizes.
- Added correct transformed TOPICPOS mapping using the per-block decompressed data size.
- Added standalone bounded WinHelp LZ77 decompression with overlapping-match support and invalid-backreference rejection.
- Added classic `|Phrases` parsing, including normal HC31+ compressed phrase images and the distinct MediaView/MVB 30-byte padded variant.
- Added Hall `|PhrIndex` / `|PhrImage` phrase reconstruction.
- Added classic and Hall LinkData2 phrase-token expansion, including Hall extended phrase-index multiplier.
- Added TOPICLINK traversal, cross-block record stitching, HC30 physical relative-pointer translation, pointer validation, and cycle/count/size guards.
- Added HC30 32-bit browse topic-number and HC31+ topic-header metadata parsing.
- Added typed `TopicPos`, `TopicOffset`, `TopicId`, `TopicMetadata`, `TopicRecord`, `Topic`, and `TopicStore` models.
- Added title extraction, inert topic macro retention, fixed/scrolled region classification, and plain visible text reconstruction.
- Exposed decoded topics through `hlp`.
- Extended `hlp-dump` with transformed blocks, phrase metadata, topic titles, regions, macros, and text previews.
- Updated the wxDragon viewer to open and report decoded topics rather than only container metadata.
- Added synthetic LZ77, phrase, TOPICPOS, cross-block, topic-header, and complete topic-grouping tests.
- Kept `LinkData1` formatting bytes intact for the 0.3 renderer rather than guessing at visual semantics.

## 0.1.0 - 2026-07-24

- Created the multi-crate Rust HLP viewer workspace.
- Added bounds-checked HLP container/header parsing.
- Added internal directory B+ tree traversal and cycle detection.
- Added internal `FILEHEADER` validation and stream lookup.
- Added `|SYSTEM` parsing for common WinHelp generations and metadata records.
- Added Windows-1252 decoding for system strings.
- Added macro-text retention with a strict no-execution policy.
- Added `hlp-dump`, the initial wxDragon shell, and GUI-independent geometry primitives.
