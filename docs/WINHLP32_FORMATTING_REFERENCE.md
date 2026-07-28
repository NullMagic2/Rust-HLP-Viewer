# Microsoft WinHlp32 formatting reference

This note records formatting behaviour reverse-engineered directly from the Microsoft
WinHlp32 binary reconstructed from the user-supplied Windows 8.1 KB917607 x64 package.
It is intended to keep the Rust implementation tied to observed behaviour rather than
to assumptions inherited from third-party viewers.

## Reference binary

- File: `winhlp32.exe` (kept external to this source tree)
- PE machine: x86 (`IMAGE_FILE_MACHINE_I386`)
- Length: 285,696 bytes
- SHA-256: `8496f19bc1d898180b57aac61326bdfcf5a48f760811283bbd604aa7a2c13f85`
- Package: Windows 8.1 KB917607 x64

Addresses below are virtual addresses in that exact executable. They are useful only
for this verified binary revision.

## Classic paragraph record

The paragraph decoder begins at approximately `0x4125db`. Before the paragraph id/flag
DWORD, WinHlp32 consumes one **compressed signed long** with the same two/four-byte decoder
used elsewhere. Its semantics are not yet needed by layout, but its variable length matters:
treating it as two fixed bytes desynchronizes every later field when the four-byte spelling
appears.

The signed-long helper itself is at `0x4129e8`. Its exact decoding is:

- short form (low bit clear): `(u16 >> 1) - 0x4000`;
- extended form (low bit set): `(u32 >> 1) - 0x40000000`.

Build-fix 9's extended-form Rust decoder used the wrong bias (`0x04000000`), so extended
signed values could decode incorrectly even when stream synchronization survived. Build-fix
10 corrects that helper globally, including paragraph/picture fields that reuse it.

The 16-bit paragraph flag word then controls optional fields in this order:

| Flag bits | Microsoft behaviour | Rust representation |
| --- | --- | --- |
| 0 | compressed signed long; semantics unresolved | `unknown_value` |
| 1 | spacing before paragraph | `spacing_above` |
| 2 | spacing after paragraph | `spacing_below` |
| 3 | line spacing | `spacing_lines` |
| 4 | left indent | `left_indent` |
| 5 | right indent | `right_indent` |
| 6 | first-line indent | `first_line_indent` |
| 7 | default tab interval; absent defaults to 72 source units | `default_tab_interval` |
| 8 | three-byte border record | `BorderInfo` |
| 9 | custom tab-stop array | `tabs` |
| 10..11 | two-bit paragraph alignment | `alignment` |
| 12 | suppress automatic word/tab wrapping | `no_wrap` |
| 13 | right-to-left paragraph/layout flag | `right_to_left` |

Bits 14 and 15 were not observed as formatting controls in the traced path.

### Metric conversion

Paragraph measurements are not interpreted through the `|FONT` generation. The
Microsoft parser converts them directly with integer arithmetic:

- vertical values (spacing before/after and line spacing): `raw * LOGPIXELSY / 144`;
- horizontal values (indents and tabs): `raw * LOGPIXELSX / 144`.

These values remain signed. In the normal layout path, spacing-before is added with a
`movsx`/`add` sequence near `0x415bae`, right indent is signed near `0x415bd1`, and
spacing-after is added signed near `0x415f18`. Negative spacing and negative side indents
therefore must not be normalized to zero.

The x86 `idiv` path truncates toward zero. The previous Rust assumption that old
paragraph values should be selected between half-point and twip conversion was therefore
incorrect.

The Microsoft viewer also applies its own global text-size preference to some paragraph
values before this conversion. The Rust viewer deliberately keeps its independent 70-200%
viewer zoom instead of reproducing that UI preference byte-for-byte.

### Alignment

At `0x416170..0x416187`, the decoded two-bit value behaves as follows:

- `0`: left;
- `1`: right;
- `2`: center;
- `3`: right.

Only value 2 takes the half-remaining-width centering path. Build-fix 9 incorrectly
interpreted value 3 as centered.

### No-wrap

Bit 12 is tested in the normal overflow paths around `0x41661f`/`0x4166e7` and in the
tab-overrun path around `0x417793`. When set, the viewer does not create an automatic
new line merely because a word or tab target exceeds the available right edge.

### Line spacing

The signed line-spacing value is applied around `0x4160c5..0x4160e4`:

- zero: natural measured line extent;
- positive: minimum line advance, `max(natural, authored)`;
- negative: exact line advance, `abs(authored)`.

This is implemented after the reference DPI/144 conversion.

## Tabs

The custom-tab lookup routine is around `0x4124c2`.

Each custom tab has a position and, when encoded, an alignment:

- `0`: left;
- `1`: right;
- `2`: center.

The first custom stop strictly to the right of the current x-position wins. When no
custom stop remains, WinHlp32 advances to the next multiple of the paragraph's default
tab interval. If bit 7 did not provide that interval, the parser installs 72 source
units before DPI conversion.

Left tabs can move the current x-position immediately. Right and center tabs cannot:
WinHlp32 remembers the target, lays out the following segment, then resolves it around
`0x412525`:

- right: place the segment's right edge on the stop;
- center: place the segment's center on the stop;
- if the required shift would be negative, leave the segment in place.

This deferred behaviour is necessary for columns of numbers and centered labels to
match the Microsoft viewer.

## 11-byte `|FONT` descriptor across compiler generations

The font builder is around `0x411884`. A second selection path at `0x411E8C..0x411EBC`
proves that **all** audited compiler generations index descriptor records in 11-byte strides.
The observed layout is:

- byte 0: attribute bits;
- byte 1: size in half-points;
- byte 2: family;
- word 3..4: face-name table index;
- bytes 5..7: foreground RGB;
- bytes 8..10: background RGB.

### Font height

