# HLP format notes used through milestone 1.0

The classic WinHelp on-disk format was not fully published. This parser is a fresh Rust implementation guided primarily by the reverse-engineered HelpDeco format description/structures and cross-checked where useful against Wine's current WinHelp implementation.

Primary reference material:

- https://github.com/joncampbell123/helpdeco/blob/master/doc/html/Windows%20Help%20File%20Format.htm
- https://github.com/joncampbell123/helpdeco/blob/master/helpdeco.h
- https://gitlab.winehq.org/wine/wine/-/tree/master/programs/winhelp

## Container and topic recap

The 16-byte HLP header points to an internal directory B+ tree. Internal streams are preceded by nine-byte `FILEHEADER`s. `|SYSTEM` identifies the compiler generation, topic block size, compression flags, metadata, and configuration macro strings.

Physical `|TOPIC` blocks begin with three signed 32-bit positions (`LastTopicLink`, `FirstTopicLink`, `LastTopicHeader`) followed by data. Compressed HC31+/HCW blocks are transformed into the logical data space before TOPICPOS values are interpreted. TOPICLINK records carry LinkData1 (structure/formatting) and LinkData2 (visible strings, phrase-expanded by 0.2).

HC30 next links are physical relative distances; HC31+ next links are absolute transformed TOPICPOS values. Cross-block reads skip physical block headers/gaps exactly once.

## `|FONT` and per-face charsets

Build-fix 17 replaced the earlier generation model after tracing the exact KB917607 renderer.
The `|FONT` prefix is eight bytes: face count, descriptor count, face-directory offset, and
descriptor-directory offset. WinHlp32 indexes **every** font descriptor with an 11-byte stride
(`index * 0x0B` at `0x411E8C..0x411EBC`). Compiler generation changes the fixed face-name slot
width, not the descriptor shape: pre-HCW-4 files use 20-byte face slots and minor version 33 uses
32-byte slots. There is no 42-byte MVB/modern descriptor path in the audited executable.

Each 11-byte descriptor contains the attribute flags, half-point size, family selector, face index,
foreground RGB, and background RGB. The retained `FontDescriptor` preserves those semantics,
including bold, italic, underline, strikeout, small caps, the otherwise-unused 0x10 attribute bit,
and the exact `RGB(1,1,0)` colour-inheritance sentinel established by the renderer.

Version 1.0 retains that verified rule in the raw parser, but the presentation layer also recognizes a narrow compiler-compatibility artifact observed in the supplied HelpScribble file: when an `RGB(0,0,0)` descriptor has an otherwise identical `RGB(1,1,0)` descriptor twin, both are treated as inheriting the help-window background. A standalone authored black background remains black.

Modern charset selection comes from `|SYSTEM`, not from a larger font descriptor. Record type 11 is
a byte array indexed by the descriptor's **face index**; record type 9 is a ten-byte locale record
whose final WORD gates WinHlp32's Arabic/Hebrew reorder path. Build-fix 34 decodes the common
Windows SBCS families (Western, Central/Eastern European, Cyrillic, Greek, Turkish, Hebrew, Arabic,
Vietnamese, Baltic and Thai) and the major DBCS families selected by Japanese, Korean, GBK and Big5
charsets before retained layout. Explicit non-default record-11 metadata wins. When it is absent or
DEFAULT_CHARSET, deterministic legacy face-name / `LANGID` inference covers the common historical
Windows families. Build-fix 39 closes Johab explicitly as Windows CP1361. `OEM_CHARSET` is not a fixed HLP encoding: the audited runtime delegates it to the host GDI charset/code-page database. Windows builds now mirror that behavior through the active system OEM code page; non-Windows builds cannot infer a Windows host OEM database from HLP bytes alone and retain the documented deterministic fallback.

A conservative MS Sans Serif fallback is used only when a file has no `|FONT` stream.

## Display/table LinkData1

Display (`0x01`/`0x20`) and table (`0x23`) TOPICLINK records begin with compressed numeric fields. Modern records add a compressed TopicLength. Table records then carry table type, optional minimum width, and per-column gap/width values.

ParagraphInfo is variable length: a flag word determines whether spacing-above/below/line-spacing, left/right/first-line indentation, borders, alignment, and tab stops follow. Table paragraphs additionally identify their column. Version 0.3 retains these raw historical metric values and converts them during layout according to the parsed `|FONT` generation.

## Character-formatting command stream

After each paragraph descriptor, LinkData1 is a command stream synchronized with NUL-terminated strings in expanded LinkData2. Build-fix 17 rechecked this scanner directly in KB917607 WinHlp32 (`0x41AAC1..0x41ACBA`) rather than extending older secondary-format descriptions.

Implemented commands include:

