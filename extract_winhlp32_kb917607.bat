Windows 8.1 KB917607 x64 WinHlp32 reference
===========================================

Observed in the supplied Windows8.1-KB917607-x64 MSU payload manifest and verified
against the user-extracted executable.

Target: amd64_microsoft-windows-winhstb_31bf3856ad364e35_6.3.9600.16398_none_19bd9f9fdd625992\winhlp32.exe
Target length: 285696 bytes
Target SHA-256: 8496f19bc1d898180b57aac61326bdfcf5a48f760811283bbd604aa7a2c13f85
Delta type in KB payload: PA30
Delta blob: 42
Basis: none (ApplyDeltaB source buffer is empty)

Reference-rendering findings used by build-fix 9
-------------------------------------------------
* WinHlp32 initializes ordinary text/background from the current window colours.
* For both foreground and background in an old 11-byte |FONT descriptor, exact
  COLORREF 0x00000101 (RGB 1,1,0) means "keep/inherit current colour".
  It is not a near-black range.
* Other authored colours, including nearby dark values and purple, are not
  normalized by this rule.
* WinHlp32 uses opaque GDI text background mode and applies descriptor
  foreground/background colours to text output.
* HC30 old-font sizes are retained in half-point units through device-size
  conversion instead of being rounded to whole points first.

The source archive does not redistribute Microsoft's winhlp32.exe. Use the bundled
extract_winhlp32_kb917607.bat against your own KB917607 x64 MSU to reconstruct and
verify the reference locally. The verified executable remains external to this project.

Additional formatting findings used by build-fix 10
----------------------------------------------------
* Paragraph parser near 0x4125db: flag bit 7 is an authored default-tab interval;
  absent values default to 72 source units.
* Paragraph metrics are converted directly as raw * device DPI / 144 with integer
  truncation; they do not depend on old/new |FONT| metric generation.
* Alignment values are 0=left, 1=right, 2=center, 3=right.
* Paragraph bit 12 suppresses automatic word/tab wrapping. Bit 13 is the RTL paragraph
  flag: it applies first-line indent from the right and gates Hebrew/Arabic reordering.
* Signed paragraph spacing/indents remain signed in layout; negative values are not
  clamped to zero.
* Signed line spacing is minimum/at-least when positive and exact when negative.
* Right and center tabs are deferred until the following segment has been measured.
* Classic font attribute 0x20 changes font height to two thirds. The classic old-font
  builder does not consume attribute 0x10, so build-fix 10 does not synthesize a
  double underline for that path.
* Border byte high bits are one style code: 0 normal, 1 thick, 2 double, 3 shadow,
  4 normal-equivalent geometry in the traced path; 5-7 remain reserved.
* Border layout reserves 5 px for styles 0/4, 6 px for styles 1/3, and 7 px for style 2
  on each active side. The final two bytes are retained raw; the verified painter does
  not establish them as a pen width.
* Paragraph payloads begin with a two/four-byte compressed signed long before the
  paragraph id/flag DWORD; fixed two-byte skipping is not compatible with long-form values.
* Signed-long decoder 0x4129e8 uses (u16 >> 1) - 0x4000 for short form and
  (u32 >> 1) - 0x40000000 for extended form; the larger bias is essential.
* Table path near 0x414f66: at most 32 columns; type 0 alone carries minimum width;
  each column is unsigned width then gap-before. Type 0 scales through a 32767-unit
  reference span after DPI/144 conversion; nonzero types use absolute DPI/144 metrics.
* Table cells advance independent per-column vertical cursors rather than shared row heights.

Additional table findings used by build-fix 12
-----------------------------------------------
* Dispatcher 0x417578 sends compact record types 0x01/0x20 to the ordinary display
  renderer and 0x04/0x23 to the table renderer, proving both Windows 3.0 and 3.1+
  table generations share the same retained table path.
* Table walker 0x414f66 reads each cell as a signed 16-bit column index followed by
  a complete compact nested TOPICLINK record. Column -1 terminates the cell list.
* Compact-header helper 0x412884 accepts types 0x01..0x06 and 0x20..0x24. Ordinary
  records use a compressed signed payload size; modern records additionally carry a
  compressed unsigned TopicLength. Types 0x02/0x21 select a fixed DWORD-size header,
  with an additional WORD TopicLength for 0x21.
* The table walker advances to the next cell by the exact compact header length plus
  the decoded payload size. There is no fixed five-byte cell prelude before ParagraphInfo.
* Build-fix 12 implements this bounded framing for 0x04/0x23 tables and decodes old
  0x01 and modern 0x20 display cells. Recursive nested tables and uncommon non-display
  compact renderer families remain bounded diagnostics instead of guessed layouts.

Residual-gap findings used by build-fix 39
-------------------------------------------
* Charset/font selection at 0x411e6f continues to use the 11-byte descriptor and the
  per-face record-11 charset byte. The Unicode drawing path at 0x416eb6 obtains the
  selected GDI charset, resolves it with TranslateCharsetInfo, and 0x416f54 converts
  source bytes with MultiByteToWideChar before normally drawing through TextOutW.
  JOHAB_CHARSET 0x82 therefore maps to Windows CP1361. OEM_CHARSET 0xff remains a
  host GDI/code-page choice rather than one fixed HLP encoding.
* Character command 0x85 is tokenized at 0x417816: its following signed WORD is
  sign-extended into render-state +0x38, no glyph is emitted, and tokenizer status 2
  is returned. Paragraph setup seeds +0x38 at 0x415ce4 and line finalization consumes
  it from 0x415fb5 through the remaining-space/alignment path at 0x416169..0x4161b1.
  It is therefore a transient horizontal line-origin override.
* Hotspot activation at 0x429c13..0x429e24 dispatches only the known macro/internal/
  external families (C8/CC, E0..E3, E6/E7, EA/EB, EE/EF, plus unrelated B0). Other
  structurally accepted C0/C8-envelope values fall through without a click action in
  this KB917607 runtime and are classified as inert rather than semantically unknown.
* Arbitrary authored controls created by 0x4240f4 use a 2*LOGPIXELSX by 2*LOGPIXELSY
  initial rectangle at 0x4242a8..0x424359. Size query 0x42464e first sends private
  message 0x706b; when the child does not return a negotiated size, WinHlp32 falls
  back to GetWindowRect via 0x41ccf3. The final size is therefore runtime control
  behavior, not a hidden static HLP dimension field.
* The safe Rust viewer intentionally does not instantiate document-supplied native
  controls. Build-fix 39 mirrors the verified initial two-device-inch rectangle for
  its placeholder while preserving that default-deny security policy.

See docs/WINHLP32_FORMATTING_REFERENCE.md and
MICROSOFT_WINHELP_INTERNAL_FORMAT_REFERENCE.md for the complete address-level audit,
confidence labels, and remaining portability/security boundaries.