WinHlp32 feeds the half-point size directly into a `MulDiv(..., LOGPIXELSY, 144)`-style
font-height conversion and negates the resulting `lfHeight`. It therefore preserves
half-point sizes instead of first rounding them to whole typographic points.

### Attribute bits used by the Microsoft builder

- `0x01`: bold (700 instead of 400 weight);
- `0x02`: italic;
- `0x04`: underline;
- `0x08`: strikeout;
- `0x20`: small caps / reduced-height font.

For `0x20`, the font height is changed to two thirds of the normal height around
`0x411a59..0x411a6c`.

The classic builder does **not** consume attribute bit `0x10`. Although the HLP parser
retains it as `double_underline` for lossless decoding, no second underline is synthesized
for this classic Microsoft path.

### Family mapping

The classic family values map to Win32 `LOGFONT` family classes as follows:

- 1: modern / fixed-pitch family (`0x30`);
- 2: Roman (`0x10`);
- 3: Swiss (`0x20`);
- 4: script (`0x40`);
- 5: decorative (`0x50`).

The Rust viewer still substitutes modern Windows faces for ordinary historical faces,
while preserving the semantic family and keeping symbol/decorative faces when required.

### Face-name slots and `|SYSTEM` charset metadata

At `0x411E8C`, WinHlp32 compares the `|SYSTEM` minor version against decimal 33. The descriptor
stride remains 11 bytes in both branches; only the fixed face-name slot changes: 20 bytes before
minor 33 (including MVB/minor 27) and 32 bytes for HCW 4.0/minor 33. The previous Rust model's
42-byte modern/MVB descriptor and style/character-map directories were therefore a speculative
misread and are removed in build-fix 17.

`|SYSTEM` record type 11 supplies charset data. Its dispatcher allocates exactly `data_size + 1`
bytes at `0x42CE6C`, and font selection later reads one byte from `[table + face_index]` at
`0x411EA7..0x411ED4`. This is a per-face GDI charset table, not one global 16-bit charset. When
the table is absent, helper `0x411E6F` asks GDI to infer a charset from the selected face.

Record type 9 is separately accepted only at a ten-byte size (`0x42CE5B`). Its final WORD is the
locale/LANGID source later masked with `0x03FF` before the Arabic/Hebrew path is entered.

The retained 1995 Portuguese Calculator HLP provides a real-file cross-check of this model: its
`|SYSTEM` minor version is 33, its `|FONT` header reports two faces with offsets 8 and 72 (exactly
two 32-byte face slots), and its nine descriptors occupy 11 bytes each. Its record 9 locale is
`0x0416`, while record 11 contains three zero charset bytes. This independently exercises the HCW
4.0 branch without relying on a synthetic fixture.

### Foreground and background colours

The colour resolver around `0x411be6` constructs the foreground and background
`COLORREF`s from the descriptor RGB bytes. For **each** colour, exact `0x00000101`
(`RGB(1,1,0)`) is a sentinel meaning that the currently active/default colour is retained.
It is not a fuzzy near-black test.

WinHlp32 uses opaque GDI text background output in this path. Therefore an explicit
font background colour is semantically meaningful and must not be discarded.

## Font selection lifetime

The selected font is **not** paragraph-scoped. WinHlp32 holds it in a single global at
`0x43C2C4`, and the character-stream scanner treats it as running state.

`0x41B024` is the topic render entry. At `0x41B05D` it initialises the selection once:

```
movzx eax, word ptr [0x43B09C]   ; per-file default / 0xFFFF sentinel
mov   dword ptr [0x43C2C4], eax
mov   dword ptr [0x43C2D0], esi  ; esi = 0
mov   dword ptr [0x43C2D8], esi
```

Character opcode `0x80` is the only writer, at `0x41AB8C`:

```
lea   eax, [edx + 1]             ; skip the opcode byte
mov   dword ptr [esi], eax
movsx edx, word ptr [eax]        ; font index, sign-extended
call  0x411E6F                   ; map index -> internal font id (returns AL)
movzx ebx, al
cmp   ebx, dword ptr [0x43C2C4]  ; unchanged? then no realise call
je    short skip
call  0x41A9B5                   ; realise via the callback at [0x43E098]
mov   dword ptr [0x43C2C4], ebx
skip:
add   dword ptr [esi], 2
```

Critically, neither paragraph terminator clears `0x43C2C4`. The `0xFF` handler at `0x41ABEB`
advances the stream and optionally reads the next ParagraphInfo; the `0x81`/`0x82`/`0x83` path
at `0x41AB7A` only raises the `0x43C2D8` progress flag. `0x43B09C` holds `0xFFFF` in the
on-disk image, a value the mapper at `0x411E6F` can never return, so the first `0x80` in a
topic always forces a real font realise.

The consequence for a decoder: a paragraph that reuses the previous paragraph's font emits no
font command at all. Re-initialising the selection at each paragraph (or at each display
record) silently substitutes descriptor 0, which in most authored files is the bold heading
face - producing bold body paragraphs immediately after every heading.

This viewer therefore threads the selection through `parse_character_stream` and across the
records of one region (`FormattedRecord::decode_with_font`). The reset boundary is chosen per
region rather than per topic: WinHlp32 initialises once per topic render and then paints the
non-scrolling region before the scrolling one, but those are independently painted surfaces
here, and every observed compiler emits a font command at the start of a body region.

## Paragraph borders

Bit 8 copies a raw three-byte border record. The first byte contains both sides and a
three-bit style code.

### Side bits

The low five bits are tested as:

- bit 0: whole box / all sides;
- bit 1: top;
- bit 2: left;
- bit 3: bottom;
- bit 4: right.

The helper near `0x415320` effectively tests `box OR requested_side`.

### Style code

The high three bits are extracted together as `(flags >> 5) & 7` around
`0x415386..0x415392`. They are **not three independent Boolean properties**.