- `0x80` font selection;
- `0x81` line break;
- `0x82` next paragraph with the same ParagraphInfo;
- `0x83` tab;
- `0x85` signed 16-bit horizontal line-origin override; glyphless, but it resets the transient x origin used by line alignment;
- `0x86..0x88` picture/embed records (indexed/embedded raster or WMF graphics decoded when supported);
- `0x89` end hotspot;
- `0xC8` / `0xCC` macro hotspots;
- `0xE0..0xE3`, `0xE6..0xE7` internal popup/topic jumps;
- `0xEA`, `0xEB`, `0xEE`, `0xEF` external/secondary-window jumps;
- `0xFF` paragraph/record transition.

The Microsoft scanner also structurally accepts the complete `C0..CF` / `E0..EF` hotspot envelope. Fixed variants selected by `(opcode & 0xD8) == 0xC0` are five bytes total (opcode + four-byte payload); variable variants selected by `(opcode & 0xD8) == 0xC8` are opcode + WORD payload length + that exact payload. Build-fix 39 traces the activation dispatcher at `0x429C13..0x429E24`: only `0xC8/0xCC`, `0xE0..0xE3`, `0xE6/0xE7`, `0xEA/0xEB`, and `0xEE/0xEF` have click-action branches (plus the unrelated `0xB0` case). Other structurally accepted envelope values fall through without dispatch, so they are now classified as verified inert for this KB917607 runtime rather than semantically unknown.

A negative result matters here: `0x20` and `0x21` are **not** inline `VariableField` / `DType` commands in this executable. They are modern TOPICLINK record types. Likewise `0x8B` and `0x8C` are not accepted character commands by this scanner. Build-fix 17 removes all four fabricated character-stream branches so they can no longer consume bytes and desynchronize a display record.

Truly unknown commands outside the structurally accepted families still stop only the affected semantic display record and produce a diagnostic warning. The document stays open and the higher layer can retain a complete plain-text fallback.

### Picture command synchronization

Inline picture commands contain a type byte and a compressed **signed** long payload size. This matters because the short-form signed-long bias is different from the unsigned-long encoding used by bitmap headers. For type `0x22`, the following hotspot count uses WinHelp's compressed **unsigned short**, not a fixed 16-bit integer.

The compact graphics renderer used by TOPICLINK types `0x03` / `0x22` enters the same graphics loader after its compact header. Direct executable tracing establishes the payload selector rule: selector word zero is followed by a signed WORD internal bitmap number (`|bmN`, negative numbers rejected); **any nonzero selector** means the logical graphics object is embedded immediately after the selector. Build-fix 17 applies that same source rule to inline and compact graphics.

The character command byte distinguishes `bmc`/character (`0x86`), `bml`/left (`0x87`), and `bmr`/right (`0x88`) variants. Version 0.5.1 retains `bmc` inline placement and treats `bml`/`bmr` as paragraph-local left/right floats: overlapping lines use the remaining horizontal span and later lines return to full width after the image bottom.

### Compact graphics and hosted controls

The generic compact dispatcher accepts old `0x01..0x06` and modern `0x20..0x24` records. The verified visual branches are now represented explicitly:

- `0x01` / `0x20`: ordinary display text;
- `0x03` / `0x22`: graphics, decoded with the normal WinHelp graphics pipeline;
- `0x04` / `0x23`: tables, including recursive nested tables;
- `0x05` / `0x24`: hosted/custom-window objects;
- `0x06`: no renderer call in the traced dispatcher.

For `0x05` / `0x24`, WinHlp32 skips a six-byte prefix, reads a NUL-terminated descriptor, creates a native child window/control, and queries its dimensions. Executing such authored controls would violate this viewer's cross-platform/default-deny design, so build-fix 17 retains the prefix and descriptor and paints a bounded placeholder instead. This is intentionally a safe presentation substitute, not a claim that the reference control's runtime dimensions can be reproduced without creating it.

## Graphics streams

An indexed `|bmN` stream or embedded picture object is a logical graphics stream containing an alternative-count header and an offset table. Version 0.5.1 prefers a DIB alternative (`type 0x06`), falls back to a portable DDB alternative (`type 0x05`), then accepts a legacy Windows metafile alternative (`type 0x08`). WMF payloads are decompressed in `hlp` and rasterized on Windows through the narrow `wmf-render` GDI adapter before the normal retained RGBA path.

Build-fix 33 also retains each picture alternative's **natural display geometry**. For bitmap types
`0x05`/`0x06`, nonzero authored x/y resolution fields scale raw pixel extents independently by the
target layout `dpi_x` / `dpi_y`; zero fields keep raw-pixel natural size. WMF physical mapping modes
use the same target per-axis DPI when converting logical extents. The bounded WMF compatibility
raster may still be produced at 96 DPI internally—the retained picture box then scales that RGBA
surface to its device-correct natural dimensions, so decoding safety and document geometry remain
separate concerns.

