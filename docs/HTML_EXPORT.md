# Interactive HTML export architecture

Build-fix 77 changes the topic renderer to a direct **decoded WinHelp semantics -> ordinary HTML/CSS** translation. Earlier builds 69-76 reused retained `LayoutEngine` text boxes and then tried to reconcile approximate headless font metrics in JavaScript; that was the architectural source of the squeeze/overlap/wrap regressions. Build-fix 78 completes that translation: decoded `|FONT` attributes now actually reach the page (see *Fonts and hyperlinks*), paragraph spacing is additive rather than collapsed, authored blank paragraphs keep their line, and the topic surface is fluid instead of frozen at the export-time width. The exported topic body comes from `FormattedRecord`, `Paragraph`, `ParagraphFormat`, `Inline`, `FontTable`, table, picture, and hotspot structures directly. Browser layout performs normal font shaping and word wrapping; no JavaScript measures, scales, or repositions prose. Build-fix 71 still preserves a usable navigation-root `.CNT`/`.GID` hierarchy, and build-fix 72 still exposes the same exporter through `--export-html <source.hlp> [target.html]` before wxDragon is initialized. HTML remains an optional one-way output; the native viewer still renders HLP data directly.


## Goals

The exporter is designed to make a portable, single-file representation of the help system while retaining the behavior already implemented by the viewer rather than inventing a second HLP parser or formatting model.

The exported file therefore contains:

- every presentation topic from the user-opened navigation-root HLP;
- the currently active cross-document HLP, when different;
- the root HLP's already loaded `:Index` / `:Link` catalog documents;
- recursively discovered **relative** HLP references used by Contents entries, external hotspots, and allow-listed CONFIG/topic macros, up to a bounded document count;
- semantic topic markup derived from the decoded HLP formatting records, including paragraphs, fonts, tabs, borders, tables, pictures, hotspots, and safe controls;
- root-owned Contents, Show all, Index, Search, Bookmarks, and History navigation; hierarchical Contents uses the authored `.CNT`/`.GID` parent/child structure with collapsible branches when available;
- Back/Forward, physical Previous/Next, authored browse navigation, navigation-pane resizing/toggling, display zoom, and browser printing;
- clickable text, picture, and standard WinHelp button hotspots;
- the viewer's safe typed macro subset, translated to local JavaScript operations; the version 1.0 browser-only `ExecFile` exception becomes an explicit HTTP(S) link action rather than arbitrary script or host execution.

The HTML contains no external stylesheet, JavaScript library, image request, font download, or server dependency. Decoded RGBA pictures are base64-embedded in the file and painted into canvases locally.

## Command-line automation

The executable can export without constructing the native interface:

```text
hlp-viewer.exe --export-html <source.hlp> [target.html]
```

If `target.html` is omitted, the source extension is replaced with `.html` in the same directory (`manual.hlp` -> `manual.html`). An explicit target is normalized to an `.html` extension by the same writer used by the GUI export dialog. Existing targets are replaced intentionally so batch conversion is deterministic.

Both pathnames may contain spaces. Quoting them is always accurate:

```text
hlp-viewer.exe --export-html "D:\Rusty HLP viewer\CALC.HLP" "D:\out\calc.html"
```

An unquoted pathname reaches the process already split into several arguments, which previously produced a usage error instead of an export. The tokens that follow a path option are therefore rejoined with the single space that separated them, and the source/target boundary is found by asking the filesystem: the shortest leading run of tokens that names a real file is the source, and whatever follows it is the target. When no run matches - a mistyped path, or one that does not exist yet - the authored `.hlp`/`.mvb` extension terminates the source instead, so the user sees "could not open" naming the full reassembled path rather than a usage message. `--dump-file` and the positional GUI pathname are rejoined the same way; two arguments that both name real files are still reported as two documents rather than silently merged. `--dump-file=<file>` and `--export-html=<source>` are also accepted.

The command-line path opens the source as both the navigation root and active document, resolves its startup topic, loads the same bounded relative `.CNT`/`.GID` `:Index`/`:Link` catalog documents as the GUI, and then calls `html_export::export_to_html`. Relative cross-document destinations are subsequently collected by the exporter itself. No wxDragon application object or native frame is created.