Observed rendering behaviour:

- style 0: normal single border;
- style 1: thick border treatment;
- style 2: double border (second rectangle/edge two pixels inward, `0x415501..0x41552c`);
- style 3: shadow border (offset bottom/right strokes, `0x41553d..0x41556b`);
- style 4: same basic geometry/clearance class as normal in the verified path;
- styles 5..7: no positive clearance returned by the traced spacing helper; retained as reserved.

### Border-to-content clearance

The helper near `0x415320` reserves the following device-pixel clearance when the
corresponding side exists:

- styles 0 and 4: 5 px;
- styles 1 and 3: 6 px;
- style 2: 7 px;
- styles 5..7: 0 px in that helper.

The normal layout path applies these independently to top, left, bottom, and right.
Thus a border changes paragraph text geometry; it is not merely a paint overlay.

The remaining two bytes of the three-byte border record are copied forward by WinHlp32 but are
not read by the traced clearance helper or border painter. Build-fix 17 therefore treats them as
**render-inert in the verified paragraph-border path** while retaining them raw for losslessness.
They are not a pen width. Styles 5..7 likewise have no defined style setup in the painter switch
and zero clearance; the deterministic viewer leaves those reserved values unpainted instead of
inventing a normal border from ambient GDI state.


## Table records (`TOPICLINK` type `0x04` / `0x23`)

The table layout path begins around `0x414f66`. The reference implementation establishes
several details that were previously only approximated in the Rust viewer.

### Header and column records

The table header begins with:

- byte 0: column count; WinHlp32 rejects values above **32**;
- byte 1: table type;
- for **type 0 only**, an additional unsigned 16-bit minimum-width value;
- then one four-byte record per column.

Each column record is two **unsigned** words in this order:

1. authored column **width**;
2. authored **gap before the column**.

The previous Rust decoder had these two words reversed and treated them as signed.

### Horizontal geometry

For table type 0, WinHlp32 first computes:

`minimum_width_px = minimum_width_raw * LOGPIXELSX / 144`

`effective_width = max(available_width, minimum_width_px)`

and a 32767-unit reference span:

`reference_px = 32767 * LOGPIXELSX / 144`

Each authored gap/width is deliberately converted in two integer stages:

`physical = raw * LOGPIXELSX / 144`

`scaled = physical * effective_width / reference_px`

The two truncations are observable and should not be algebraically collapsed.

For every **nonzero** table type, gap and width are absolute unsigned source metrics:

`pixels = raw * LOGPIXELSX / 144`

There is no proportional normalization to the viewport in that path.

For each column, the gap is added to the running x-position first; that resulting x is
the column origin; the converted width is then added to reach the next column.

### Vertical flow

WinHlp32 does not form HTML-like rows. It maintains one cumulative vertical cursor per
column:

- a cell begins at `table_y + column_height[column]`;
- rendering the cell advances **only that column's** height;
- overall table height is the maximum cumulative column height.

Therefore a tall item in column 1 does not force the next item in column 0 down to a
shared row baseline. Build-fix 10 replaces the previous row-grouping approximation with
this independent-column flow.

### Nested cell framing

The reference table walker at `0x414F66` shows that each cell begins with a **signed
16-bit column index**. Column `-1` terminates the complete cell list. Any nonnegative
column is followed immediately by a complete compact nested TOPICLINK record; there is
no fixed five-byte table-cell prelude before ParagraphInfo.

The compact header helper at `0x412884` accepts record generations `0x01..0x06` and
`0x20..0x24`. For ordinary display/table/graphics families it consumes:

1. one-byte record type;
2. compressed signed payload size;
3. for modern record types (`> 0x10`), a compressed unsigned TopicLength;
4. exactly `payload_size` bytes of nested payload.

Types `0x02` and `0x21` use the helper's fixed-width topic-header form instead: a DWORD
payload size follows the type byte, and `0x21` additionally carries a WORD TopicLength.

Dispatcher `0x417578` resolves the compact visual families more specifically:

- `0x01` / `0x20` -> ordinary display renderer `0x415929`;
- `0x03` / `0x22` -> graphics renderer `0x41200D`;
- `0x04` / `0x23` -> table renderer `0x414F66`;
- `0x05` / `0x24` -> hosted/custom-window renderer `0x419281`;
- old `0x06` -> fallthrough with no renderer call in this dispatcher.

After a nested renderer returns, the caller decodes the same compact header again and advances by
**header length + decoded payload size** before reading the next signed column. This exact boundary
rule prevents one cell from consuming the following cell's metadata.

Build-fix 12 implements the framing, build-fix 13 implements recursive `0x04`/`0x23` tables, and
build-fix 17 closes the remaining concrete visual branches. `0x03`/`0x22` reuse the existing
indexed/embedded graphics decoder both at top level and inside tables. The graphics loader at
`0x4062DF` proves selector zero means a signed WORD `|bmN` index (negative rejected) and every
nonzero selector means an embedded logical graphics stream. `0x05`/`0x24` skip a six-byte prefix,
read a NUL-terminated descriptor, create a native child control, and query its dimensions. The Rust
viewer retains that metadata but deliberately does not execute authored native controls; it reserves
a bounded placeholder instead. Old `0x06` is retained explicitly as a no-render record.

### Recursive table dispatch and returned geometry

The recursive behavior is explicit in the executable rather than inferred from the file format.
For every cell, `0x414F66` prepares a child layout state containing the current column origin,
current column y position, and converted column width, then calls the generic record dispatcher
`0x417578` at `0x4151B6`. In that dispatcher, compact record types `0x04` and `0x23` take
the table branch at `0x4175E3..0x4175F1`, which calls `0x414F66` again. There is no separate
"nested table" algorithm.

