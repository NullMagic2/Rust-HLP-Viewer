[package]
name = "hlp"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
encoding_rs = "=0.8.35"
wmf-render = { path = "../wmf" }

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.61", features = ["Win32_Globalization"] }

[lints]
workspace = true