A successful export prints the final HTML pathname to stdout. Non-fatal unresolved-link information is written to stderr while retaining exit status 0; HLP/open/export failures use exit status 1 and command-line syntax errors use exit status 2. This keeps stdout easy to capture in batch, PowerShell, or CI pipelines.

## Rendering model

`viewer/src/html_export.rs` consumes the same decoded `HelpDocument` presentation data as the desktop renderer, but **does not use `LayoutEngine` to position ordinary exported topic text**. It also does not decode `|TOPIC`, `|FONT`, graphics records, or macros independently. Parsing remains owned by the `hlp` crate.

The semantic mapping is intentionally straightforward:

| Decoded WinHelp object | HTML/CSS representation |
| --- | --- |
| `FormattedRecord` display paragraphs | normal block-flow HTML |
| `Paragraph` / `ParagraphFormat` | `<p>` with decoded margins, line spacing, alignment, direction, no-wrap policy, and border styling |
| `TextRun` / `FontDescriptor` | inline `<span>` with natural browser font shaping |
| text hotspot | real `<a>` element, green and underlined, carrying the local typed action |
| `Inline::LineBreak` | `<br>`; only authored breaks are forced |
| `Inline::Tab` | CSS-grid segment using decoded WinHelp tab stops/default interval |
| decoded picture | inline/floated canvas with embedded RGBA |
| picture hotspot | transparent positioned `<a>` over the picture |
| WinHelp table | independent flowing CSS columns, matching the verified non-row/grid table model |
| safe built-in WinHelp button | HTML button with the same allow-listed macro action |
| arbitrary hosted control | non-executing placeholder |

### Paragraph metrics

The exporter translates the verified paragraph fields rather than inferring geometry from a fallback font measurement. Signed paragraph metrics use the reference conversion already documented by the project:

- vertical spacing and line spacing: `raw * 96 / 144`;
- horizontal indents and tabs: `raw * 96 / 144`.

`spacing_above`, `spacing_below`, `left_indent`, `right_indent`, and `first_line_indent` become ordinary CSS paragraph spacing/indentation. Alignment and RTL map to `text-align` and `direction`. `no_wrap` maps to `white-space: nowrap`; otherwise the browser uses normal word wrapping (`white-space: normal`, normal word breaking, no synthetic hyphenation).

WinHelp **adds** a paragraph's space-below to the following paragraph's space-above, while adjacent CSS margins collapse to the larger of the two. Build-fix 78 therefore carries the owed space forward: each paragraph receives `spacing_above + previous spacing_below` as its top margin and a zero bottom margin, so the authored distance survives exactly. A region boundary and each table cell start a fresh vertical flow.

An authored paragraph with no visible content is a blank line in WinHelp, but an empty `<p>` has no line box at all and would collapse to nothing. Such paragraphs are marked `hlp-blank-paragraph` and given exactly one line of the topic's own font. Each region also inherits font descriptor 0 - the descriptor WinHlp32 selects once per topic render - so unstyled content has the authored height rather than the browser default.

The signed line-spacing rule follows the verified WinHlp32 behavior: zero keeps the natural measured line extent, a positive value is a minimum line advance, and a negative value is an exact line advance. Paragraphs with no authored line-spacing value use a slightly open default (`line-height: 1.35`) as a number rather than a length, so mixed font sizes inside one paragraph each keep proportional leading.

Only `Inline::LineBreak` creates a forced HTML line break. Automatic lines that the native retained layout might choose from host font metrics are never serialized, so the browser cannot double-wrap an already pre-wrapped source line.

### Fitting the visible page

Exported topics are **fluid**. The topic view and its regions carry no fixed pixel width; the export-time layout width is retained only as `--hlp-authored-width` and as the reference width for objects with absolute WinHelp geometry, chiefly table columns. The page surface fills the content host, so a narrower window, a dragged navigation splitter, a hidden navigation pane, or a text-zoom step re-wraps the prose inside the visible page instead of clipping it against a frozen width.

Text zoom remains CSS `zoom` on the topic host. Because a zoomed element scales its own pixel lengths, the shell sizes that host in pre-zoom pixels - `available / zoom factor`, which renders at exactly the visible page width - and refreshes it on zoom changes, window resizes, splitter drags, pane toggling, and container resizes. This measures the container only; it is not a return to the removed prose-reconciliation pass, and no glyph, word, or paragraph is measured, scaled, or repositioned. Printing ignores both the computed width and the zoom.