When the child renderer returns, the parent reads the child state's height field (state offset
`+0x14`) and adds it only to the cumulative height for the cell's signed column at
`0x4151DC..0x4151E9`. The parent's overall table height remains the maximum of those independent
column cursors. Therefore recursion has these concrete layout rules:

- the nested table's x/y origin is the containing cell's current origin;
- its available width is exactly the containing parent column width;
- it computes its own type-0 proportional or nonzero absolute column geometry from that width;
- its own columns advance independently;
- the nested table returns its maximum child-column height;
- only the containing parent column advances by that returned height.

The dispatcher shares the same surrounding render state while recursing. Build-fix 13 mirrors that
with one shared LinkData2 string stream and one running font selection through the recursive parser,
so a sibling cell after a nested table resumes after every string/font command consumed by the nested
subtree. Paragraph objects are stored once in the owning `FormattedRecord`; recursive display cells
retain paragraph ranges into that store, avoiding duplicate picture/hotspot state.

Microsoft's executable has no table-format recursion-depth field. The Rust engine adds a defensive
64-level cap solely to prevent malicious files from exhausting the process stack; reaching it produces
the normal bounded formatting diagnostic/fallback path rather than reading outside the compact payload.

## Character-command scanner corrections

The reference scanner at `0x41AAC1..0x41ACBA` gives a stricter command set than the older Rust
model assumed. `0x20`, `0x21`, `0x8B`, and `0x8C` all take the unsupported exit; none consumes an
inline payload. This matters because pretending that `0x20`/`0x21` were `VariableField`/`DType`
objects could advance LinkData1 by two or four extra bytes and corrupt every subsequent paragraph
boundary. Build-fix 17 removes those branches.

The real omitted command was `0x85`: the scanner advances exactly three bytes, while tokenizer
`0x417816..0x417827` sign-extends the following WORD into transient state `+0x38`, emits token kind
`0x36`, returns tokenizer status 2, and creates no glyph. The downstream role is now characterized.
Paragraph setup seeds that same `+0x38` state from the DPI-converted horizontal paragraph origin at
`0x415CE4..0x415D35`; line finalization copies it at `0x415FB5` and uses it in the available-span /
alignment calculation at `0x416169..0x4161B1`. Therefore `0x85` is a glyphless signed horizontal
line-origin override, not a variable field. The Rust retained layout now applies the marker as a
device-coordinate x-origin reset. Separately, the text-extraction scanner sets pending flag
`0x43C2D8`; `0x41AA18..0x41AA58` inserts one ASCII space before the next extracted string and clears
the flag.

The same scanner accepts the entire masked hotspot structure families. `(opcode & 0xD8) == 0xC0`
advances exactly five bytes total; `(opcode & 0xD8) == 0xC8` advances by opcode + WORD length +
that many payload bytes. The known `0xC8`/`0xCC` macro reader now follows the same rule: that WORD
is the following payload size, not a total size from which three header bytes should be subtracted.
Known hotspot opcodes retain their semantic navigation models. A second trace of the activation
dispatcher at `0x429C13..0x429E24` closes the remaining navigation question: only `0xC8`/`0xCC`,
`0xE0..0xE3`, `0xE6`/`0xE7`, `0xEA`/`0xEB`, and `0xEE`/`0xEF` have click-action branches in this
KB917607 runtime (plus unrelated command `0xB0`). The remaining structurally accepted envelope
variants fall through without dispatching navigation or a macro. The Rust parser therefore keeps
their exact boundaries but labels them **KB917607-inert**, rather than “unknown action”.

## Paragraph flag bit 0

At `0x4125D9` paragraph flag bit 0 conditionally decodes one compressed signed long. The value is
retained as `unknown_value`, but the verified paragraph geometry, line-layout, border, and paint
paths do not read that field. Build-fix 17 therefore classifies it as retained non-visual metadata
for this renderer rather than inventing a spacing or width meaning.

## Right-to-left paragraphs and charset-run ordering

Paragraph bit 13 selects the reference right-to-left base-direction path. On the first line, the
signed first-line indent is subtracted from the **right** edge at `0x415D1C..0x415D35` instead of
being added to the left edge. Build-fix 10 already retained that geometry.

Build-fix 17 traces and implements the later language/font gates as well. The path beginning around
`0x41623B` first masks the resolved language value with `0x03FF` and continues only for primary
language 1 or 0x0D (Arabic or Hebrew). Helper `0x415F30` classifies a layout record as RTL only
when its selected face resolves to charset `0xB1` or `0xB2`; with record 11 present, that charset
is read directly from the per-face byte table.

The reordering routine operates on retained layout records rather than replacing characters. It
groups contiguous records by that face-charset classification and repositions the groups according
to the paragraph base direction. The Rust retained layout now follows the same policy for contiguous
text boxes: RTL groups are visually reversed, mixed LTR/RTL group order follows paragraph bit 13,
and tabs/non-text objects keep their independently established geometry. Text within each box stays
in logical Unicode order so the native renderer can perform glyph shaping, matching the division of
responsibility in WinHlp32.

LinkData2 decoding now follows the selected legacy charset before layout. Build-fix 34 retains the
existing Windows-1255/1256 Hebrew/Arabic paths and adds the common Windows SBCS families plus
Shift-JIS, Korean, GBK and Big5 DBCS families. Build-fix 39 closes the rare Johab case with a
deterministic CP1361 decoder. The Microsoft mechanism is now fully identified: `0x411E6F` indexes
the universal 11-byte descriptor and per-face record-11 byte, then `0x4334FA` lets GDI enumerate /
resolve the requested face and charset. When no record-11 table exists, Symbol/WingDings select
`SYMBOL_CHARSET`; otherwise WinHlp32 falls back to the host text charset at `0x43B09C`. The DBCS
gate special-cases only `0x80`, `0x81`, `0x86`, and `0x88`; `JOHAB_CHARSET` (`0x82`) and
`OEM_CHARSET` (`0xFF`) flow through as ordinary selected charsets.

