# Microsoft WinHelp (.HLP) Internal Format

## Reference Manual

**Project reference:** rust-hlp-viewer v0.7.1-buildfix46\
**Reference runtime:** Microsoft WinHlp32 from Windows 8.1 KB917607 x64  
**Reference executable:** `winhlp32.exe`, x86, 285,696 bytes  
**SHA-256:** `8496f19bc1d898180b57aac61326bdfcf5a48f760811283bbd604aa7a2c13f85`  
**Document date:** 2026-07-26

This manual consolidates the HLP container model implemented by the Rust viewer, the behavior recovered from real Windows Help files, and the address-level findings obtained by auditing the hash-verified Microsoft WinHlp32 executable. It is deliberately more conservative than older secondary descriptions: where a byte sequence is structurally known but its semantic effect is not established, the document says so.

### Evidence labels

| Label | Meaning in this manual |
|---|---|
| **VERIFIED** | Established directly from the hash-verified KB917607 WinHlp32 executable, or independently cross-checked against concrete HLP bytes in a way that fixes the interpretation. |
| **STRONG INFERENCE** | Supported by the file structure, multiple producers/fixtures, parser behavior, or a consistent implementation trace, but not claimed as a direct Microsoft-runtime proof. |
| **UNRESOLVED** | The structure can be retained or bounded, but its complete legacy semantics have not been proven and are intentionally not invented. |

> Scope note. “Verified” does not mean that every historical WinHelp build must be byte-for-byte identical. Address claims in Part III apply specifically to the 285,696-byte executable identified above. Format claims are stated at the narrowest confidence level supported by the evidence.

# Contents

1. Purpose, provenance, and methodology  
2. HLP container and internal named streams  
3. `|SYSTEM`: generation, metadata, windows, locale, and charset tables  
4. Phrase compression and `LinkData2` expansion  
5. `|TOPIC`, `TOPICPOS`, `TOPICOFFSET`, and `TOPICLINK`  
6. Paragraph geometry, character streams, and synchronization  
7. `|FONT`, GDI font semantics, charsets, and DPI  
8. Tables and recursive compact records  
9. Graphics: bitmap, DDB, DIB, WMF, and image hotspots  
10. Text hotspots and navigation  
11. Context maps, keywords, browse sequences, `.CNT`, and `.GID`  
12. Hosted controls and `!label,macro` BUTTON forms  
13. WinHelp macros and safe execution semantics  
14. Corrections to received WinHelp lore  
15. Confidence matrix and remaining compatibility boundaries\
16. WinHlp32 executable audit and address-level findings  
17. Quick-reference tables  
18. Executable-address appendix

# 1. Purpose, provenance, and methodology

**Status: VERIFIED**

The primary Microsoft reference used by this project is the WinHlp32 executable reconstructed from the user-supplied Windows 8.1 KB917607 x64 package. The target is an x86 PE image with length 285,696 bytes and SHA-256 `8496f19bc1d898180b57aac61326bdfcf5a48f760811283bbd604aa7a2c13f85`. The source tree does not redistribute Microsoft’s executable; the supplied extraction scripts reconstruct it from the user’s own KB package and verify the result.

The reverse-engineering method is intentionally layered:

1. Parse container structures with strict bounds checks and preserve unknown records rather than treating them as padding.
2. Cross-check real HLP files from different compiler generations to distinguish generation changes from accidental file-specific layout.
3. Trace WinHlp32 dispatchers and helpers when a rendering or synchronization question cannot be resolved safely from secondary descriptions.
4. Convert only proven semantics into renderer behavior; retain unresolved bytes or opcodes as inert metadata where possible.
5. Maintain synchronization as the first priority. A harmless-looking wrong field length can corrupt every later paragraph, string, picture, or hotspot in the record.

This methodology is why several earlier assumptions in the viewer were later removed. The most important example is the abandoned “modern/MVB 42-byte font descriptor” model: direct WinHlp32 indexing proves an 11-byte descriptor stride across the audited generations.

## 1.1 Generation terminology

**Status: STRONG INFERENCE**

The project uses the `|SYSTEM` minor version as the stable generation discriminator:

| Minor | Project name | Typical family |
|---:|---|---|
| 15 | Windows30 | HC30 / Windows 3.0 |
| 21 | Windows31 | HC31 / Windows 3.1 |
| 27 | Multimedia | WMVC/MMVC / MVB family |
| 33 | Windows95 | HCW 4.0 / Windows 95 generation |

Unknown minor values are retained rather than coerced into a known generation.

# 2. HLP container and internal named streams

**Status: STRONG INFERENCE**

A classic HLP is a container of internal named files. The outer fixed header is 16 bytes. The project validates the magic `0x00035F3F`, then reads the internal-directory offset, free-list head, and logical whole-file size. All later reads are restricted to that logical size even when the host file has trailing bytes.

## 2.1 Fixed outer header

| Offset | Width | Field | Notes |
|---:|---:|---|---|
| `0x00` | 4 | Magic | `0x00035F3F` |
| `0x04` | 4 | Directory start | Absolute file offset of the directory internal file’s `FILEHEADER` |
| `0x08` | 4 | First free block | Signed; `-1` means no free-list head |
| `0x0C` | 4 | Entire file size | Logical HLP length used for all bounds checks |

## 2.2 Internal `FILEHEADER`

**Status: STRONG INFERENCE**

Every internal named file is reached through a nine-byte header:

| Field | Width | Meaning |
|---|---:|---|
| Reserved space | 4 | Total reserved bytes including the 9-byte header |
| Used space | 4 | Number of payload bytes after the header |
| Flags | 1 | Legacy internal-file flags byte |

`used_space + 9` must fit inside `reserved_space`; the complete reserved range must fit inside the logical outer HLP size. The parser exposes only the bounded used payload to stream-specific decoders.

## 2.3 Internal directory B+ tree

**Status: STRONG INFERENCE**

The directory itself is an internal file whose payload is a B+ tree. Its fixed tree header is 38 bytes and begins with magic `0x293B`. Leaf records contain a NUL-terminated internal filename and a 32-bit absolute offset to that named file’s `FILEHEADER`. The Rust parser walks from the root to the leftmost leaf, then follows leaf links in logical order with cycle detection and verifies that the observed entry count equals the header’s declared count.

Internal names are treated ASCII-case-insensitively. This matters for producer variation and for side streams such as bitmap resources.

## 2.4 Named-stream map

**Status: STRONG INFERENCE**

The following streams are implemented or explicitly recognized by the current project. Not every file contains every stream.

| Stream/family | Purpose |
|---|---|
| <code>&#124;SYSTEM</code> | Mandatory generation and project metadata; CONFIG macros; windows; locale; per-face charset bytes; contents filename |
| <code>&#124;TOPIC</code> | Physical topic blocks and `TOPICLINK` records |
| <code>&#124;FONT</code> | Face directory plus 11-byte font descriptors |
| <code>&#124;Phrases</code> | Classic phrase dictionary and phrase image |
| <code>&#124;PhrIndex</code> + <code>&#124;PhrImage</code> | Hall phrase compression dictionary |
| <code>&#124;TOMAP</code> | HC30 TopicNumber -> `TOPICPOS` map |
| <code>&#124;CONTEXT</code> | Context-hash -> `TOPICOFFSET` B+ tree |
| <code>&#124;CTXOMAP</code> | Numeric MapID -> `TOPICOFFSET` array |
| <code>&#124;TopicId</code> | HCW `TOPICOFFSET` -> symbolic context name B+ tree |
| <code>&#124;Viola</code> | HCW `TOPICOFFSET` -> default window number B+ tree |
| <code>&#124;?WBTREE</code> + <code>&#124;?WDATA</code> | Keyword / associative-link tables; `K` is the normal Index table, `A` is used by ALink |
| <code>&#124;bmN</code> | Indexed logical graphics stream N |

The directory can contain additional producer-specific streams. Unknown streams are not evidence of corruption.

# 3. `|SYSTEM`: generation, metadata, windows, locale, and charset tables

**Status: STRONG INFERENCE**

The mandatory `|SYSTEM` stream begins with magic `0x036C`, followed by minor version, major version, generation timestamp, and flags. For Windows 3.0-era files (`minor <= 16`) the remainder can be the title string. Later generations use typed length-prefixed records.

## 3.1 Topic block and compression selection

| Condition | Compression | Physical block | Decoded data capacity |
|---|---|---:|---:|
| `minor <= 16` | none | 2048 | 2036 |
| flags `0` | none | 4096 | 4084 |
| flags `4` | LZ77 | 4096 | 16384 |
| flags `8` | LZ77 | 2048 | 16384 |

Unrecognized flags are retained as an unknown compression mode rather than guessed.

## 3.2 Modern `|SYSTEM` records

