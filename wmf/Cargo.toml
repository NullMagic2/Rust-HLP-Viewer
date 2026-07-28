[package]
name = "wmf-render"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.61", features = [
    "Win32_Foundation",
    "Win32_Graphics_Gdi",
    "Win32_System_DataExchange",
] }