At `0x416EB6` WinHlp32 calls `GetTextCharset`, resolves the code page through
`TranslateCharsetInfo` (with locale/default-ANSI fallback), and then converts with
`MultiByteToWideChar`. Thus Johab is Windows CP1361. OEM is not an HLP-defined code page at all: it
is intentionally a host GDI/OEM database decision. No universal OEM mapping is invented: on Windows the parser uses the active `CP_OEMCP` conversion,
while non-Windows builds retain an explicit deterministic fallback because the Windows OEM database
is not present.

The draw helper at `0x416F54..0x41706D` also resolves the former shaping question. It converts with
`MultiByteToWideChar` and normally emits Unicode through `TextOutW`; `TextOutA` is the conversion-
failure fallback. No private WinHelp shaping engine appears in this audited path. Build-fix 39
therefore preserves authored non-ANSI/default face names on Windows so the retained GDI backend
receives the same legacy face/charset pairing before the Windows font mapper performs its normal
work.

## Residual-gap audit: resolved semantics and remaining policy boundaries

A build-fix 39 re-audit of the exact 285,696-byte KB917607 executable closes the five items that
previously appeared under “Still unresolved”. They do not represent five unknown HLP format rules:

1. **Johab is resolved as CP1361; OEM is a host property.** The executable uses the selected GDI
   charset and `TranslateCharsetInfo`, not a private WinHelp encoding table. The viewer now decodes
   charset `0x82` as CP1361. `0xFF` remains intentionally host-defined because the same HLP can map
   through a different OEM environment on a different Windows installation. Windows builds now
   reproduce that host selection through `CP_OEMCP`; non-Windows builds keep a documented fallback.
2. **There is no hidden WinHelp shaping engine.** `0x416F54..0x41706D` converts source bytes with
   `MultiByteToWideChar` and normally paints with `TextOutW`, with `TextOutA` only as the failure
   fallback. On Windows the viewer already measures/paints through GDI; build-fix 39 also preserves
   the authored face for non-ANSI legacy charsets so GDI receives the reference face/charset pair.
   Non-Windows toolkit shaping remains a portability difference, not an unknown file-format rule.
3. **Character command `0x85` is a horizontal line-origin override.** Tokenizer `0x417816` writes
   the signed WORD to render-state `+0x38` and returns status 2. That state is initialized from
   paragraph horizontal geometry and consumed by the line alignment finalizer. The command emits no
   glyph. The independent scanner path uses it as a pending textual separator. The retained layout
   now applies the marker as a glyphless x-origin reset.
4. **The less-common hotspot variants are inert in this runtime.** The exact C0/C8 envelope rules
   remain important for synchronization, but the click dispatcher has no action branch for the
   residual envelope opcodes. The parser now diagnoses them as verified-inert instead of assigning
   an unresolved navigation meaning.
5. **Hosted-control sizing is runtime negotiation, not hidden HLP geometry.** Factory `0x4240F4`
   creates an arbitrary authored child initially at exactly `2 * LOGPIXELSX` by `2 * LOGPIXELSY`.
   Size query `0x42464E` first sends private message `0x706B`; a nonzero result supplies the control's
   requested dimensions. Otherwise WinHlp32 uses the actual `GetWindowRect` size. Consequently no
   static parser can know the final size without running the authored control. Build-fix 39 uses the
   verified two-inch creation rectangle for the safe placeholder while retaining the security rule
   that document-supplied native controls are never instantiated.

The earlier items for `VariableField`/`DType`, a 42-byte MVB style/character-map descriptor, border
trailing bytes, border styles 5..7, paragraph bit 0, and the core Hebrew/Arabic run-ordering path
remain closed as visual-format questions: direct tracing disproved the inherited interpretation,
established a render-inert/reserved path, or provided the implementation. What remains is mainly a
**portability/security boundary** (host OEM databases, non-Windows font shaping, and intentionally
blocked authored native code), not an unidentified HLP record layout.

## Table visibility: geometry is not a grid

The verified Microsoft table walker at `0x414F66` is a layout/dispatch routine. Between
`0x414F66` and its return at `0x4152CC`, it decodes table type/column geometry, tracks one
vertical cursor per column, and calls generic record dispatcher `0x417578` for each bounded cell.
It does **not** call GDI `MoveToEx`, `LineTo`, or `Rectangle`. Those imports are used by the
separate paragraph-border painter around `0x415379..0x4157xx`.

Consequently, a WinHelp table does not intrinsically paint cell outlines or a grid. Visual rules
inside a table are ordinary authored paragraph borders belonging to the cell content. In the
Calculator `Equivalentes de teclado` topic, the four independently flowing aligned columns under
the heading are the table. The two long horizontal rules visible above the first data row come
from paragraph border formatting; synthesizing vertical or horizontal table grid lines in the Rust
viewer would diverge from Microsoft WinHlp32.

## Build-fix 15 retained-layout corrections

The paragraph parser already retained the signed first-line metric correctly. The remaining defect
was in retained layout: `constrained_line()` placed an LTR first line at
`content_left + first_line_indent`, but `finish_line()` subsequently clamped every box back to the
unindented `line_left`. That erased negative first-line values used for hanging/outdented first
lines. Build-fix 15 clamps to the line's actual `start_x` instead. Wrapped continuation lines are
created with a zero first-line offset, so the authored metric affects only the first visual line.
The RTL path continues to apply the confirmed first-line metric from the right edge.