| Type | Current interpretation | Confidence |
|---:|---|---|
| 1 | Help title string | Strong inference |
| 2 | Copyright string | Strong inference |
| 3 | Contents topic offset | Strong inference |
| 4 | CONFIG macro string; multiple records allowed | Strong inference |
| 6 | Ordinary 90-byte `[WINDOWS]` definition outside Multimedia generation | Strong inference |
| 9 | Exactly 10 bytes in audited WinHlp32; final WORD is locale/LANGID source | Verified |
| 10 | Associated `.CNT` filename | Strong inference |
| 11 | Byte array indexed by <code>&#124;FONT</code> face index to select GDI charset | Verified |
| other | Retained raw | Unresolved |

## 3.3 Window definitions

**Status: STRONG INFERENCE**

The ordinary 90-byte modern window record retains validity flags, a 10-byte type field, a 9-byte name, a 51-byte caption, normalized x/y/width/height values, maximize/style information, scrolling and non-scrolling `COLORREF`s, always-on-top state, and auto-size-height state. Multimedia record type 6 is structurally different and must not be forced through this layout.

The current viewer deliberately presents popup and secondary-window destinations in one integrated main surface, but the parser retains the authored window metadata because it is part of the HLP semantics and is needed for compatibility decisions.

## 3.4 Locale and per-face charset metadata

**Status: VERIFIED**

WinHlp32’s record-11 dispatcher allocates exactly `data_size + 1` bytes and copies the record payload. Font selection later indexes one byte at `[table + face_index]`. Therefore the record is a per-face GDI charset table, not a global 16-bit charset field.

Record 9 is accepted by the audited path only when its size is 10 bytes. The final WORD becomes the locale/LANGID source and is later masked with `0x03FF` before the Arabic/Hebrew reordering path.

# 4. Phrase compression and `LinkData2` expansion

**Status: STRONG INFERENCE**

Visible strings are carried in `LinkData2`, but many HLP files phrase-compress them. The project chooses the phrase generation from streams present in the file:

| Streams | Compression family |
|---|---|
| none | No phrase table; stored `LinkData2` must already match advertised size |
| <code>&#124;Phrases</code> | Classic phrase dictionary |
| <code>&#124;PhrIndex</code> + <code>&#124;PhrImage</code> | Hall phrase compression |

Classic `|Phrases` begins with a phrase count and marker `0x0100`, followed by offsets and a phrase image. HC31+ can LZ77-compress the phrase image. Some Multimedia/MVB files use a `0x0800` prefix and 30 reserved bytes before the image data; this is a generation-specific prefix, not a universal phrase layout.

Hall compression uses the paired index/image streams and a different token scheme. Expansion is always bounded by the record’s advertised decoded `DataLen2`, and mismatched lengths are treated as malformed input rather than silently tolerated.

# 5. `|TOPIC`, `TOPICPOS`, `TOPICOFFSET`, and `TOPICLINK`

**Status: STRONG INFERENCE**

`|TOPIC` is physically block-oriented but logically traversed as transformed topic data. Every physical topic block begins with three signed 32-bit positions: `LastTopicLink`, `FirstTopicLink`, and `LastTopicHeader`. The 12-byte physical header is not part of the transformed data capacity.

## 5.1 `TOPICPOS` versus `TOPICOFFSET`

**Status: STRONG INFERENCE**

These two values must remain distinct:

- `TOPICPOS` is a logical byte position in transformed `|TOPIC` data and identifies actual compact records.
- `TOPICOFFSET` is a navigation-oriented cursor value used by contexts and hotspots. Its block portion selects a topic block, while the low character-count portion advances using display/table `TopicLength` values.

Conflating them can produce links that appear numerically valid but resolve to the wrong record family.

## 5.2 Physical block transformation

Compressed HC31+/HCW blocks are LZ77-decoded into their logical block data before `TOPICPOS` is interpreted. HC30 next-link values are physical relative distances and must account for physical block headers; HC31+ next links are absolute transformed `TOPICPOS` values.

The parser does not allow the fixed 21-byte `TOPICLINK` header to straddle transformed blocks. Record payloads can be read logically across block boundaries, but header crossing is treated as corruption rather than stitched across compiler padding.

## 5.3 The 21-byte `TOPICLINK` header

| Field | Width | Meaning |
|---|---:|---|
| `BlockSize` | 4 signed | Total compact record bytes including the 21-byte header |
| `DataLen2` | 4 signed | Expected phrase-expanded `LinkData2` length |
| Previous | 4 signed | Previous link pointer/value |
| Next | 4 signed | Next link; HC30 physical-relative, HC31+ transformed absolute |
| `DataLen1` | 4 signed | Offset/end of `LinkData1`, measured from record start |
| Record type | 1 | TOPICLINK generation/type byte |

`LinkData1` is the structured formatting/control payload between bytes 21 and `DataLen1`; stored `LinkData2` follows through `BlockSize` and is phrase-expanded to `DataLen2`.

## 5.4 TOPICLINK record families

**Status: VERIFIED** for dispatcher mapping; **STRONG INFERENCE** for topic grouping metadata.

| Old | Modern | Meaning |
|---:|---:|---|
| `0x01` | `0x20` | Ordinary display text |
| `0x02` | `0x21` | Topic metadata/header |
| `0x03` | `0x22` | Standalone graphics/special display |
| `0x04` | `0x23` | Table |
| `0x05` | `0x24` | Hosted/custom-window object |
| `0x06` | - | Old no-render dispatcher case |

A type-2/`0x21` record establishes topic metadata and region boundaries. Displayable records are classified as non-scrolling, scrolling, or temporarily unclassified according to those boundaries; the current viewer has a recovery path for displayable records that otherwise become invisible because of inconsistent authored ranges.

# 6. Paragraph geometry, character streams, and synchronization

This chapter summarizes the semantic layers before the address-level audit in Part III.

## 6.1 ParagraphInfo

**Status: VERIFIED**

A classic paragraph payload begins with one compressed signed long whose rendering semantics remain unresolved, followed by the paragraph identifier/flag word and optional fields controlled by that flag word. The signed-long decoder has two forms:

- low bit clear: `(u16 >> 1) - 0x4000`
- low bit set: `(u32 >> 1) - 0x40000000`

The flag word controls spacing, line spacing, indents, tabs, borders, alignment, no-wrap, and RTL state. All of these are detailed with addresses in Part III.

## 6.2 LinkData1 / LinkData2 synchronization

**Status: VERIFIED**

After ParagraphInfo, `LinkData1` becomes a command stream while `LinkData2` supplies NUL-terminated text strings. The two streams are consumed in lockstep. Font commands alter running state; line/tab/picture/hotspot commands consume structural bytes; text-producing boundaries consume the next string with the charset selected by the current font.

The cardinal rule is that a command must consume exactly the bytes WinHlp32 consumes. A false inline command interpretation does not merely mis-render one token: it shifts the command cursor and can reinterpret ordinary bytes as pictures, fonts, hotspot lengths, or paragraph terminators.

## 6.3 Verified character commands

| Opcode | Meaning | Confidence |
|---:|---|---|
| `0x80` | Select font descriptor/index; running state | Verified |
| `0x81` | Line break / progress boundary | Verified |
| `0x82` | Next paragraph using same ParagraphInfo | Verified |
| `0x83` | Tab | Verified |
| `0x85` | Signed 16-bit horizontal line-origin override; glyphless | **Verified** |
| `0x86` | Inline `bmc` picture/object | Verified |
| `0x87` | Left-floating `bml` picture | Verified |
| `0x88` | Right-floating `bmr` picture | Verified |
| `0x89` | End hotspot | Verified |
| `0xC8`, `0xCC` | Macro hotspot forms | Verified structurally |
| `0xE0..0xE3`, `0xE6..0xE7` | Internal hotspot families | Verified with destination distinctions in §10 |
| `0xEA`, `0xEB`, `0xEE`, `0xEF` | External / explicit-window hotspot families | Verified |
| `0xFF` | Paragraph/record transition | Verified |

`0x20`, `0x21`, `0x8B`, and `0x8C` are not accepted character commands by the audited scanner. The first two are TOPICLINK generations, not inline VariableField/DType commands.

# 7. `|FONT`, GDI font semantics, charsets, and DPI

## 7.1 Font stream structure

**Status: VERIFIED**

The `|FONT` prefix is eight bytes: face count, descriptor count, face-directory offset, and descriptor-directory offset. WinHlp32 indexes every font descriptor in **11-byte strides**. Compiler generation changes the fixed face-name slot width, not the descriptor record shape.

| Generation condition | Face slot |
|---|---:|
| pre-HCW-4 / minor < 33 | 20 bytes |
| HCW 4.0 / minor 33 | 32 bytes |

Each descriptor is:

| Byte(s) | Meaning |
|---:|---|
| 0 | Attribute bits |
| 1 | Size in half-points |
| 2 | Family selector |
| 3..4 | Face-name table index |
| 5..7 | Foreground RGB |
| 8..10 | Background RGB |

The old speculative 42-byte descriptor/style/character-map model is rejected by the audited executable.

## 7.2 Font size and attribute behavior

**Status: VERIFIED**

Half-point sizes are preserved through the device conversion instead of being rounded to whole points. Attribute bits used by the classic builder include bold (`0x01`), italic (`0x02`), underline (`0x04`), strikeout (`0x08`), and reduced-height small caps (`0x20`). The audited classic path does not consume bit `0x10` as a double-underline request.

