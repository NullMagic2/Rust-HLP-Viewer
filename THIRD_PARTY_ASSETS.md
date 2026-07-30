# Security notes

Classic `.HLP` files can contain malformed offsets, compressed data, hyperlinks, macros, and references to external resources. Treat every help file as untrusted input.

The `hlp` engine crate uses safe Rust only and validates structural bounds before exposing internal data. Version 0.5.1 adds a narrow `wmf-render` platform crate for legacy Windows-metafile playback: input/decompressed sizes and output dimensions are bounded before entry, GDI handles are checked and released on every return path, and only the resulting RGBA buffer crosses back into `hlp`. Build-fix 32 adds a second narrowly scoped Windows GDI boundary in the viewer for text measurement/painting and device-DPI queries; it receives already-decoded bounded Rust strings/styles, creates only GDI font/DC objects owned by the viewer, restores selected GDI state, and releases every acquired DC/font. The separate application-shell `AttachConsole(ATTACH_PARENT_PROCESS)` FFI remains limited to diagnostic/help/version console attachment.

External HLP links are interpreted as document navigation. Relative filenames are resolved against the directory of the currently loaded HLP; they are never passed to a shell. Failure to open a linked file is reported as a viewer error.

Version 0.7.1 parses startup CONFIG macros, per-topic macros, and hotspot macros before dispatch. Only an explicit allow-list of viewer-local navigation/UI commands can execute. Arbitrary process execution, shell commands, Control Panel/shortcut launching, DLL routine registration, host-state operations, unknown macro names, malformed programs, and recognized-but-unsupported legacy UI mutations are blocked and recorded in a diagnostic log capped by both entry count and entry length. A shared command budget also stops cyclic macro navigation. `SetPopupColor` is implemented only as a viewer-owned paint-state change. Build-fix 30 adds `ALink`/`AL` to the safe set solely as an in-memory lookup of the already parsed HLP A-keyword table followed by normal viewer navigation; it performs no host I/O or code execution. Built-in `!label,macro` button placeholders may now expose their macro as a clickable hotspot, but the command still passes through this same allow-list/default-deny dispatcher.

Legacy WMF bytes are handed to Windows GDI only after the surrounding HLP graphics record has passed size/range/decompression limits. Invalid conversion or playback returns a non-fatal picture warning/placeholder rather than executing a fallback program.

## Sidecar catalog loading

Automatic `.CNT` catalog expansion is limited to relative `:Index`/`:Link` references and at most 32 unique HLP files. Absolute/UNC catalog paths are not opened during document load; explicit user-activated cross-file links retain their normal behavior.