Border-only paragraphs are also no longer treated as if they contained an invisible 16-pixel text
line. The 5/6/7-pixel per-side border clearances established from the Microsoft painter remain
unchanged; only the non-authored empty-text fallback is removed when a paragraph contains no
visible inline content and exists solely to carry a border. This preserves both authored rules in
CALC.HLP while removing the exaggerated blank gap between them. Ordinary empty text lines and
explicit line breaks keep their text-line fallback behavior. Build-fix 24 adds the narrower verified
exception for completely empty compact display paragraphs inside table cells.

This change does not alter table semantics. As established in build-fix 14, table records provide
geometry/flow and visible rules come from paragraph borders. Contents source policy was refined in
build-fix 46: a discovered `.CNT` remains the primary authored model; when it is unavailable, a
verified same-basename WinHelp `.GID` can recover the cached hierarchy from `|CntText`, `|CntJump`,
and the observed `|Flags` hierarchy tail. Physical HLP topic order is exposed separately through
**Show all** rather than being presented as authored Contents.

## Build-fix 16 integrated-viewer presentation policy

Build-fix 16 intentionally changes two *viewer presentation* choices without changing the decoded
WinHelp metadata. First, popup and secondary-window destinations no longer create native child or
top-level frames. The parser continues to retain popup bits, window numbers/names, default-window
assignments, and popup macros so the destination can be resolved correctly, but the resolved topic
is installed into the single main viewer surface. This keeps hyperlinks usable and browser history
coherent while eliminating detached floating help windows altogether.

Second, the compact border-only paragraph pattern carrying only top+bottom horizontal edges is
normalized when used as a separator. In the retained layout it is aligned on the left to the region's content edge rather than preserving a separator-only left indent;
its authored right-side inset is retained, and it reserves 12 pixels before the following paragraph. The painter emits one horizontal rule at the top of that retained separator box
instead of drawing both top and bottom edges. This is the integrated viewer policy requested for the
`Related Topics` header treatment; ordinary paragraph boxes, side borders, shadows, and genuinely
styled double borders continue through the existing border painter.

The visible browse toolbox also uses explicit native-control spacing: 5 pixels above and below,
4 pixels within paired controls, and 10 pixels between logical groups. These values affect only the
viewer chrome and do not participate in HLP topic layout.


## Build-fix 18 hover restoration under the single-surface policy

Build-fix 16's decision to eliminate detached popup and secondary topic frames remains unchanged,
but it is independent from the destination metadata shown while the pointer rests on a hotspot.
Build-fix 18 restores the pre-build-fix-16 main-canvas hover presentation: internal, context-hash,
and cross-file text/image hotspots resolve the destination title into the native canvas tooltip.
When the authored hotspot carries popup semantics the tooltip is prefixed `Popup: `; ordinary links
show the destination title directly. The status-line description likewise distinguishes `Popup link`
from `Topic link`.

This restoration does **not** revive auxiliary topic windows. `show_topic_window()` remains the
single-surface routing shim: activating popup-marked or secondary-window destinations installs the
resolved topic into the main viewer and participates in the same Back/Forward history. Popup bits,
window names, and related metadata are therefore allowed to inform hover text without becoming a
request to construct a native frame.

The character-command audit from build-fix 17 is untouched. In particular, the KB917607 scanners
used for that audit reject `0x8B` and `0x8C`; they are not reintroduced as speculative commands in
order to implement this viewer-layer correction.

## Build-fix 20 popup hover content

Build-fix 18 restored the old hover plumbing but restored the wrong payload for popup-marked hotspots: it displayed a resolved destination label such as `Popup: Topic 6`. That is not useful for untitled glossary/context popup topics, whose meaningful authored content is their body text.

Build-fix 20 keeps ordinary jump-link hover behavior unchanged, but popup-marked internal, context-hash, and cross-file hotspots now resolve the destination `TopicPresentation` and flatten its visible text into the native tooltip. Paragraph boundaries, explicit line breaks, and tabs are preserved; pictures, retained controls, and other non-text objects contribute no invented caption. Textless popup topics fall back to the resolved title.

This is a hover-content correction only. Build-fix 16's single-surface activation policy remains in force: clicking a popup-marked hotspot still navigates the main help surface rather than constructing a detached popup frame.

## Build-fix 21: inline compact records and CALC Related Topics control

The Calculator topic that visually precedes `Tópicos relacionados` contains an inline-object command from the `0x86`/`0x87`/`0x88` family. The previous Rust decoder treated those character commands as synonymous with pictures. The retained 285,696-byte KB917607 WinHlp32 reference disproves that assumption: the command points at a compact TOPICLINK record, and the nested record's own type selects the generic renderer. The helper reached at `0x412884` parses that compact header; graphics are the `0x03`/`0x22` families, while `0x05`/`0x24` are hosted/custom-window families.

For the affected CALC.HLP object the nested type is `0x05`. Its six-byte hosted prefix is followed by the NUL-terminated descriptor:

```text
!,AL("A_CALC_LIST_EQUIV;A_CALC_KEYB_SEQ")
```

WinHlp32 hosted-control renderer `0x419281` calls the compact-header helper, advances six bytes into the payload, copies/splits the descriptor, and calls factory `0x4240F4`. At `0x424143` the factory detects the leading byte `0x21` (`!`). In that mode it splits the descriptor at the comma. The bytes after `!` and before the comma are the window/button label; the remainder is the authored macro. CALC therefore has an empty label and macro `AL("A_CALC_LIST_EQUIV;A_CALC_KEYB_SEQ")`.

The standard-button branch computes initial height as `12 + (label_nonempty ? 4 : 0)` around `0x4244CD..0x4244DF`, pushes initial width `0x1E` (30) at `0x4244E0`, passes class string `BUTTON` from `0x4020A0`, and calls imported `CreateWindowExA` at `0x42451F`. That is not the final empty-button size: when the label is empty, `0x424545..0x42455B` selects packed dimensions `0x000C000C`, and the following `MoveWindow` call at `0x424593` resizes the child to **12x12**. Non-empty labels instead pass through text measurement helper `0x42489D` before `MoveWindow`.