## 7.3 Foreground/background sentinel

**Status: VERIFIED**

For both descriptor colors, exact `RGB(1,1,0)` / `COLORREF 0x00000101` means “retain the currently active/default color.” It is not a near-black range. Nearby dark values remain authored colors. WinHlp32 uses opaque text background output in this path, so descriptor background colors are real formatting semantics.

## 7.4 Font selection lifetime

**Status: VERIFIED**

Font selection is running topic-render state, not paragraph-scoped state. Opcode `0x80` is the writer. Paragraph transitions do not reset the global selected-font slot. A decoder that resets at every paragraph silently substitutes descriptor 0 when the HLP intentionally omits a redundant font command.

## 7.5 Charset decoding

**Status: VERIFIED** for record-11 selection and RTL gating; **STRONG INFERENCE** for deterministic host-independent fallback.

The current engine supports the common Windows single-byte families (Western, Central/Eastern European, Cyrillic, Greek, Turkish, Hebrew, Arabic, Vietnamese, Baltic, Thai) and major Japanese/Korean/Simplified-Chinese/Traditional-Chinese DBCS families. An explicit non-default record-11 charset wins. When metadata is absent or `DEFAULT_CHARSET`, the viewer uses deterministic legacy face-name/LANGID inference so decoding does not depend on the host machine’s installed font mapper.

Build-fix 39 closes the two rare charset questions. `JOHAB_CHARSET` (`0x82`) reaches the same `TranslateCharsetInfo` / `MultiByteToWideChar` path as other selected GDI charsets and therefore denotes Windows CP1361; the viewer now includes a deterministic CP1361 decoder. `OEM_CHARSET` (`0xFF`) is different in kind: WinHlp32 delegates it to the active host GDI/OEM charset database, so there is no single portable OEM code page encoded by the HLP bytes. On Windows, the viewer now mirrors that delegation with `MultiByteToWideChar(CP_OEMCP, ...)`; non-Windows builds retain a deterministic fallback because no authoritative Windows OEM database exists there.

The same trace removes the former shaping ambiguity. The audited draw path converts legacy source bytes with `MultiByteToWideChar`, normally calls `TextOutW`, and falls back to `TextOutA` only if Unicode conversion fails. There is no separate WinHelp-private shaping engine in this path. On Windows, the viewer therefore preserves authored face names for non-ANSI/default legacy charsets and passes the retained face/charset pair into its existing GDI `LOGFONTW` measurement/painting backend. A non-Windows toolkit can still differ from historical GDI; that is a platform rendering difference, not an unresolved HLP record semantic.

## 7.6 DPI and physical-size semantics

**Status: VERIFIED**

Paragraph geometry uses `raw * device_DPI / 144` with x/y axes kept separate. Font half-point sizes convert through vertical DPI without whole-point truncation. Bitmap alternatives with nonzero authored x/y resolution use independent natural-size conversion `pixels * target_dpi / authored_resolution`; zero-resolution bitmap alternatives retain raw pixel sizing. WMF physical mapping modes similarly use target per-axis DPI for display geometry.

# 8. Tables and recursive compact records

## 8.1 Tables are column flows, not HTML rows

**Status: VERIFIED**

The audited table renderer maintains one cumulative vertical cursor per column. A cell starts at `table_y + column_height[column]`, and rendering advances only that column. The table’s returned height is the maximum of all column cursors. There is no shared row baseline that forces neighboring columns down after a tall cell.

## 8.2 Header and column geometry

**Status: VERIFIED**

A table header contains a column count (maximum 32), table type, an extra minimum-width WORD only for type 0, then one four-byte record per column: unsigned width followed by unsigned gap-before. Type 0 uses proportional geometry against a 32767-unit reference span after DPI/144 conversion; nonzero types use absolute DPI/144 metrics.

## 8.3 Cell framing and recursion

**Status: VERIFIED**

Every cell begins with a **signed 16-bit column index**. `-1` terminates the cell list. A nonnegative index is followed immediately by a complete bounded compact TOPICLINK record. There is no fixed five-byte cell prelude before ParagraphInfo.

The same generic dispatcher handles nested table records. A nested table receives the containing cell’s current x/y origin and exact parent-column width, lays out its own columns, returns its maximum height, and advances only the parent column that contained it. Font state and the `LinkData2` string stream remain shared through the recursive subtree.

## 8.4 Compact record framing

**Status: VERIFIED**

The compact-header helper accepts `0x01..0x06` and `0x20..0x24`. Ordinary visual records carry a compressed signed payload size; modern records additionally carry compressed unsigned `TopicLength`. Types `0x02`/`0x21` use their fixed-width topic-header form. The next cell/record begins at **exact compact-header length + decoded payload size**.

# 9. Graphics: bitmap, DDB, DIB, WMF, and image hotspots

## 9.1 Graphics source selection

**Status: VERIFIED** for compact selector behavior; **STRONG INFERENCE** for format decoding details.

Inline pictures and compact graphics eventually enter the same graphics loader. A compact graphics payload whose selector WORD is zero is followed by a signed WORD internal bitmap index (`|bmN`); negative indices are rejected. Any nonzero selector means an embedded logical graphics object follows.

Indexed `|bmN` and embedded objects can contain multiple alternatives. The current renderer prefers a DIB alternative (`0x06`), falls back to portable DDB (`0x05`), then accepts a Windows metafile alternative (`0x08`).

## 9.2 Bitmap decoding

**Status: STRONG INFERENCE**

Implemented DIB depths are 1, 4, 8, 16, 24, and 32 bpp. Portable DDB support covers 1/16/24/32-bit cases that do not require unresolved external palette interpretation. Windows bottom-up scan lines and 32-bit row alignment are honored. Packing modes are raw (`0`), WinHelp RLE (`1`), LZ77 (`2`), and LZ77 then RLE (`3`). The flagged last palette entry can become transparent under WinHelp’s important-colour convention.

Decoded images are normalized to top-down RGBA so the GUI layer never reparses HLP graphics bytes.

## 9.3 WMF

**Status: STRONG INFERENCE**

WMF alternative type `0x08` is decompressed in the HLP engine and rasterized through a narrow Windows GDI adapter. Display geometry remains separate from the bounded raster surface: physical mapping modes use target DPI to determine natural size, while the compatibility raster can remain at its safe internal DPI and then be scaled to the retained layout box.

## 9.4 Graphical hotspots

**Status: STRONG INFERENCE**

Bitmap/metafile records can contain a declared-size hotspot table with fixed 15-byte geometry/action records plus paired hotspot-name/link-name strings. Supported semantic targets include macro actions, same-file context targets, and named-window context targets. The retained layout scales hotspot coordinates whenever the picture is resized and places invisible hit-test rectangles in front of the picture.

# 10. Text hotspots and navigation

## 10.1 Internal destination encodings

**Status: VERIFIED**

Two internal text-hotspot destination encodings must be distinguished:

- `0xE0` / `0xE1`: physical `TOPICOFFSET` destinations.
- `0xE2` / `0xE3` / `0xE6` / `0xE7`: context hashes resolved through `|CONTEXT`, with a compatibility physical-offset fallback for unusual producers.

Treating the second family as raw `TOPICOFFSET` produces huge bogus unresolved values in real help files.

## 10.2 Popup versus normal navigation

**Status: VERIFIED**

For external text and graphical context families, the low bit carries the popup/jump distinction: even opcodes are popup-marked and odd opcodes are normal navigation links. Thus `0xEA`/`0xEE` are popup-marked; `0xEB`/`0xEF` are normal jumps.

The current viewer intentionally routes both into its single main surface while retaining popup metadata for hover/status behavior. That presentation decision is project policy, not an assertion that WinHlp32 itself never creates auxiliary windows.

## 10.3 Variable hotspot envelopes

**Status: VERIFIED**

The scanner accepts the full `C0..CF` / `E0..EF` hotspot envelope. Fixed variants selected by `(opcode & 0xD8) == 0xC0` are five bytes total: opcode plus four-byte payload. Variable variants selected by `(opcode & 0xD8) == 0xC8` are opcode, WORD payload length, then exactly that payload.

The build-fix 39 activation trace at `0x429C13..0x429E24` resolves the semantic question for this exact KB917607 runtime. Click-action branches exist for `0xC8`/`0xCC` macro hotspots, `0xE0..0xE3` and `0xE6`/`0xE7` internal/context destinations, and `0xEA`/`0xEB` plus `0xEE`/`0xEF` external families (with unrelated command `0xB0` handled separately). The remaining structurally accepted envelope values simply fall through without navigation or macro dispatch. They are therefore retained as **KB917607-inert**, not as unknown destinations.

## 10.4 External hotspot structure length

**Status: VERIFIED**

For external hotspot commands, signed 16-bit `SizeOfFollowingStruct` describes the following structure itself: type byte, 32-bit destination, and optional window/file strings. It does not include the command byte or the size field. Getting this wrong desynchronizes the rest of `LinkData1`.

# 11. Context maps, keywords, browse sequences, `.CNT`, and `.GID`

## 11.1 HC30 `|TOMAP`