Bitmap alternatives carry compressed numeric metadata for resolution, planes, bit depth, dimensions, palette counts and packed size, followed by raw 32-bit offsets to pixel/hotspot data. DIB palette entries are stored as BGRA quads. The implemented raster path supports:

- DIB depths 1, 4, 8, 16, 24 and 32 bpp;
- 1/16/24/32-bit DDBs where no external palette interpretation is required;
- bottom-up Windows scan lines with 32-bit row alignment;
- raw packing (`0`), WinHelp RLE (`1`), LZ77 (`2`), and LZ77 followed by RLE (`3`);
- the WinHelp transparent-palette convention when the important-colour field marks the last palette entry as transparent.

Decoded graphics are normalized to top-down RGBA before wxDragon sees them; wxDragon never parses HLP graphics bytes. Repeated indexed resources share their RGBA allocation. Dimensions, palette counts, packed ranges and decoded buffers are bounded before allocation/decompression.

Bitmap and metafile records can also carry a graphical-hotspot table. Version 0.5.1 bounds the table by its declared size, parses its fixed 15-byte geometry/action records, skips the macro-data prefix, then reads the paired hotspot-name/link-name strings. Macro (`0xC8`), same-file context (`0xE6`/`0xE7`), and named-window context (`0xEE`/`0xEF`) actions become semantic `PictureHotspot`s. The retained layout scales source-image coordinates when a picture is resized and inserts invisible hotspot boxes in front of the picture so normal front-to-back hit testing handles image links.

### External hotspot structure length

External hotspot commands start with a signed 16-bit `SizeOfFollowingStruct`. That value describes the following structure itself: type byte, 32-bit TOPICOFFSET, and optional window/file strings according to type. It is not interpreted as a total including the command byte or size field. This distinction is required to keep subsequent LinkData1 commands synchronized.

## Retained layout

`hlp` converts semantic paragraphs into integer document-coordinate boxes without wxWidgets. It currently handles:

- half-point/twip conversion at configurable DPI;
- font-resolved approximate text metrics;
- wrapping, first-line/left/right indentation;
- paragraph spacing/alignment;
- custom/default tabs;
- table columns/cells;
- paragraph borders;
- decoded raster/WMF pictures plus fallback picture placeholders;
- inline and left/right floating picture placement with text wrap;
- text and graphical hotspot rectangles.

Fixed and scrolling topic records are laid out separately. The wxDragon crate paints the retained result but does not reinterpret HLP bytes.

The engine retains deterministic approximate metrics for headless tests, while the native viewer runs the same retained-layout algorithm with `wxWidgets` text extents from the exact font object used for painting. Explicit DPI-change reflow and font-object caching remain future polish.

## Security interpretation

CONFIG, topic, and hotspot macro strings are parsed into a typed default-deny command model. Only viewer-local allow-listed operations are dispatched; shell/process execution, DLL registration, host interaction, unknown names, malformed syntax, and unsupported legacy UI mutations remain inert and diagnostic-only. Macro programs, arguments, nesting, strings, runtime command count, and retained diagnostics all have explicit ceilings. Picture payloads and external hotspot structures remain length checked before access, and pathological table/tab/paragraph counts retain their existing ceilings.

## 0.4 navigation metadata

Version 0.4.x keeps WinHelp's two location types distinct. `TOPICPOS` identifies transformed records in `|TOPIC`. `TOPICOFFSET` is the cursor-like value stored by contexts and hotspots: its block portion identifies a topic block and its low character-count portion advances by the `TopicLength` values of display/table links. `hlp` reconstructs exact per-record anchors and resolves a target to the containing topic only within the same TOPICOFFSET block.

### HC30 `|TOMAP`

Windows 3.0 uses numeric topic identifiers. `|TOMAP` is a flat little-endian array of `TOPICPOS` values and is indexed directly by TopicNumber; the historic normal-topic base of 16 is **not** subtracted. Element zero identifies the project INDEX/contents topic. HC30 previous/next browse numbers are therefore resolved through this map before matching a reconstructed type-2 topic header.

### HC31+/HCW context streams

`|CONTEXT` is a B+ tree of signed 32-bit context hashes to `TOPICOFFSET`. The project implements the documented 256-entry signed-character hash table and wrapping `hash = hash * 43 + table[byte]` arithmetic. `|CTXOMAP` is a counted flat array mapping signed 32-bit map IDs to `TOPICOFFSET`.

HCW can additionally contain `|TopicId` (TOPICOFFSET -> symbolic ContextName) and `|Viola` (TOPICOFFSET -> DefaultWindowNumber) B+ trees. `|Viola` assignments are treated as exact topic-offset assignments; they are not inherited from the nearest preceding entry.