### Tabs and hanging lists

Tabs use the decoded `ParagraphFormat.tabs` and `default_tab_interval`. For every Tab command the exporter chooses the first custom stop **strictly to the right of the current position**, exactly as documented for WinHlp32; when none remains it advances to the next default-tab multiple. The missing default is 72 source units, which is 48 px at the export's 96-DPI reference scale. Left/right/center alignment is preserved in the corresponding grid segment.

This is especially important for numbered steps and bullet paragraphs: the marker occupies its own tab segment and the prose occupies the following segment. Browser-created continuation lines therefore wrap inside the prose column instead of back under the marker or being manually shifted after layout.

### Fonts and hyperlinks

Font face-family policy, point size, weight, italic, underline, strikeout, small caps, foreground/background inheritance, and charset-aware face choice come from the decoded `FontTable`. The browser shapes each run naturally. There is no `scaleX`, synthetic letter spacing, glyph-width fitting, or post-layout word movement.

Every run's declarations are written into a double-quoted HTML `style` attribute, so **CSS font families are single-quoted and the complete declaration list is attribute-escaped**. This is not cosmetic: a double-quoted family name terminates the attribute at its first quote, and the browser then discards every declaration after `font-family:` - which silently removed authored size, weight, italic, underline, strikeout, small caps, and colour from exported topics before build-fix 78. Weight, italic, and the underline/strikeout decoration are emitted explicitly per run, and `font-synthesis: weight style small-caps` is stated so a face without a real bold, italic, or small-capital cut is synthesized rather than drawn plain.

Small caps follow WinHlp32 (`0x411a59..0x411a6c`), which renders the HC30 attribute by reducing the authored cell height to two thirds and drawing the authored characters - correct for the usual authored all-capital key names such as `NUM LOCK`. A run that still contains lower-case characters instead keeps its authored size and receives real CSS small-capital shaping (`font-variant-caps: small-caps`), which is what the attribute means typographically and avoids rendering ordinary prose two thirds too small.

Exported type is scaled by one constant. `EXPORT_BASE_FONT_PX` (14 px) is the size a 10 pt authored font renders at; every authored size is multiplied by `EXPORT_BASE_FONT_PX / 13.333`, since the verified 96 DPI reference conversion puts 10 pt at 13.333 px and WinHlp32 itself draws at the host display DPI. Relative size relationships between authored fonts are preserved exactly, and paragraph indents, tab stops, spacing, and table geometry keep the unscaled reference conversion. This constant is the only place the exported type scale is tuned.

Text hotspots are real HTML anchors and are explicitly dark green (`#008000`) and underlined, with a darker hover/focus state. Their typed local action and tooltip metadata are preserved. Topic-title paragraphs whose visible text matches the decoded topic title are emitted bold; authored bold section labels remain bold through their font descriptor.

### Tables, borders, pictures, and controls

The verified WinHelp table structure is not treated as an HTML row grid. Each signed table column is an independent vertical cell flow. Type-0 relative width/gap metrics and nonzero absolute metrics use the same reference conversion as the core layout, and nested tables recurse through the semantic renderer. Paragraph borders remain paragraph formatting rather than synthetic cell borders.

Pictures participate as inline or left/right floated objects according to their decoded picture command. Their graphical hotspots remain transparent anchors over the image. Safe standard hosted buttons such as the empty `ALink` Related Topics button become normal HTML buttons; arbitrary hosted controls remain inert placeholders. Standard buttons sit below the surrounding text baseline by the `--control-drop` custom property (4 px by default) so the control lines up with the rule its authoring paragraph draws beside it rather than riding high against it.

A paragraph whose first visible object is a hosted control also keeps that control on the paragraph's left edge rather than in the hanging area. WinHelp's negative first-line indent is authored for a text marker - the marker hangs and the prose after it lines up with the left indent - so applying it to a control moves the whole control box out of alignment with the rules and text around it by the authored hanging distance. Authored bullet and list bitmaps are excluded from this rule and keep hanging, because those are real markers.

### No browser-side geometry reconciliation