**Status: STRONG INFERENCE**

`|TOMAP` is a flat little-endian array of `TOPICPOS` values indexed directly by TopicNumber. The historic normal-topic base of 16 is not subtracted. Element zero identifies the project INDEX/contents topic. HC30 previous/next browse numbers are resolved through this map before matching a reconstructed type-2 header.

## 11.2 `|CONTEXT`, `|CTXOMAP`, `|TopicId`, `|Viola`

**Status: STRONG INFERENCE**

`|CONTEXT` is a B+ tree of signed 32-bit context hashes to `TOPICOFFSET`; the project uses the documented signed-character hash table with wrapping `hash = hash * 43 + table[byte]` arithmetic. `|CTXOMAP` is a counted flat mapping of signed map IDs to `TOPICOFFSET`.

HCW files can additionally contain `|TopicId` (`TOPICOFFSET` -> symbolic ContextName) and `|Viola` (`TOPICOFFSET` -> DefaultWindowNumber). `|Viola` assignments are treated as exact offsets, not inherited from the nearest preceding entry.

## 11.3 Browse sequences

**Status: STRONG INFERENCE**

HC30 topic headers can carry previous/next topic numbers that are resolved through `|TOMAP`. HC31+ headers can carry previous/next `TOPICOFFSET` values. These authored browse sequences are distinct from the viewer’s physical Previous/Next-topic navigation.

## 11.4 `.CNT` authored contents

**Status: STRONG INFERENCE**

WinHelp contents hierarchy can live outside the HLP in a `.CNT` sidecar. The current project first checks the filename named by `|SYSTEM` record 10, then tries a same-basename `.cnt` case-insensitively. The text is decoded as Windows-1252. Numbered lines create the hierarchy; targets can contain context, external help file, and named window components. `:Title`, `:Base`, `:Index`, and `:Link` are parsed. Unknown directives are warning-only.

## 11.5 WinHelp 4.x `.GID` compiled Contents cache

**Status: STRONG INFERENCE / SPECIMEN-VERIFIED**

Windows 95 testing with the supplied Portuguese WordPad help set establishes an important lifecycle distinction. After WinHelp has created a Contents-bearing `WORDPAD.GID`, renaming/removing `WORDPAD.CNT` does not destroy the hierarchical Contents view. Removing/renaming both CNT and GID makes the Contents control unavailable while the authored Index remains available from HLP keyword structures. With CNT absent, WinHelp does not synthesize a replacement Contents-bearing GID from the HLP alone. Therefore the GID can preserve navigation state compiled from CNT, but the HLP by itself is not the source of this hierarchy.

The supplied files provide a controlled pair:

| File | Size | Relevant streams |
|---|---:|---|
| `WORDPAD.GID_OLD` | 25,201 bytes | `|CntText`, `|CntJump`, `|FILES`, `|Flags`, `|KWBTREE`, `|KWMAP`, `|Pete`, `|WinPos` |
| `WORDPAD.GID` | 16,826 bytes | `|FILES`, `|Flags`, `|KWBTREE`, `|KWMAP`, `|Pete` |

Both use the ordinary WinHelp container magic and directory/B+tree machinery. This agrees with HelpDeco's older observation that GID is based on the same file format as Windows help files, and that `|CntJump` / `|CntText` hold CNT jump references and titles respectively. HelpDeco left `|Flags` unresolved; the WordPad pair lets this project decode the Contents-specific tail much more narrowly.

### 11.5.1 `|CntText`

`|CntText` is an `Lz` B+tree. In the supplied GID its leaf records are:

```text
u32 key
STRINGZ text          ; Windows-1252 in this specimen
```

There are 37 records. Keys 1 through 35 reproduce the 35 numbered `WORDPAD.CNT` entries in authored order. Two special keys follow:

- `70000` -> `Ajuda do WordPad` (`:Title`)
- `70001` -> `wordpad.hlp>proc4` (`:Base`)

The viewer treats these special-key meanings as specimen-verified WinHelp 4.x behavior and preserves ordinary keys for row/target association.

### 11.5.2 `|CntJump`

`|CntJump` uses the same observed `u32 key + STRINGZ` leaf form, but contains only the clickable rows. The WordPad GID contains 29 records, exactly matching the 29 CNT lines with `=target` destinations. The six non-clickable book headings have no matching key. This gives a safer clickability rule than trying to infer it from an undocumented flag nibble: a row is clickable when its key is present in `|CntJump`.

### 11.5.3 `|Flags` hierarchy tail

The Contents-bearing WordPad GID has 792 used bytes in `|Flags`; the non-Contents GID has 756. The final 36 bytes of the Contents-bearing stream are one byte `0x0C` followed by exactly 35 node bytes. Their high nibbles reproduce the CNT hierarchy levels at every position:

```text
CNT level 1 book   -> observed 0x10
CNT level 2 topic  -> observed 0x22
```

The project therefore decodes `node_byte >> 4` as the hierarchy level for this record family. The low nibble is **not** claimed as fully decoded because this specimen correlates level and node kind too conveniently: all level-1 rows are books and all level-2 rows are clickable topics. Build-fix 46 deliberately obtains clickability from `|CntJump` instead. For safety, a GID is accepted as a hierarchy source only when the trailing record is exactly `0x0C` plus one nonzero-level byte per ordinary `|CntText` row. Otherwise the viewer refuses to invent a flat hierarchy.

### 11.5.4 `|FILES`

The supplied `|FILES` stream is an `L4z` B+tree whose leaf records are observed as:

```text
u32 key
u32 cached_metadata
STRINGZ text
```

WordPad contains four records: labeled cached HLP paths for `Ajuda do WordPad` and `Tarefas básicas`, an unlabeled cached `Windows.hlp` path, and key `10000` containing the cached `wordpad.CNT` path. Build-fix 46 maps labeled help rows to the same one-hop Index catalog role as CNT `:Index`, unlabeled help rows to the same Link/Search catalog role as CNT `:Link`, and ignores key `10000` as a help destination. Because these are resolved Win9x absolute cache paths, the portable viewer reduces absolute cached HLP paths to their final filename before applying its existing sibling-relative automatic-loading security policy.

### 11.5.5 Viewer source precedence and scope

Build-fix 46 intentionally uses a conservative policy rather than claiming the complete WinHlp32 cache invalidation algorithm: a readable CNT remains authoritative; only when CNT is absent/unreadable does the viewer try a same-basename GID case-insensitively. The GID reader is read-only. It does not create, refresh, timestamp-check, invalidate, or write GID files. A GID that exists but lacks a valid `|CntText`/`|Flags` hierarchy is not treated as supplying Contents.

## 11.6 Keyword tables

**Status: STRONG INFERENCE**

Authored keyword tables use paired internal streams such as `|KWBTREE` and `|KWDATA`; other leading letters use the same family for alternate tables. The B+ tree shares the directory’s fixed 38-byte header/page linkage but has keyword-specific leaf payloads: NUL-terminated keyword, target count, and data-stream offset. Data records are signed `TOPICOFFSET` values. Nonnegative values become topic targets; `-1` macro targets are retained as metadata but are not executed automatically.

The standard `K` table drives Index. The `A` table is used by ALink/associative linking.

# 12. Hosted controls and `!label,macro` BUTTON forms

## 12.1 Compact hosted/custom-window records

**Status: VERIFIED**

Compact types `0x05` / `0x24` enter the WinHlp32 hosted/custom-window renderer. The audited path skips a six-byte prefix, reads a NUL-terminated descriptor, and dispatches the descriptor through the hosted-object factory. For an arbitrary authored native child, `0x4242A8..0x424359` reads `LOGPIXELSX` and `LOGPIXELSY` and creates the child at exactly **`2 * DPI_X` by `2 * DPI_Y`**: a two-device-inch pre-negotiation rectangle.

That creation rectangle is not necessarily the final layout size. The query path at `0x42464E` first sends private message `0x706B` with an output-size pointer. If the child accepts the message, WinHlp32 uses the returned dimensions; otherwise it obtains the actual child rectangle through the `GetWindowRect` fallback helper. The final dimensions of arbitrary hosted controls are therefore **runtime-negotiated behavior of the instantiated child**, not a missing static width/height field in the HLP record.

The safe Rust viewer intentionally does not instantiate document-supplied native code. Its generic hosted-control placeholder now uses the verified two-inch pre-negotiation rectangle, bounded to the available line width, instead of the former invented 180x36 geometry. This is a deliberate security boundary after the format semantics have been resolved, not an unresolved record-layout assumption.

Old compact type `0x06` reaches no renderer call in the traced dispatcher and is represented explicitly as a no-render record.

## 12.2 Leading-`!` `BUTTON` form

**Status: VERIFIED**

The audited hosted-object path distinguishes a leading-`!` descriptor form and dispatches it through a factory. For the classic empty-label button branch, WinHlp32 stores `0x000C000C` and calls `MoveWindow`, confirming a 12x12 control geometry. This finding was used to reproduce CALC.HLP’s Related Topics control without inventing a page-margin override.