Thus this CALC object is **not an omitted bitmap**. Native WinHlp32 constructs a tiny blank 12x12 standard button at that location and subclasses it for the stored macro action. The viewer reproduces the verified visual geometry but deliberately leaves that macro inert under the existing default-deny hosted-code policy.

Build-fix 21 therefore changes the inline decoder from `0x86..0x88 => Picture` to nested compact-record dispatch. Real `0x03`/`0x22` records continue through the image pipeline, `0x05`/`0x24` become retained hosted controls, old `0x06` is consumed as no-render, and other structurally bounded compact records are skipped with layout-safe diagnostics rather than corrupting following LinkData1 parsing.

## Build-fix 24 empty table-cell display semantics

The retained table walker remains column-independent, matching the Microsoft routine at `0x414F66`: after dispatching one bounded cell through `0x417578`, WinHlp32 adds the returned child height only to that cell's signed-column cursor at `0x4151DC..0x4151E9`. The Calculator keyboard-equivalents table therefore must not be normalized into a conventional shared-row grid.

The alignment defect was instead in the child display height. CALC.HLP interleaves completely empty compact display records among visible cells. In the verified KB917607 display renderer, `0x415B44..0x415B57` first tests for an empty LinkData2 string and an immediate `0xFF` character-stream terminator. When both are present, `0x415B5C..0x415BA1` consumes the terminator/string and returns through the fast path before the ordinary paragraph-spacing path beginning at `0x415BA6`; in particular, the `spacing_above` addition at `0x415BAE..0x415BB2` is bypassed. Such a display therefore contributes no synthetic text-line advance.

Build-fix 24 mirrors that behavior specifically in retained table-cell display layout. An unbordered paragraph with no layout-bearing inline command (zero-width `0x85` markers are allowed) is skipped. Borders, explicit tabs and line breaks, pictures, and hosted controls remain layout-bearing. This makes CALC's four visible logical column groups share the intended baseline while preserving the reference's independent column cursors and recursive-table model.

As a viewer presentation refinement, a rule-only top-level display record immediately followed by a table reserves an additional 8 pixels before the first table cell. The gap is attached to that display-to-table transition only; it is not a per-table-record margin and therefore cannot accumulate between the keyboard-equivalents data records.



## Build-fix 25 mixed-font line baseline correction

The retained layout previously assigned every text run on a visual line the same top y-coordinate. That is only correct when all runs have the same measured height. CALC.HLP's `Dicas` list marker is a smaller square-glyph run beside the ordinary body font, so top alignment places the marker visibly above the text.

Build-fix 25 performs vertical text-run alignment during line finalization, after ordinary horizontal alignment and any Hebrew/Arabic charset-run reordering. The largest measured text height on the line establishes the retained baseline band; shorter text boxes are shifted downward by the height difference so their bottoms coincide. Non-text inline boxes are excluded. This preserves the existing native-width wrapping and line advance while correcting small symbol/font runs such as the Calculator bullets.


## Build-fix 28 native baseline retention

Build-fix 25-27 attempted to align the small CALC.HLP list marker from retained text-box height. That is insufficient: the native measurement API can return equal cell heights for different faces while their descents differ, so a height-only rule may make no change at all.

The retained metric now carries a baseline offset explicitly. In the wxWidgets viewer the baseline is derived from `GetFullTextExtent` data as the measured text height minus descent (external leading remains part of line height, not ascent). Each `LayoutKind::Text` retains that offset, and line finalization shifts text runs so `bounds.y + baseline` is common across the line. The headless fallback synthesizes a conservative baseline only when native ascent/descent is unavailable.

This changes only within-line text placement. Line advance, paragraph spacing, hanging indentation, table flow, pictures, hosted controls, and hotspot bounds continue through the existing paths.

## Build-fix 29: CALC inline list-marker baseline

The supplied Portuguese `CALC.HLP` resolves the two list markers that exposed the remaining
vertical-alignment defect as real compact graphics, not symbol-font text. In the relevant display
paragraphs, character command `0x86` encloses a nested `0x22` graphics record. Its four-byte indexed
payload selects `|bm1` for the triangular marker and `|bm0` for the square marker. Decoding those
streams yields natural sizes of 4x8 and 3x7 pixels respectively.

This also explains why build-fixes 25-28 did not visibly move these markers: those passes changed
text-run baseline handling, whereas the marker remained a `LayoutKind::Picture` positioned at the
unadjusted line top.

The KB917607 inline-object path at `0x416A73` dispatches the compact record through `0x417578`.
After the nested renderer returns, the loop at `0x416B64..0x416B70` stores, for each emitted object
record, the common object-bottom coordinate minus that record's y origin into field `+0x0A`.
Text layout uses the analogous field as its vertical line metric. For a single inline bitmap this
makes the bitmap's bottom edge its line baseline. Build-fix 29 mirrors that relationship in retained
layout: text contributes its measured font baseline, inline pictures contribute their retained
height, and all items are shifted to a common maximum baseline. Transparent graphical-hotspot
overlays are shifted by the same delta as their owning image. Floating pictures are not part of the
line slice and remain unaffected.


## Build-fix 31: Related Topics alignment uses rendered border geometry

Build-fix 30 incorrectly inferred horizontal placement by replacing the stock ALink paragraph's authored left and first-line indents with zero. The verified KB917607 hosted-button trace does not justify that metadata rewrite: dispatcher `0x419281` reaches factory `0x4240F4`, and the empty-label path sets a packed 12x12 size before the `MoveWindow` call at `0x424593`. That establishes control dimensions, not a universal x-coordinate.