### SYSTEM WINDOW records

Ordinary HC31/HCW SYSTEM record type 6 uses the 90-byte window structure: validity flags, 10-byte type, 9-byte name, 51-byte caption, normalized x/y/width/height values, maximize/style bits, and scroll/non-scroll `COLORREF`s. Multimedia record type 6 has a different structure and remains preserved as an unknown SYSTEM record rather than being misparsed as the ordinary structure.

### Hotspot navigation

Internal text hotspot commands use two destination encodings. `0xE0`/`0xE1` carry physical `TOPICOFFSET` values, while `0xE2`/`0xE3`/`0xE6`/`0xE7` carry context hashes that are resolved through `|CONTEXT` (with a compatibility TOPICOFFSET fallback for unusual producers). External text commands and graphical context commands use the WinHelp low-bit convention: an even opcode is a popup (`0xEA`, `0xEE`) and an odd opcode is a normal navigation link (`0xEB`, `0xEF`). External file names are resolved relative to the HLP carrying the link unless already absolute. Type-specific window numbers/names select a parsed SYSTEM window definition when available.

Macro hotspots now enter the same safe parser/dispatcher as CONFIG and topic macros. Navigation compatibility never evaluates arbitrary host code: only the typed viewer-local allow-list can execute.

## 0.6 authored contents, keyword index, and search

WinHelp Contents metadata can live outside the HLP in a `.CNT` sidecar. Version 0.6 discovers the filename named by SYSTEM record type 10 first and then tries a same-basename `.cnt` case-insensitively. The sidecar is decoded as Windows-1252 text. Numbered lines form the authored hierarchy; `title=context@helpfile>window` targets retain symbolic/numeric context, optional external HLP, and optional named-window components. `:Title`, `:Base`, `:Index`, and `:Link` are parsed. Unknown directives are warnings rather than fatal HLP errors.

Build-fix 46 adds a read-only WinHelp 4.x `.GID` fallback when that CNT is absent or unreadable. GID uses the same outer container/B+tree family as HLP. In the supplied Windows 95 WordPad cache, `|CntText` is an `Lz` B+tree of `u32 key + STRINGZ` records: 35 ordinary ordered rows plus special keys `70000` (Contents title) and `70001` (base HLP/window). `|CntJump` uses the same leaf form but contains only the 29 clickable rows. `|Flags` ends with tag `0x0C` followed by 35 node bytes; each high nibble exactly matches the corresponding CNT level (`0x10` for the observed level-1 books, `0x22` for the observed level-2 clickable topics). The low nibble is intentionally not treated as fully decoded: clickability is derived from key presence in `|CntJump`. A second supplied GID lacks both `|CntText` and `|CntJump`, so the mere existence of a GID is not enough to claim Contents support. The reader requires the verified `0x0C + one byte per row` tail and otherwise leaves hierarchical Contents unavailable rather than guessing.

GID `|FILES` is also read as the observed `L4z` leaf form (`u32 key`, cached metadata dword, `STRINGZ`). Labeled rows become Index catalogs, unlabeled rows become Link/Search catalogs, and key `10000` (the cached CNT pathname) is skipped. Because WinHelp stores resolved Win9x absolute paths in this cache, build-fix 46 reduces absolute cached help paths to their final filename before applying the viewer's existing sibling-relative and automatic-catalog security rules. The viewer does not generate, refresh, or invalidate GIDs. A present CNT remains the preferred authored source; GID is a compatibility fallback for the experimentally demonstrated `HLP + GID, no CNT` case.

Authored keyword tables use paired internal streams such as `|KWBTREE` and `|KWDATA` (and the same pattern for alternate table letters). Their B+ tree uses the same fixed 38-byte header/page linkage as the internal directory but different leaf records: a NUL-terminated keyword, target count, and data-stream offset. The associated data is a sequence of signed TOPICOFFSET values. Every non-negative target is resolved through the document's reconstructed TOPICOFFSET anchors; `-1` macro targets are retained as metadata but never executed. The standard `K` table drives the viewer's Index page.

The modern Search page is deliberately a derived convenience rather than another HLP format primitive. When a document opens, the engine pre-folds decoded topic titles, plain text, and standard authored keywords. Ranking is deterministic: exact/prefix/substring title and keyword matches outrank ordinary body-text phrase/all-term matches. The viewer merges those results across the active HLP plus at most 32 unique one-hop relative Contents-linked help files from CNT `:Index`/`:Link` or GID `|FILES`; absolute/UNC catalog references remain warning-only during automatic loading. Destinations remain file-qualified so the ordinary cross-file history code performs the actual navigation. The Contents tree re-selects the authored/cached row for the current same-file topic when its context resolves. Recursive `.CNT` `:Include` graphs are not followed and are surfaced as warnings.