The Rust viewer recognizes the retained `!label,macro` BUTTON description as a viewer-local clickable macro hotspot. The button itself invokes the same bounded macro dispatcher as text macro hotspots.

# 13. WinHelp macros and safe execution semantics

## 13.1 On-disk role

**Status: STRONG INFERENCE**

Macro text appears in `|SYSTEM` CONFIG records, topic metadata, text hotspots, graphical hotspots, and hosted button descriptors. A compatibility viewer must parse enough syntax to recognize navigation/UI behavior, but executing arbitrary legacy host actions would be unsafe.

## 13.2 Project execution model

**Status: STRONG INFERENCE / PROJECT POLICY**

The current implementation tokenizes macro programs into a typed AST with hard limits on source text, call count, argument count, nesting depth, string size, runtime command budget, and retained diagnostics. Recognized viewer-local operations can be allow-listed; process/shell execution, DLL invocation, host interaction, malformed calls, unknown names, and unsupported unsafe UI mutation are blocked by default.

Current safe operations include navigation/history and selected WinHelp UI semantics such as `ALink`/`AL` and `BrowseButtons`. The macro layer reuses the ordinary navigation model; it does not bypass HLP destination resolution.

## 13.3 ALink

**Status: STRONG INFERENCE**

`ALink`/`AL` consumes semicolon-delimited associative names and performs exact lookup in the authored `A` keyword table. Matches are resolved through ordinary `TOPICOFFSET` anchors, deduplicated in authored order, and either navigated directly or shown in a multi-topic chooser. No match is non-fatal.

# 14. Corrections to received WinHelp lore

The following table records interpretations that this project once inherited or considered but later disproved or materially narrowed.

| Received interpretation | Corrected finding | Confidence |
|---|---|---|
| `0x20`/`0x21` are VariableField/DType inline character commands | They are modern TOPICLINK record generations; the audited character scanner rejects them | **Verified** |
| `0x8B`/`0x8C` are ordinary character commands | The audited scanner rejects both | **Verified** |
| Modern/MVB fonts use a 42-byte descriptor with extra style/character-map directories | Descriptor stride is universally 11 bytes in the audited paths; face-name slot changes 20 -> 32 bytes | **Verified** |
| Modern charset is a global 16-bit <code>&#124;SYSTEM</code> value | Record 11 is a byte table indexed by font face index | **Verified** |
| Tables form HTML-like rows | Each column has its own vertical cursor; table height is the maximum column height | **Verified** |
| Tables imply a visible cell grid | The table path supplies geometry/flow only; visible rules come from authored paragraph borders | **Verified** |
| Each table cell has a fixed five-byte prelude before ParagraphInfo | Cell = signed column index + complete bounded compact TOPICLINK record | **Verified** |
| Nested tables need a separate algorithm | The generic dispatcher recursively calls the same table renderer with the parent column’s geometry | **Verified** |
| Paragraph metrics depend on font-generation twip/half-point interpretation | Paragraph metrics are converted directly with signed `raw * DPI / 144` arithmetic | **Verified** |
| Paragraph font selection resets at each paragraph | Font selection is running render state and survives paragraph transitions | **Verified** |
| `RGB(1,1,0)` means “near black” or can be normalized with neighboring colors | Only exact `0x00000101` is the inherit-current-color sentinel | **Verified** |
| Paragraph border high bits are independent booleans | They form one three-bit style code | **Verified** |
| The two trailing border bytes are pen width | The traced paragraph-border painter does not establish them as pen width; they remain render-inert metadata | **Verified negative result** |
| External hotspot structure size includes command/size bytes | It measures the following structure itself | **Verified** |
| Context-hash hotspot families contain physical `TOPICOFFSET` | `0xE2/0xE3/0xE6/0xE7` resolve hashes through <code>&#124;CONTEXT</code> | **Verified** |
| Type 0 table scaling can be algebraically collapsed | WinHlp32 performs two integer truncation stages; collapsing them can change geometry | **Verified** |

# 15. Confidence matrix and remaining compatibility boundaries

| Area | Current status | Notes |
|---|---|---|
| Container header / FILEHEADER / directory | **Strong inference** | Stable parser model; bounded and exercised across files |
| <code>&#124;SYSTEM</code> record 9 locale | **Verified** | 10-byte audited branch, final WORD source |
| <code>&#124;SYSTEM</code> record 11 charset table | **Verified** | Per-face byte indexing traced in executable |
| 11-byte fonts, 20/32-byte face slots | **Verified** | Direct descriptor indexing and real-file HCW cross-check |
| Paragraph optional-field order and DPI rules | **Verified** | Direct parser/layout trace |
| Font lifetime | **Verified** | Global state writes and paragraph transition behavior traced |
| Tables / recursion / no implicit grid | **Verified** | Dispatcher and table renderer traced |
| Common graphics decoding | **Strong inference** | Reconstructed format behavior and concrete resources; WMF sizing cross-checked against runtime paths |
| Common text hotspot families | **Verified** | Command scanner and target handling traced |
| Residual `C0..CF/E0..EF` hotspot variants | **Verified inert in KB917607** | Envelope boundaries remain parsed; activation dispatcher has no action branch for residual variants |
| `0x85` downstream semantic effect | **Verified** | Signed WORD resets transient horizontal line origin; glyphless; scanner also uses pending separator state |
| Johab charset `0x82` | **Verified / implemented** | Win32 JOHAB_CHARSET resolves to CP1361; deterministic decoder added |
| Host-dependent OEM charset selection | **Verified / implemented on Windows** | WinHlp32 delegates to host GDI code-page state; Windows viewer uses `CP_OEMCP`, while non-Windows remains an explicit portability fallback |
| Paragraph flag bit 0 visual effect | **Verified render-inert in audited path** | Field is retained, but traced geometry/paint paths do not consume it |
| Paragraph-border trailing two bytes | **Verified render-inert in audited path** | Preserved raw; traced painter does not use them as width/pen data |
| Reserved border styles 5..7 | **Verified reserved/no-paint in audited path** | Zero clearance and no defined style setup in the traced renderer |
| Arbitrary hosted native controls | **Verified runtime-negotiated / intentionally not executed** | Initial 2×DPI rectangle known; final size comes from private message 0x706B or actual child window rectangle |

# 16. WinHlp32 executable audit and address-level findings

The remainder of this part preserves the detailed Microsoft-runtime trace used to correct the renderer. Addresses are virtual addresses in the exact executable identified at the beginning of this manual.

This note records formatting behaviour reverse-engineered directly from the Microsoft
WinHlp32 binary reconstructed from the user-supplied Windows 8.1 KB917607 x64 package.
It is intended to keep the Rust implementation tied to observed behaviour rather than
to assumptions inherited from third-party viewers.

### Reference binary

**Status: VERIFIED**

- File: `winhlp32.exe` (kept external to this source tree)
- PE machine: x86 (`IMAGE_FILE_MACHINE_I386`)
- Length: 285,696 bytes
- SHA-256: `8496f19bc1d898180b57aac61326bdfcf5a48f760811283bbd604aa7a2c13f85`
- Package: Windows 8.1 KB917607 x64

Addresses below are virtual addresses in that exact executable. They are useful only
for this verified binary revision.

### Classic paragraph record

**Status: VERIFIED**

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

#### Metric conversion

**Status: VERIFIED**

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

#### Alignment

**Status: VERIFIED**

At `0x416170..0x416187`, the decoded two-bit value behaves as follows:

- `0`: left;
- `1`: right;
- `2`: center;
- `3`: right.

Only value 2 takes the half-remaining-width centering path. Build-fix 9 incorrectly
interpreted value 3 as centered.

#### No-wrap

**Status: VERIFIED**

Bit 12 is tested in the normal overflow paths around `0x41661f`/`0x4166e7` and in the
tab-overrun path around `0x417793`. When set, the viewer does not create an automatic
new line merely because a word or tab target exceeds the available right edge.

#### Line spacing

**Status: VERIFIED**

The signed line-spacing value is applied around `0x4160c5..0x4160e4`:

- zero: natural measured line extent;
- positive: minimum line advance, `max(natural, authored)`;
- negative: exact line advance, `abs(authored)`.

This is implemented after the reference DPI/144 conversion.

### Tabs

**Status: VERIFIED**

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

### 11-byte `|FONT` descriptor across compiler generations

**Status: VERIFIED**

The font builder is around `0x411884`. A second selection path at `0x411E8C..0x411EBC`
proves that **all** audited compiler generations index descriptor records in 11-byte strides.
The observed layout is:

- byte 0: attribute bits;
- byte 1: size in half-points;
- byte 2: family;
- word 3..4: face-name table index;
- bytes 5..7: foreground RGB;
- bytes 8..10: background RGB.

#### Font height

**Status: VERIFIED**

WinHlp32 feeds the half-point size directly into a `MulDiv(..., LOGPIXELSY, 144)`-style
font-height conversion and negates the resulting `lfHeight`. It therefore preserves
half-point sizes instead of first rounding them to whole typographic points.

#### Attribute bits used by the Microsoft builder

**Status: VERIFIED**

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

#### Family mapping

**Status: VERIFIED**