The retained renderer now completes normal paragraph layout first. A rule-only top-level record records the x-coordinate of its emitted `Border`; if the immediately following record contains the stock ALink button, the 12x12 button and text boxes sharing its visual line are shifted by `rule_left - button_left`. Authored paragraph indents therefore continue to influence normal layout/wrapping, while the final Related Topics row follows the actual separator edge rather than assuming `PAGE_MARGIN`.


## Build-fix 32: native per-axis DPI and retained half-point font sizes

The retained parser has preserved HLP font sizes in twentieths of a point since the font-model correction, but the wxWidgets front end still created fonts through an integer-point constructor. That final conversion rounded an authored half-point size before native measurement and therefore could change glyph width, baseline, wrapping, paragraph height, and hotspot placement even though the parsed descriptor was correct.

Build-fix 32 keeps the retained twip value through viewer zoom. On Windows the topic canvas obtains a GDI device context from its native HWND, reads `LOGPIXELSX` and `LOGPIXELSY`, and creates a `LOGFONTW` whose negative `lfHeight` is derived directly from the zoomed twip value and vertical device DPI. The same selected GDI font is used by `GetTextExtentPoint32W` / `GetTextMetricsW` for retained measurement and by `TextOutW` for painting. This keeps the layout and paint paths synchronized instead of measuring one rounded font and drawing another. The wxDragon integer-point font path remains a portable fallback.

The layout engine now retains DPI per axis. LinkData1 x geometry (left/right/first-line indents and tab stops), table widths/gaps, and headless fallback text width use horizontal DPI. Paragraph spacing, line spacing, and headless fallback font height use vertical DPI. The old `LayoutEngine::new(dpi)` constructor remains a square-DPI convenience wrapper; native front ends use `LayoutEngine::with_dpi(dpi_x, dpi_y)`.

Build-fix 33 subsequently extends the same per-axis device-DPI context to picture natural sizing, so the 96-DPI/raw-pixel assumption described here no longer applies to retained display geometry.


## Build-fix 33: bitmap and WMF physical-resolution sizing

The KB917607 graphics path around `0x40661A..0x406718` distinguishes raw bitmap pixel extents from
authored physical size. For bitmap alternatives `0x05`/`0x06`, when both retained resolution fields
are nonzero, the reference computes the displayed axes independently:

```text
display_width  = pixel_width  * LOGPIXELSX / x_resolution
display_height = pixel_height * LOGPIXELSY / y_resolution
```

Build-fix 33 preserves those fields as picture-sizing metadata rather than discarding them after
decoding. Zero-resolution records retain their raw-pixel natural size. This metadata affects only
layout geometry: the decoded RGBA pixel buffer is unchanged, and the existing painter/hotspot path
performs proportional scaling to the retained picture box.

Legacy WMF alternatives likewise no longer use a fixed 96-DPI assumption for their natural layout
box. The retained helper at `0x4072C7` treats mapping modes 7/8 explicitly as HIMETRIC, using
`MulDiv(extent, LOGPIXELS*, 2540)` per axis; the other mapping modes flow through `SetMapMode` plus
`LPtoDP`, with mapping mode 1 remaining pixel-for-pixel. Build-fix 33 mirrors those physical-unit
conversions with the layout engine's actual `dpi_x` and `dpi_y`. The narrow Windows WMF adapter
may still rasterize to a bounded 96-DPI compatibility bitmap internally. Separating rasterization
from natural document geometry avoids coupling safety limits to the monitor DPI while preserving
the reference display size.


## Build-fix 34: legacy charset decoding and CJK wrapping

The reference charset helper at `0x411E6F` uses the per-face record-11 byte when available and falls
back to GDI face/default charset selection otherwise. The later Unicode path around
`0x416EB6..0x416FF8` obtains the selected text charset, translates it to a code page, and converts
the multibyte source before Unicode drawing. Build-fix 34 mirrors that division without executing
platform locale APIs inside the parser.

The portable decoder now handles the common Windows SBCS families (1250/1251/1252/1253/1254,
1255/1256, 1257/1258 and Thai) plus the major Japanese, Korean, Simplified-Chinese and
Traditional-Chinese DBCS families. Explicit non-default record-11 charset bytes take precedence.
When metadata is absent/default, common legacy face names and record-9 `LANGID` values provide a
deterministic inference. Build-fix 39 closes Johab as Windows CP1361. `OEM_CHARSET` is a host-defined
GDI/code-page choice; Windows builds use the active `CP_OEMCP`, while non-Windows builds retain the
documented fallback rather than pretending one OEM page is universal.

Retained wrapping also recognizes CJK text that has no ASCII spaces. Ideographic, kana and hangul
characters become legal break units while Latin words remain grouped; simple opening punctuation
stays with the following unit and closing punctuation with the preceding unit. This is deliberately
a conservative legacy line-break model rather than a new shaping engine: final glyph shaping and
font fallback remain the responsibility of the native painter.


## Build-fix 35: zoomed line advance and restored-window reflow

Viewer text zoom is intentionally separate from WinHlp32 device DPI. Native measurement already
creates the zoomed font before retained layout, but pre-build-fix-35 signed `spacing_lines` values
were converted only by `LOGPIXELSY`. Negative values are exact WinHlp32 line advances at native
scale, so at 150%-200% they could remain close to the 100% pitch while glyph cells grew.
Build-fix 35 retains the 100% signed/minimum semantics and multiplies only the authored line-advance
value by the viewer zoom percentage. Paragraph x geometry, physical picture dimensions, and target
device DPI remain unchanged.

The maximize/restore correction is a viewer-window lifetime rule rather than an HLP format rule.
The frame size notification may precede wxWidgets sizer propagation to the content host; querying
the scrolling viewport there can therefore return the old maximized width. Build-fix 35 listens to
the content host size event instead, rebuilds retained layout from the post-sizer viewport width,
and invalidates newly exposed native page/background areas. This guarantees that restored-window
wrapping and the cream page rectangle agree with the actual client area.