The shell has no prose-layout correction pass. After cloning a topic template it initializes embedded canvases, sizes the topic surface to the visible page (see *Fitting the visible page*), and leaves text layout to normal HTML/CSS. This is a deliberate invariant: if browser font metrics differ from the native Windows viewer, normal text may wrap at a slightly different word, but it must remain readable, naturally shaped, non-overlapping, and governed by the decoded paragraph margins rather than by approximate headless token rectangles.

## Navigation-root policy

Build-fix 68 separated the active topic document from the navigation root. The exporter preserves the same rule.

For example, after `WORDPAD.HLP` opens a topic in `COMMON.HLP`:

- the exported start topic may be the active COMMON topic;
- Contents remains WORDPAD's hierarchy; when WORDPAD has usable `.CNT`/`.GID` data, that hierarchy is rendered as the same nested tree relation as the native Contents view, with nested branches collapsed until opened (or automatically expanded to reveal the selected authored root-document topic); cross-document navigation leaves the root tree/selection stable;
- Show all remains WORDPAD's physical presentation list;
- Index/Search remain WORDPAD plus only the catalog documents authored by WORDPAD;
- links inside COMMON still resolve relative to COMMON;
- Back/Forward history can move between embedded documents.

Popup-marked and secondary-window destinations are still resolved, but activation follows the desktop viewer's build-fix 16 single-main-surface policy. Hover text preserves the build-fix 20 distinction: ordinary links expose the destination title while popup-marked links expose the resolved popup topic's visible text.

## Palette

The exported shell intentionally mirrors the viewer's current surfaces:

- outer/content-host gray: `RGB(212,212,212)` / `#D4D4D4`;
- help-page cream: `RGB(255,255,228)` / `#FFFFE4`;
- WinHelp information yellow: `RGB(249,249,158)` / `#F9F99E`;
- normal text: black.

Authored explicit foreground/background colors on retained text remain authoritative.

## Related Topics / ALink

The classic small **Related Topics** button used by files such as CALC.HLP is the verified built-in `!label,macro` BUTTON form whose macro is `ALink` (or its `AL` alias). The core layout retains that hosted button as a safe standard-button placeholder with a viewer-local macro hotspot; HTML export turns that hotspot into a normal local action rather than leaving the control inert.

For an exported `ALink`, the exporter resolves the authored semicolon-delimited associative names against the originating HLP's `A` keyword table. One match navigates immediately. Multiple matches open the self-contained **Topics Found** chooser, and no matches report `No related topics found` in the status area. This path does not execute arbitrary WinHelp macro text; it uses the same typed `SafeHelpMacro::ALink` allow-list as the desktop viewer.

## Macro and file security

HTML export keeps the existing default-deny WinHelp macro model. Only macros already represented by `SafeHelpMacro` are converted into typed actions. The exporter never inserts raw HLP macro text as executable JavaScript. The narrow version 1.0 `OpenUrl` action accepts only validated `http://` or `https://` targets originating from the browser-only `ExecFile` form; activation uses the browser's own navigation API in a new tab.

Automatic cross-document collection is intentionally limited to relative HLP paths. Absolute drive paths, rooted paths, and UNC/network paths are not traversed during export. A target that is not embedded is retained as a safe unavailable action rather than causing the HTML file to access the user's filesystem or network.

Arbitrary WinHelp hosted controls remain inert placeholders. No native executable, DLL, ActiveX control, command shell, `file:` target, or arbitrary external command is launched by the exported viewer. A user-clicked allow-listed `OpenUrl` action may open its validated HTTP(S) target in a new browser tab.

## Self-contained state

Bookmarks are stored best-effort in browser `localStorage` under an export-specific key; History remains session-local. Storage failures are ignored so a `file://` export remains usable in restrictive browsers. The HTML does not modify the source HLP/CNT/GID files and does not write a companion database. Browser printing uses the current topic and removes navigation chrome with print CSS.

## Source boundaries

The implementation remains deliberately one-way:

`HLP/CNT/GID -> HelpDocument / decoded formatting semantics -> HTML`

The desktop application never renders through the generated HTML, and the core `hlp` crate has no HTML or browser dependency. This preserves the project's native-viewer architecture while adding a portable export facility.