The classic family values map to Win32 `LOGFONT` family classes as follows:

- 1: modern / fixed-pitch family (`0x30`);
- 2: Roman (`0x10`);
- 3: Swiss (`0x20`);
- 4: script (`0x40`);
- 5: decorative (`0x50`).

The Rust viewer still substitutes modern Windows faces for ordinary historical faces,
while preserving the semantic family and keeping symbol/decorative faces when required.

#### Face-name slots and `|SYSTEM` charset metadata

**Status: VERIFIED**

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

#### Foreground and background colours

**Status: VERIFIED**

The colour resolver around `0x411be6` constructs the foreground and background
`COLORREF`s from the descriptor RGB bytes. For **each** colour, exact `0x00000101`
(`RGB(1,1,0)`) is a sentinel meaning that the currently active/default colour is retained.
It is not a fuzzy near-black test.

WinHlp32 uses opaque GDI text background output in this path. Therefore an explicit
font background colour is semantically meaningful and must not be discarded.

### Font selection lifetime

**Status: VERIFIED**

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

### Paragraph borders

**Status: VERIFIED**

Bit 8 copies a raw three-byte border record. The first byte contains both sides and a
three-bit style code.

#### Side bits

**Status: VERIFIED**

The low five bits are tested as:

- bit 0: whole box / all sides;
- bit 1: top;
- bit 2: left;
- bit 3: bottom;
- bit 4: right.

The helper near `0x415320` effectively tests `box OR requested_side`.

#### Style code

**Status: VERIFIED**

The high three bits are extracted together as `(flags >> 5) & 7` around
`0x415386..0x415392`. They are **not three independent Boolean properties**.

Observed rendering behaviour:

- style 0: normal single border;
- style 1: thick border treatment;
- style 2: double border (second rectangle/edge two pixels inward, `0x415501..0x41552c`);
- style 3: shadow border (offset bottom/right strokes, `0x41553d..0x41556b`);
- style 4: same basic geometry/clearance class as normal in the verified path;
- styles 5..7: no positive clearance returned by the traced spacing helper; retained as reserved.

#### Border-to-content clearance

**Status: VERIFIED**

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


### Table records (`TOPICLINK` type `0x04` / `0x23`)

**Status: VERIFIED**

The table layout path begins around `0x414f66`. The reference implementation establishes
several details that were previously only approximated in the Rust viewer.

#### Header and column records

**Status: VERIFIED**

The table header begins with:

- byte 0: column count; WinHlp32 rejects values above **32**;
- byte 1: table type;
- for **type 0 only**, an additional unsigned 16-bit minimum-width value;
- then one four-byte record per column.

Each column record is two **unsigned** words in this order:

1. authored column **width**;
2. authored **gap before the column**.

The previous Rust decoder had these two words reversed and treated them as signed.

#### Horizontal geometry

**Status: VERIFIED**

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

#### Vertical flow

**Status: VERIFIED**

WinHlp32 does not form HTML-like rows. It maintains one cumulative vertical cursor per
column:

- a cell begins at `table_y + column_height[column]`;
- rendering the cell advances **only that column's** height;
- overall table height is the maximum cumulative column height.

Therefore a tall item in column 1 does not force the next item in column 0 down to a
shared row baseline. Build-fix 10 replaces the previous row-grouping approximation with
this independent-column flow.

#### Nested cell framing

**Status: VERIFIED**

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

#### Recursive table dispatch and returned geometry

**Status: VERIFIED**

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

### Character-command scanner corrections

**Status: VERIFIED**

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
line-origin override, not a variable field. Build-fix 39 applies it as a device-coordinate x-origin
reset. The separate scanner path sets pending flag `0x43C2D8`; `0x41AA18..0x41AA58` inserts one
ASCII space before the next extracted string and clears the flag.

The same scanner accepts the entire masked hotspot structure families. `(opcode & 0xD8) == 0xC0`
advances exactly five bytes total; `(opcode & 0xD8) == 0xC8` advances by opcode + WORD length +
that many payload bytes. The known `0xC8`/`0xCC` macro reader now follows the same rule: that WORD
is the following payload size, not a total size from which three header bytes should be subtracted.
Known hotspot opcodes retain their semantic navigation models. A second trace of activation dispatcher
`0x429C13..0x429E24` closes the residual navigation question: only `0xC8`/`0xCC`, `0xE0..0xE3`,
`0xE6`/`0xE7`, `0xEA`/`0xEB`, and `0xEE`/`0xEF` have click-action branches in this KB917607
runtime (plus unrelated command `0xB0`). The remaining structurally accepted envelope variants fall
through without navigation or macro dispatch. Build-fix 39 labels them KB917607-inert rather than
assigning an unresolved target.

### Paragraph flag bit 0

**Status: VERIFIED**

At `0x4125D9` paragraph flag bit 0 conditionally decodes one compressed signed long. The value is
retained as `unknown_value`, but the verified paragraph geometry, line-layout, border, and paint
paths do not read that field. Build-fix 17 therefore classifies it as retained non-visual metadata
for this renderer rather than inventing a spacing or width meaning.

### Right-to-left paragraphs and charset-run ordering

**Status: VERIFIED**

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

LinkData2 decoding now follows the selected legacy charset before layout. Build-fix 34 covered the
common Windows SBCS/DBCS families; build-fix 39 closes `JOHAB_CHARSET` with CP1361. The Microsoft
selection mechanism is now identified end-to-end. `0x411E6F` combines the 11-byte font descriptor,
per-face record-11 byte when present, and GDI face enumeration (`0x4334FA`). With no record-11 table,
Symbol/WingDings select charset 2 and otherwise the host text charset at `0x43B09C` is used. At
`0x416EB6`, `GetTextCharset` plus `TranslateCharsetInfo` yields the code page before
`MultiByteToWideChar`. Therefore OEM charset `0xFF` is intentionally a host GDI/OEM database
decision rather than an unknown HLP encoding.

### Residual-gap audit: resolved semantics and remaining policy boundaries

**Status: VERIFIED**

A build-fix 39 re-audit of the exact 285,696-byte KB917607 executable closes the five items that
previously appeared under “Still unresolved”. They are now classified as follows:

1. **Johab:** charset `0x82` is CP1361 and is implemented. **OEM:** charset `0xFF` is verified as
   host-selected through GDI/code-page APIs rather than missing on-disk semantics. Windows builds
   now mirror the active host with `CP_OEMCP`; non-Windows builds retain the portability fallback.
2. **Shaping:** WinHlp32 has no private shaping engine in this path. It converts with
   `MultiByteToWideChar`, paints normally with `TextOutW`, and falls back to `TextOutA` only on
   conversion failure (`0x416F54..0x41706D`). The Windows viewer now preserves authored faces for
   non-ANSI legacy charsets so its GDI face/charset pair follows the reference.
3. **`0x85`:** verified as a glyphless signed horizontal line-origin override. It writes transient
   state `+0x38`, returns tokenizer status 2, affects line-alignment state, and acts as a pending
   separator in the scanner/text-extraction path.
4. **Residual hotspot opcodes:** structurally accepted but action-inert in the verified click
   dispatcher. They are no longer assigned an unknown navigation meaning.
5. **Hosted-control dimensions:** final size is runtime-negotiated. Arbitrary controls are created
   at `2 * LOGPIXELSX` by `2 * LOGPIXELSY`; query helper `0x42464E` first sends private message
   `0x706B`, then falls back to actual `GetWindowRect`. The safe viewer uses the verified two-inch
   pre-negotiation rectangle but intentionally never instantiates document-supplied native code.

The remaining differences are therefore mainly **platform/security policy boundaries**, not unknown
HLP record semantics: host OEM databases can differ; non-Windows toolkits need not shape exactly like
legacy GDI; and safe mode deliberately refuses to run authored native child controls.

### Table visibility: geometry is not a grid

**Status: VERIFIED**

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

### Build-fix 15 retained-layout corrections

**Status: VERIFIED**

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
geometry/flow and visible rules come from paragraph borders. It also does not alter the retained topic-layout semantics. Contents source policy was later refined: build-fix 46 keeps a discovered `.CNT` authoritative, falls back to a verified same-basename `.GID` hierarchy when CNT is unavailable, and exposes physical topic order only through the explicit **Show all** mode.

### Build-fix 16 integrated-viewer presentation policy

**Status: STRONG INFERENCE**

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


### Build-fix 18 hover restoration under the single-surface policy

**Status: STRONG INFERENCE**

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

### Build-fix 20 popup hover content

**Status: STRONG INFERENCE**

Build-fix 18 restored the old hover plumbing but restored the wrong payload for popup-marked hotspots: it displayed a resolved destination label such as `Popup: Topic 6`. That is not useful for untitled glossary/context popup topics, whose meaningful authored content is their body text.

Build-fix 20 keeps ordinary jump-link hover behavior unchanged, but popup-marked internal, context-hash, and cross-file hotspots now resolve the destination `TopicPresentation` and flatten its visible text into the native tooltip. Paragraph boundaries, explicit line breaks, and tabs are preserved; pictures, retained controls, and other non-text objects contribute no invented caption. Textless popup topics fall back to the resolved title.

