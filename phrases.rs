//! Error types for bounds-checked Windows HLP parsing.

use std::fmt;
use std::io;

/// Errors returned while loading or decoding a Windows HLP file.
#[derive(Debug)]
pub enum HlpError {
    /// An operating-system I/O operation failed.
    Io(io::Error),
    /// The parser needed bytes beyond the current structure boundary.
    UnexpectedEof { context: &'static str },
    /// A structure did not contain its required magic value.
    InvalidMagic {
        context: &'static str,
        expected: u32,
        actual: u32,
    },
    /// A field was internally inconsistent or outside the backing file.
    InvalidField {
        context: &'static str,
        detail: String,
    },
    /// A requested internal HLP stream does not exist.
    MissingInternalFile(String),
    /// The file uses a known feature that this milestone intentionally cannot decode.
    Unsupported { context: &'static str, detail: String },
}

impl HlpError {
    /// Creates an invalid-field error without exposing formatting boilerplate to parsers.
    pub(crate) fn invalid(context: &'static str, detail: impl Into<String>) -> Self {
        Self::InvalidField {
            context,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for HlpError {
    /// Formats an HLP parsing error for diagnostics and the GUI.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::UnexpectedEof { context } => {
                write!(formatter, "unexpected end of data while reading {context}")
            }
            Self::InvalidMagic {
                context,
                expected,
                actual,
            } => write!(
                formatter,
                "invalid {context} magic: expected 0x{expected:08X}, got 0x{actual:08X}"
            ),
            Self::InvalidField { context, detail } => {
                write!(formatter, "invalid {context}: {detail}")
            }
            Self::MissingInternalFile(name) => {
                write!(formatter, "HLP internal file '{name}' was not found")
            }
            Self::Unsupported { context, detail } => {
                write!(formatter, "unsupported {context}: {detail}")
            }
        }
    }
}

impl std::error::Error for HlpError {
    /// Returns the underlying I/O error when one exists.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for HlpError {
    /// Converts standard I/O errors into the project error type.
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