This is a hover-content correction only. Build-fix 16's single-surface activation policy remains in force: clicking a popup-marked hotspot still navigates the main help surface rather than constructing a detached popup frame.

### Build-fix 21: inline compact records and CALC Related Topics control

**Status: VERIFIED**

The Calculator topic that visually precedes `Tópicos relacionados` contains an inline-object command from the `0x86`/`0x87`/`0x88` family. The previous Rust decoder treated those character commands as synonymous with pictures. The retained 285,696-byte KB917607 WinHlp32 reference disproves that assumption: the command points at a compact TOPICLINK record, and the nested record's own type selects the generic renderer. The helper reached at `0x412884` parses that compact header; graphics are the `0x03`/`0x22` families, while `0x05`/`0x24` are hosted/custom-window families.

For the affected CALC.HLP object the nested type is `0x05`. Its six-byte hosted prefix is followed by the NUL-terminated descriptor:

```text
!,AL("A_CALC_LIST_EQUIV;A_CALC_KEYB_SEQ")
```

WinHlp32 hosted-control renderer `0x419281` calls the compact-header helper, advances six bytes into the payload, copies/splits the descriptor, and calls factory `0x4240F4`. At `0x424143` the factory detects the leading byte `0x21` (`!`). In that mode it splits the descriptor at the comma. The bytes after `!` and before the comma are the window/button label; the remainder is the authored macro. CALC therefore has an empty label and macro `AL("A_CALC_LIST_EQUIV;A_CALC_KEYB_SEQ")`.

The standard-button branch computes initial height as `12 + (label_nonempty ? 4 : 0)` around `0x4244CD..0x4244DF`, pushes initial width `0x1E` (30) at `0x4244E0`, passes class string `BUTTON` from `0x4020A0`, and calls imported `CreateWindowExA` at `0x42451F`. That is not the final empty-button size: when the label is empty, `0x424545..0x42455B` selects packed dimensions `0x000C000C`, and the following `MoveWindow` call at `0x424593` resizes the child to **12x12**. Non-empty labels instead pass through text measurement helper `0x42489D` before `MoveWindow`.

Thus this CALC object is **not an omitted bitmap**. Native WinHlp32 constructs a tiny blank 12x12 standard button at that location and subclasses it for the stored macro action. The viewer reproduces the verified visual geometry but deliberately leaves that macro inert under the existing default-deny hosted-code policy.

Build-fix 21 therefore changes the inline decoder from `0x86..0x88 => Picture` to nested compact-record dispatch. Real `0x03`/`0x22` records continue through the image pipeline, `0x05`/`0x24` become retained hosted controls, old `0x06` is consumed as no-render, and other structurally bounded compact records are skipped with layout-safe diagnostics rather than corrupting following LinkData1 parsing.

### Build-fix 24 empty table-cell display semantics

**Status: VERIFIED**

The retained table walker remains column-independent, matching the Microsoft routine at `0x414F66`: after dispatching one bounded cell through `0x417578`, WinHlp32 adds the returned child height only to that cell's signed-column cursor at `0x4151DC..0x4151E9`. The Calculator keyboard-equivalents table therefore must not be normalized into a conventional shared-row grid.

The alignment defect was instead in the child display height. CALC.HLP interleaves completely empty compact display records among visible cells. In the verified KB917607 display renderer, `0x415B44..0x415B57` first tests for an empty LinkData2 string and an immediate `0xFF` character-stream terminator. When both are present, `0x415B5C..0x415BA1` consumes the terminator/string and returns through the fast path before the ordinary paragraph-spacing path beginning at `0x415BA6`; in particular, the `spacing_above` addition at `0x415BAE..0x415BB2` is bypassed. Such a display therefore contributes no synthetic text-line advance.

Build-fix 24 mirrors that behavior specifically in retained table-cell display layout. An unbordered paragraph with no layout-bearing inline command (zero-width `0x85` markers are allowed) is skipped. Borders, explicit tabs and line breaks, pictures, and hosted controls remain layout-bearing. This makes CALC's four visible logical column groups share the intended baseline while preserving the reference's independent column cursors and recursive-table model.

As a viewer presentation refinement, a rule-only top-level display record immediately followed by a table reserves an additional 8 pixels before the first table cell. The gap is attached to that display-to-table transition only; it is not a per-table-record margin and therefore cannot accumulate between the keyboard-equivalents data records.



### Build-fix 25 mixed-font line baseline correction

**Status: VERIFIED**

The retained layout previously assigned every text run on a visual line the same top y-coordinate. That is only correct when all runs have the same measured height. CALC.HLP's `Dicas` list marker is a smaller square-glyph run beside the ordinary body font, so top alignment places the marker visibly above the text.

Build-fix 25 performs vertical text-run alignment during line finalization, after ordinary horizontal alignment and any Hebrew/Arabic charset-run reordering. The largest measured text height on the line establishes the retained baseline band; shorter text boxes are shifted downward by the height difference so their bottoms coincide. Non-text inline boxes are excluded. This preserves the existing native-width wrapping and line advance while correcting small symbol/font runs such as the Calculator bullets.


### Build-fix 28 native baseline retention

**Status: VERIFIED**

Build-fix 25-27 attempted to align the small CALC.HLP list marker from retained text-box height. That is insufficient: the native measurement API can return equal cell heights for different faces while their descents differ, so a height-only rule may make no change at all.

The retained metric now carries a baseline offset explicitly. In the wxWidgets viewer the baseline is derived from `GetFullTextExtent` data as the measured text height minus descent (external leading remains part of line height, not ascent). Each `LayoutKind::Text` retains that offset, and line finalization shifts text runs so `bounds.y + baseline` is common across the line. The headless fallback synthesizes a conservative baseline only when native ascent/descent is unavailable.

This changes only within-line text placement. Line advance, paragraph spacing, hanging indentation, table flow, pictures, hosted controls, and hotspot bounds continue through the existing paths.

### Build-fix 29: CALC inline list-marker baseline

**Status: VERIFIED**

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


### Build-fix 31: Related Topics alignment uses rendered border geometry

**Status: VERIFIED**

Build-fix 30 incorrectly inferred horizontal placement by replacing the stock ALink paragraph's authored left and first-line indents with zero. The verified KB917607 hosted-button trace does not justify that metadata rewrite: dispatcher `0x419281` reaches factory `0x4240F4`, and the empty-label path sets a packed 12x12 size before the `MoveWindow` call at `0x424593`. That establishes control dimensions, not a universal x-coordinate.

The retained renderer now completes normal paragraph layout first. A rule-only top-level record records the x-coordinate of its emitted `Border`; if the immediately following record contains the stock ALink button, the 12x12 button and text boxes sharing its visual line are shifted by `rule_left - button_left`. Authored paragraph indents therefore continue to influence normal layout/wrapping, while the final Related Topics row follows the actual separator edge rather than assuming `PAGE_MARGIN`.


### Build-fix 32: native per-axis DPI and retained half-point font sizes

**Status: VERIFIED**

The retained parser has preserved HLP font sizes in twentieths of a point since the font-model correction, but the wxWidgets front end still created fonts through an integer-point constructor. That final conversion rounded an authored half-point size before native measurement and therefore could change glyph width, baseline, wrapping, paragraph height, and hotspot placement even though the parsed descriptor was correct.

Build-fix 32 keeps the retained twip value through viewer zoom. On Windows the topic canvas obtains a GDI device context from its native HWND, reads `LOGPIXELSX` and `LOGPIXELSY`, and creates a `LOGFONTW` whose negative `lfHeight` is derived directly from the zoomed twip value and vertical device DPI. The same selected GDI font is used by `GetTextExtentPoint32W` / `GetTextMetricsW` for retained measurement and by `TextOutW` for painting. This keeps the layout and paint paths synchronized instead of measuring one rounded font and drawing another. The wxDragon integer-point font path remains a portable fallback.

The layout engine now retains DPI per axis. LinkData1 x geometry (left/right/first-line indents and tab stops), table widths/gaps, and headless fallback text width use horizontal DPI. Paragraph spacing, line spacing, and headless fallback font height use vertical DPI. The old `LayoutEngine::new(dpi)` constructor remains a square-DPI convenience wrapper; native front ends use `LayoutEngine::with_dpi(dpi_x, dpi_y)`.

Build-fix 33 subsequently extends the same per-axis device-DPI context to picture natural sizing, so the 96-DPI/raw-pixel assumption described here no longer applies to retained display geometry.


### Build-fix 33: bitmap and WMF physical-resolution sizing

**Status: VERIFIED**

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


### Build-fix 34: legacy charset decoding and CJK wrapping

**Status: VERIFIED**

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
GDI/code-page choice; Windows builds use active `CP_OEMCP`, while non-Windows builds retain the
documented fallback rather than pretending one OEM page is universal.

Retained wrapping also recognizes CJK text that has no ASCII spaces. Ideographic, kana and hangul
characters become legal break units while Latin words remain grouped; simple opening punctuation
stays with the following unit and closing punctuation with the preceding unit. This is deliberately
a conservative legacy line-break model rather than a new shaping engine: final glyph shaping and
font fallback remain the responsibility of the native painter.


### Build-fix 35: zoomed line advance and restored-window reflow

**Status: VERIFIED**

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

# 17. Quick-reference tables

## 17.1 Core magics and fixed sizes

| Item | Value | Confidence |
|---|---:|---|
| Outer HLP magic | `0x00035F3F` | Strong inference |
| Outer HLP header | 16 bytes | Strong inference |
| Internal `FILEHEADER` | 9 bytes | Strong inference |
| Directory/navigation B+ tree magic | `0x293B` | Strong inference |
| Common B+ tree header | 38 bytes | Strong inference |
| <code>&#124;SYSTEM</code> magic | `0x036C` | Strong inference |
| Physical <code>&#124;TOPIC</code> block header | 12 bytes | Strong inference |
| Full TOPICLINK header | 21 bytes | Strong inference |
| <code>&#124;FONT</code> prefix | 8 bytes | Verified |
| Font descriptor | 11 bytes | Verified |
| Legacy face slot | 20 bytes | Verified |
| HCW 4.0 face slot | 32 bytes | Verified |
| Ordinary modern WINDOW record | 90 bytes | Strong inference |
| Maximum table columns in audited renderer | 32 | Verified |

## 17.2 TOPICLINK dispatcher

| Record | Renderer/meaning | Confidence |
|---:|---|---|
| `0x01` / `0x20` | Display | Verified |
| `0x02` / `0x21` | Topic header | Verified structurally |
| `0x03` / `0x22` | Graphics | Verified |
| `0x04` / `0x23` | Table | Verified |
| `0x05` / `0x24` | Hosted/custom window; runtime-negotiated dimensions | Verified |
| `0x06` | No renderer call in audited dispatcher | Verified |

## 17.3 Paragraph flag word

| Bit(s) | Meaning | Confidence |
|---:|---|---|
| 0 | Compressed signed long, semantic purpose unresolved | Verified structure / Unresolved semantics |
| 1 | Spacing before | Verified |
| 2 | Spacing after | Verified |
| 3 | Line spacing | Verified |
| 4 | Left indent | Verified |
| 5 | Right indent | Verified |
| 6 | First-line indent | Verified |
| 7 | Default tab interval; absent -> 72 source units | Verified |
| 8 | Three-byte border record | Verified |
| 9 | Custom tab-stop array | Verified |
| 10..11 | Alignment | Verified |
| 12 | No-wrap | Verified |
| 13 | RTL paragraph/layout | Verified |
| 14..15 | No formatting control established | Unresolved |

## 17.4 Font attribute bits

| Bit | Effect in audited classic builder | Confidence |
|---:|---|---|
| `0x01` | Bold / weight 700 | Verified |
| `0x02` | Italic | Verified |
| `0x04` | Underline | Verified |
| `0x08` | Strikeout | Verified |
| `0x10` | Not consumed by classic builder | Verified negative result |
| `0x20` | Small-caps/reduced height: 2/3 normal | Verified |

## 17.5 Common GDI charset bytes handled by project

| Charset | Value | Decoder/family |
|---|---:|---|
| ANSI | `0x00` | Windows-1252 |
| DEFAULT | `0x01` | Deterministic face/LANGID inference when possible |
| SHIFTJIS | `0x80` | Shift-JIS |
| HANGEUL | `0x81` | Korean |
| JOHAB | `0x82` | Windows CP1361 / deterministic Johab decoder |
| GB2312 | `0x86` | GBK-compatible path |
| CHINESEBIG5 | `0x88` | Big5 |
| GREEK | `0xA1` | Windows-1253 |
| TURKISH | `0xA2` | Windows-1254 |
| VIETNAMESE | `0xA3` | Windows-1258 |
| HEBREW | `0xB1` | Windows-1255 |
| ARABIC | `0xB2` | Windows-1256 |
| BALTIC | `0xBA` | Windows-1257 |
| RUSSIAN | `0xCC` | Windows-1251 |
| THAI | `0xDE` | Windows-874 |
| EASTEUROPE | `0xEE` | Windows-1250 |
| OEM | `0xFF` | Host-selected GDI/OEM code page; Windows uses active `CP_OEMCP` |

# 18. Executable-address appendix

All addresses below refer only to the verified 285,696-byte KB917607 `winhlp32.exe` image.

| Address / range | Finding |
|---|---|
| `0x4062DF` | Graphics source selector: zero -> signed WORD <code>&#124;bmN</code>; nonzero -> embedded graphics stream |
| `0x411884` | Classic font builder |
| `0x411E6F` | Font index / charset inference helper used by selection path |
| `0x411E96..0x411ED4` | 11-byte descriptor + per-face charset selection; record-11 indexing |
| `0x411E8C..0x411EBC` | Universal 11-byte descriptor indexing; generation-dependent face slots |
| `0x411EA7..0x411ED4` | Per-face charset byte indexed from record-11 table |
| `0x411BE6` | Foreground/background color construction and exact sentinel behavior |
| `0x41200D` | Graphics renderer selected by compact dispatcher |
| `0x4124C2` | Custom-tab lookup routine |
| `0x412525` | Deferred right/center tab resolution |
| `0x4125DB` | Classic paragraph decoder |
| `0x412884` | Compact-header helper for `0x01..0x06` / `0x20..0x24` |
| `0x4129E8` | Compressed signed-long helper |
| `0x414F66` | Table renderer / recursive cell walker |
| `0x4151B6` | Table cell calls generic compact dispatcher |
| `0x4151DC..0x4151E9` | Child height added only to containing column cursor |
| `0x415320` | Border side/style clearance helper |
| `0x415386..0x415392` | Extract three-bit border style code |
| `0x415501..0x41552C` | Double-border second edge |
| `0x41553D..0x41556B` | Shadow-border bottom/right strokes |
| `0x415929` | Ordinary display renderer selected by compact dispatcher |
| `0x415BAE..0x415BB2` | Signed spacing-before application |
| `0x415F18` | Signed spacing-after application |
| `0x4160C5..0x4160E4` | Signed line-spacing behavior |
| `0x415CE4..0x415D35`, `0x415FB5`, `0x416169..0x4161B1` | Horizontal line-origin state later overridden by character command `0x85` |
| `0x416EB6..0x41706D` | GDI charset -> code page -> `MultiByteToWideChar`; normal `TextOutW`, fallback `TextOutA` |
| `0x416170..0x416187` | Paragraph alignment dispatch |
| `0x41661F`, `0x4166E7` | No-wrap overflow paths |
| `0x416A73..0x416B7A` | Inline-object baseline/line metric path used to fix marker alignment |
| `0x417578` | Generic compact visual dispatcher |
| `0x4175E3..0x4175F1` | Table branch recursively calls `0x414F66` |
| `0x417793` | No-wrap tab-overrun path |
| `0x417816..0x417827` | `0x85` sign-extended origin override, token kind `0x36`, tokenizer status 2 |
| `0x419281` | Hosted/custom-window renderer |
| `0x4242A8..0x424359` | Arbitrary hosted child initial size = `2 * LOGPIXELSX/Y` |
| `0x42464E` | Hosted-control size query: private message `0x706B`, then `GetWindowRect` fallback |
| `0x41A9B5` | Font realize callback path |
| `0x41AA18..0x41AA58` | Pending separator insertion used by `0x85` in text extraction |
| `0x41AAC1..0x41ACBA` | Character-command scanner; rejects false `0x20/0x21/0x8B/0x8C` commands |
| `0x41AB7A` | `0x81/0x82/0x83` progress path without font reset |
| `0x41AB8C` | `0x80` font-selection command writer |
| `0x41ABEB` | `0xFF` transition without font reset |
| `0x41B024` | Topic render entry |
| `0x41B05D` | One-time render-state font initialization |
| `0x4240F4` | Hosted-object factory used by leading-`!` form |
| `0x424593` | Empty-label button `MoveWindow`; 12x12 geometry |
| `0x429C13..0x429E24` | Hotspot click dispatcher; residual envelope opcodes fall through inertly |
| `0x4334FA` | GDI font-family/charset enumeration and face-resolution helper |
| `0x42CE5B` | <code>&#124;SYSTEM</code> record 9 exact 10-byte acceptance |
| `0x42CE6C` | <code>&#124;SYSTEM</code> record 11 allocation/copy path |
| `0x43B09C` | Per-file default / `0xFFFF` font sentinel in image |
| `0x43C2C4` | Global selected-font state |
| `0x43C2D0`, `0x43C2D8` | Adjacent topic render state fields |
| `0x43E098` | Font realization callback pointer |

# Closing implementation rule

**Status: VERIFIED as project practice**

When an HLP interpretation is uncertain, preserve synchronization and bounds first. A viewer can render a safe placeholder for a structurally known object and improve it later; it cannot recover reliably after consuming the wrong number of bytes from `LinkData1`, a nested table record, a hotspot envelope, or a graphics payload. The corrections documented here consistently came from applying that rule and then auditing the original Microsoft path when secondary descriptions disagreed.
