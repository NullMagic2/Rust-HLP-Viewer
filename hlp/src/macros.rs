//! Parser and safety classification for the classic WinHelp macro language.
//!
//! WinHelp macros are executable content in the historical viewer.  This module deliberately
//! stops at a typed, GUI-independent command model: a small viewer-local subset plus a narrowly
//! validated HTTP(S) browser action is marked safe, while general process execution, dynamic DLL
//! registration, system interaction, unsupported legacy UI mutation, and unknown operations remain
//! explicit blocked values.

use std::fmt;

const MAX_MACRO_TEXT: usize = 64 * 1024;
const MAX_MACRO_CALLS: usize = 256;
const MAX_MACRO_ARGUMENTS: usize = 16;
const MAX_MACRO_DEPTH: usize = 16;
const MAX_MACRO_STRING: usize = 32 * 1024;

/// One parsed WinHelp macro program. Multiple calls are separated by semicolons in HLP data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpMacroProgram {
    pub macros: Vec<HelpMacro>,
}

impl HelpMacroProgram {
    /// Parses a complete macro string and classifies every syntactically valid invocation.
    pub fn parse(text: &str) -> Result<Self, MacroParseError> {
        if text.len() > MAX_MACRO_TEXT {
            return Err(MacroParseError::new(
                0,
                format!("macro text exceeds the {MAX_MACRO_TEXT}-byte safety limit"),
            ));
        }
        let mut parser = Parser::new(text);
        let mut macros = Vec::new();
        parser.skip_space();
        while !parser.is_eof() {
            if macros.len() >= MAX_MACRO_CALLS {
                return Err(MacroParseError::new(
                    parser.position(),
                    format!("macro program exceeds the {MAX_MACRO_CALLS}-call safety limit"),
                ));
            }
            let invocation = parser.parse_invocation(0)?;
            macros.push(classify(invocation));
            parser.skip_space();
            if parser.is_eof() {
                break;
            }
            parser.expect_byte(b';', "expected ';' between WinHelp macro calls")?;
            parser.skip_space();
            // WinHelp accepts one trailing semicolon.
            if parser.is_eof() {
                break;
            }
        }
        Ok(Self { macros })
    }
}

/// A fully parsed macro, either allow-listed for viewer-local execution or explicitly blocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelpMacro {
    Allowed(SafeHelpMacro),
    Blocked(BlockedHelpMacro),
}

/// Allow-listed WinHelp operations that cannot execute arbitrary host code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafeHelpMacro {
    ALink { keywords: String },
    About,
    Back,
    BackFlush,
    BookmarkDefine,
    BookmarkMore,
    BrowseButtons,
    Contents,
    Finder,
    FocusWindow { window: String },
    History,
    /// Opens a validated HTTP(S) URL in the user's default browser. This is the only host-launch
    /// exception to the default-deny WinHelp macro policy and is produced only from a constrained
    /// `ExecFile` form with no command-line arguments.
    OpenUrl { url: String },
    JumpContents { help_file: String, window: String },
    JumpContext { help_file: String, window: String, context: i32 },
    JumpHash { help_file: String, window: String, hash: i32 },
    JumpId { path_window: String, topic_id: String },
    Next,
    Prev,
    PopupContext { help_file: String, context: i32 },
    PopupHash { help_file: String, hash: i32 },
    PopupId { help_file: String, topic_id: String },
    Search,
    SetPopupColor { red: u8, green: u8, blue: u8 },
}

/// A syntactically valid macro that the safe viewer will not execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedHelpMacro {
    pub invocation: MacroInvocation,
    pub reason: MacroBlockReason,
}

/// Why a macro is rejected by the allow-list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroBlockReason {
    /// Launches a program, shell verb, shortcut, or Control Panel action.
    ExternalExecution,
    /// Loads/registers code from a DLL or equivalent dynamically supplied routine.
    DynamicCode,
    /// Consults or mutates host/system state outside the HLP viewer.
    HostInteraction,
    /// A known WinHelp UI macro that this viewer does not yet emulate safely.
    UnsupportedViewerOperation,
    /// The macro name is unknown and therefore unsafe by default.
    UnknownOperation,
    /// The macro name is recognized, but its arguments do not match the legacy signature.
    InvalidArguments,
}

impl fmt::Display for MacroBlockReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::ExternalExecution => "external process/shell execution is disabled",
            Self::DynamicCode => "dynamic DLL/routine loading is disabled",
            Self::HostInteraction => "host/system interaction is disabled",
            Self::UnsupportedViewerOperation => "viewer-local macro is not implemented",
            Self::UnknownOperation => "unknown macro is blocked by default",
            Self::InvalidArguments => "arguments do not match the recognized WinHelp signature",
        };
        formatter.write_str(text)
    }
}

/// Generic syntax tree used for diagnostics and blocked operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroInvocation {
    pub name: String,
    pub arguments: Vec<MacroArgument>,
}

impl fmt::Display for MacroInvocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}(", self.name)?;
        for (index, argument) in self.arguments.iter().enumerate() {
            if index != 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{argument}")?;
        }
        formatter.write_str(")")
    }
}

/// Literal or nested-call argument in a WinHelp macro invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacroArgument {
    String(String),
    Integer(i32),
    Invocation(Box<MacroInvocation>),
}

impl fmt::Display for MacroArgument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(value) => write!(formatter, "`{}'", value.replace('\\', "\\\\").replace('\'', "\\'")),
            Self::Integer(value) => write!(formatter, "{value}"),
            Self::Invocation(value) => write!(formatter, "{value}"),
        }
    }
}

/// Location-aware syntax failure for malformed macro text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroParseError {
    pub offset: usize,
    pub message: String,
}

impl MacroParseError {
    fn new(offset: usize, message: impl Into<String>) -> Self {
        Self {
            offset,
            message: message.into(),
        }
    }
}

impl fmt::Display for MacroParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "macro parse error at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for MacroParseError {}

/// Converts the generic invocation into the small default-safe command set.
fn classify(invocation: MacroInvocation) -> HelpMacro {
    let canonical = canonical_name(&invocation.name);
    let allowed = match canonical {
        Some("ALink") => one_string(&invocation).map(|keywords| SafeHelpMacro::ALink { keywords }),
        Some("About") => no_args(&invocation).then_some(SafeHelpMacro::About),
        Some("Back") => no_args(&invocation).then_some(SafeHelpMacro::Back),
        Some("BackFlush") => no_args(&invocation).then_some(SafeHelpMacro::BackFlush),
        Some("BookmarkDefine") => no_args(&invocation).then_some(SafeHelpMacro::BookmarkDefine),
        Some("BookmarkMore") => no_args(&invocation).then_some(SafeHelpMacro::BookmarkMore),
        Some("BrowseButtons") => no_args(&invocation).then_some(SafeHelpMacro::BrowseButtons),
        Some("Contents") => no_args(&invocation).then_some(SafeHelpMacro::Contents),
        Some("Finder") => no_args(&invocation).then_some(SafeHelpMacro::Finder),
        Some("FocusWindow") => one_string(&invocation).map(|window| SafeHelpMacro::FocusWindow { window }),
        Some("History") => no_args(&invocation).then_some(SafeHelpMacro::History),
        Some("ExecFile") => exec_file_browser_url(&invocation).map(|url| SafeHelpMacro::OpenUrl { url }),
        Some("JumpContents") => two_strings(&invocation).map(|(help_file, window)| {
            SafeHelpMacro::JumpContents { help_file, window }
        }),
        Some("JumpContext") => two_strings_integer(&invocation).map(|(help_file, window, context)| {
            SafeHelpMacro::JumpContext { help_file, window, context }
        }),
        Some("JumpHash") => two_strings_integer(&invocation).map(|(help_file, window, hash)| {
            SafeHelpMacro::JumpHash { help_file, window, hash }
        }),
        Some("JumpID") => two_strings(&invocation).map(|(path_window, topic_id)| {
            SafeHelpMacro::JumpId { path_window, topic_id }
        }),
        Some("Next") => no_args(&invocation).then_some(SafeHelpMacro::Next),
        Some("Prev") => no_args(&invocation).then_some(SafeHelpMacro::Prev),
        Some("PopupContext") => string_integer(&invocation).map(|(help_file, context)| {
            SafeHelpMacro::PopupContext { help_file, context }
        }),
        Some("PopupHash") => string_integer(&invocation).map(|(help_file, hash)| {
            SafeHelpMacro::PopupHash { help_file, hash }
        }),
        Some("PopupId") => two_strings(&invocation).map(|(help_file, topic_id)| {
            SafeHelpMacro::PopupId { help_file, topic_id }
        }),
        Some("Search") => no_args(&invocation).then_some(SafeHelpMacro::Search),
        Some("SetPopupColor") => three_integers(&invocation).and_then(|(red, green, blue)| {
            Some(SafeHelpMacro::SetPopupColor {
                red: u8::try_from(red).ok()?,
                green: u8::try_from(green).ok()?,
                blue: u8::try_from(blue).ok()?,
            })
        }),
        _ => None,
    };

    if let Some(macro_) = allowed {
        return HelpMacro::Allowed(macro_);
    }

    let reason = if canonical.is_some() && is_allowlisted_name(canonical.unwrap_or_default()) {
        MacroBlockReason::InvalidArguments
    } else {
        blocked_reason(canonical)
    };
    HelpMacro::Blocked(BlockedHelpMacro { invocation, reason })
}

fn is_allowlisted_name(name: &str) -> bool {
    matches!(
        name,
        "ALink"
            | "About"
            | "Back"
            | "BackFlush"
            | "BookmarkDefine"
            | "BookmarkMore"
            | "BrowseButtons"
            | "Contents"
            | "Finder"
            | "FocusWindow"
            | "History"
            | "JumpContents"
            | "JumpContext"
            | "JumpHash"
            | "JumpID"
            | "Next"
            | "Prev"
            | "PopupContext"
            | "PopupHash"
            | "PopupId"
            | "Search"
            | "SetPopupColor"
    )
}

fn blocked_reason(name: Option<&'static str>) -> MacroBlockReason {
    match name {
        Some("ExecFile" | "ExecProgram" | "ShellExecute" | "ShortCut" | "ControlPanel") => {
            MacroBlockReason::ExternalExecution
        }
        Some("RegisterRoutine") => MacroBlockReason::DynamicCode,
        Some("FileExist" | "Generate" | "TCard") => MacroBlockReason::HostInteraction,
        Some(_) => MacroBlockReason::UnsupportedViewerOperation,
        None => MacroBlockReason::UnknownOperation,
    }
}

/// Canonicalizes long names and the documented short aliases used by WinHelp's macro dispatcher.
fn canonical_name(name: &str) -> Option<&'static str> {
    let name = name.trim();
    MACRO_NAMES
        .iter()
        .find(|entry| {
            entry.long.eq_ignore_ascii_case(name)
                || entry.alias.is_some_and(|alias| alias.eq_ignore_ascii_case(name))
        })
        .map(|entry| entry.long)
}

struct MacroName {
    long: &'static str,
    alias: Option<&'static str>,
}

const MACRO_NAMES: &[MacroName] = &[
    MacroName { long: "About", alias: None },
    MacroName { long: "AddAccelerator", alias: Some("AA") },
    MacroName { long: "ALink", alias: Some("AL") },
    MacroName { long: "Annotate", alias: None },
    MacroName { long: "AppendItem", alias: None },
    MacroName { long: "Back", alias: None },
    MacroName { long: "BackFlush", alias: Some("BF") },
    MacroName { long: "BookmarkDefine", alias: None },
    MacroName { long: "BookmarkMore", alias: None },
    MacroName { long: "BrowseButtons", alias: None },
    MacroName { long: "ChangeButtonBinding", alias: Some("CBB") },
    MacroName { long: "ChangeEnable", alias: Some("CE") },
    MacroName { long: "ChangeItemBinding", alias: Some("CIB") },
    MacroName { long: "CheckItem", alias: Some("CI") },
    MacroName { long: "CloseSecondarys", alias: Some("CS") },
    MacroName { long: "CloseWindow", alias: Some("CW") },
    MacroName { long: "Compare", alias: None },
    MacroName { long: "Contents", alias: None },
    MacroName { long: "ControlPanel", alias: None },
    MacroName { long: "CopyDialog", alias: None },
    MacroName { long: "CopyTopic", alias: Some("CT") },
    MacroName { long: "CreateButton", alias: Some("CB") },
    MacroName { long: "DeleteItem", alias: None },
    MacroName { long: "DeleteMark", alias: None },
    MacroName { long: "DestroyButton", alias: None },
    MacroName { long: "DisableButton", alias: Some("DB") },
    MacroName { long: "DisableItem", alias: Some("DI") },
    MacroName { long: "EnableButton", alias: Some("EB") },
    MacroName { long: "EnableItem", alias: Some("EI") },
    MacroName { long: "EndMPrint", alias: None },
    MacroName { long: "ExecFile", alias: Some("EF") },
    MacroName { long: "ExecProgram", alias: Some("EP") },
    MacroName { long: "Exit", alias: None },
    MacroName { long: "ExtAbleItem", alias: None },
    MacroName { long: "ExtInsertItem", alias: None },
    MacroName { long: "ExtInsertMenu", alias: None },
    MacroName { long: "FileExist", alias: Some("FE") },
    MacroName { long: "FileOpen", alias: Some("FO") },
    MacroName { long: "Find", alias: None },
    MacroName { long: "Finder", alias: Some("FD") },
    MacroName { long: "FloatingMenu", alias: None },
    MacroName { long: "Flush", alias: Some("FH") },
    MacroName { long: "FocusWindow", alias: None },
    MacroName { long: "Generate", alias: None },
    MacroName { long: "GotoMark", alias: None },
    MacroName { long: "HelpOn", alias: None },
    MacroName { long: "HelpOnTop", alias: None },
    MacroName { long: "History", alias: None },
    MacroName { long: "InitMPrint", alias: None },
    MacroName { long: "InsertItem", alias: None },
    MacroName { long: "InsertMenu", alias: None },
    MacroName { long: "IfThen", alias: Some("IF") },
    MacroName { long: "IfThenElse", alias: Some("IE") },
    MacroName { long: "IsBook", alias: None },
    MacroName { long: "IsMark", alias: None },
    MacroName { long: "IsNotMark", alias: Some("NM") },
    MacroName { long: "JumpContents", alias: None },
    MacroName { long: "JumpContext", alias: Some("JC") },
    MacroName { long: "JumpHash", alias: Some("JH") },
    MacroName { long: "JumpHelpOn", alias: None },
    MacroName { long: "JumpID", alias: Some("JI") },
    MacroName { long: "JumpKeyword", alias: Some("JK") },
    MacroName { long: "KLink", alias: Some("KL") },
    MacroName { long: "Menu", alias: Some("MU") },
    MacroName { long: "MPrintHash", alias: None },
    MacroName { long: "MPrintID", alias: None },
    MacroName { long: "Next", alias: None },
    MacroName { long: "NoShow", alias: Some("NS") },
    MacroName { long: "PopupContext", alias: Some("PC") },
    MacroName { long: "PopupHash", alias: None },
    MacroName { long: "PopupId", alias: Some("PI") },
    MacroName { long: "PositionWindow", alias: Some("PW") },
    MacroName { long: "Prev", alias: None },
    MacroName { long: "Print", alias: None },
    MacroName { long: "PrinterSetup", alias: None },
    MacroName { long: "RegisterRoutine", alias: Some("RR") },
    MacroName { long: "RemoveAccelerator", alias: Some("RA") },
    MacroName { long: "ResetMenu", alias: None },
    MacroName { long: "SaveMark", alias: None },
    MacroName { long: "Search", alias: None },
    MacroName { long: "SetContents", alias: None },
    MacroName { long: "SetHelpOnFile", alias: None },
    MacroName { long: "SetPopupColor", alias: Some("SPC") },
    MacroName { long: "ShellExecute", alias: Some("SE") },
    MacroName { long: "ShortCut", alias: Some("SH") },
    MacroName { long: "TCard", alias: None },
    MacroName { long: "Test", alias: None },
    MacroName { long: "TestALink", alias: None },
    MacroName { long: "TestKLink", alias: None },
    MacroName { long: "UncheckItem", alias: Some("UI") },
    MacroName { long: "UpdateWindow", alias: Some("UW") },
];

fn no_args(invocation: &MacroInvocation) -> bool {
    invocation.arguments.is_empty()
}

fn one_string(invocation: &MacroInvocation) -> Option<String> {
    match invocation.arguments.as_slice() {
        [MacroArgument::String(value)] => Some(value.clone()),
        _ => None,
    }
}

fn two_strings(invocation: &MacroInvocation) -> Option<(String, String)> {
    match invocation.arguments.as_slice() {
        [MacroArgument::String(left), MacroArgument::String(right)] => {
            Some((left.clone(), right.clone()))
        }
        _ => None,
    }
}

fn string_integer(invocation: &MacroInvocation) -> Option<(String, i32)> {
    match invocation.arguments.as_slice() {
        [MacroArgument::String(value), MacroArgument::Integer(number)] => Some((value.clone(), *number)),
        _ => None,
    }
}

fn two_strings_integer(invocation: &MacroInvocation) -> Option<(String, String, i32)> {
    match invocation.arguments.as_slice() {
        [MacroArgument::String(first), MacroArgument::String(second), MacroArgument::Integer(number)] => {
            Some((first.clone(), second.clone(), *number))
        }
        _ => None,
    }
}

fn three_integers(invocation: &MacroInvocation) -> Option<(i32, i32, i32)> {
    match invocation.arguments.as_slice() {
        [MacroArgument::Integer(first), MacroArgument::Integer(second), MacroArgument::Integer(third)] => {
            Some((*first, *second, *third))
        }
        _ => None,
    }
}

/// Recognizes the constrained `ExecFile` shape used by HelpScribble and similar WinHelp authors
/// for Internet links. Arbitrary files/programs, URL parameters, and non-HTTP(S) schemes remain
/// blocked by the normal external-execution policy.
fn exec_file_browser_url(invocation: &MacroInvocation) -> Option<String> {
    let url = match invocation.arguments.as_slice() {
        [MacroArgument::String(url)] => url,
        [
            MacroArgument::String(url),
            MacroArgument::String(parameters),
            MacroArgument::Integer(_show_command),
        ] if parameters.is_empty() => url,
        [
            MacroArgument::String(url),
            MacroArgument::String(parameters),
            MacroArgument::Integer(_show_command),
            MacroArgument::String(context),
        ] if parameters.is_empty() && context.is_empty() => url,
        _ => return None,
    };
    is_browser_url(url).then(|| url.clone())
}

fn is_browser_url(value: &str) -> bool {
    if value.is_empty()
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return false;
    }
    value
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
        || value
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            bytes: text.as_bytes(),
            offset: 0,
        }
    }

    fn position(&self) -> usize {
        self.offset
    }

    fn is_eof(&self) -> bool {
        self.offset >= self.bytes.len()
    }

    fn skip_space(&mut self) {
        while self.bytes.get(self.offset).is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.offset += 1;
        }
    }

    fn parse_invocation(&mut self, depth: usize) -> Result<MacroInvocation, MacroParseError> {
        if depth >= MAX_MACRO_DEPTH {
            return Err(MacroParseError::new(
                self.offset,
                format!("macro nesting exceeds the {MAX_MACRO_DEPTH}-level safety limit"),
            ));
        }
        self.skip_space();
        let name = self.parse_identifier()?;
        self.skip_space();
        self.expect_byte(b'(', "expected '(' after macro name")?;
        self.skip_space();
        let mut arguments = Vec::new();
        if self.peek_byte() != Some(b')') {
            loop {
                if arguments.len() >= MAX_MACRO_ARGUMENTS {
                    return Err(MacroParseError::new(
                        self.offset,
                        format!("macro has more than {MAX_MACRO_ARGUMENTS} arguments"),
                    ));
                }
                arguments.push(self.parse_argument(depth + 1)?);
                self.skip_space();
                match self.peek_byte() {
                    Some(b',') => {
                        self.offset += 1;
                        self.skip_space();
                    }
                    Some(b')') => break,
                    _ => {
                        return Err(MacroParseError::new(
                            self.offset,
                            "expected ',' or ')' after macro argument",
                        ));
                    }
                }
            }
        }
        self.expect_byte(b')', "expected ')' after macro arguments")?;
        Ok(MacroInvocation { name, arguments })
    }

    fn parse_argument(&mut self, depth: usize) -> Result<MacroArgument, MacroParseError> {
        self.skip_space();
        match self.peek_byte() {
            Some(b'`' | b'\'' | b'"') => self.parse_string().map(MacroArgument::String),
            Some(b'+' | b'-' | b'0'..=b'9') => self.parse_integer().map(MacroArgument::Integer),
            Some(byte) if byte.is_ascii_alphabetic() => self
                .parse_invocation(depth)
                .map(|value| MacroArgument::Invocation(Box::new(value))),
            Some(_) => Err(MacroParseError::new(self.offset, "unsupported macro argument token")),
            None => Err(MacroParseError::new(self.offset, "unexpected end of macro arguments")),
        }
    }

    fn parse_identifier(&mut self) -> Result<String, MacroParseError> {
        let start = self.offset;
        let Some(first) = self.peek_byte() else {
            return Err(MacroParseError::new(start, "expected macro name"));
        };
        if !first.is_ascii_alphabetic() {
            return Err(MacroParseError::new(start, "macro name must begin with an ASCII letter"));
        }
        self.offset += 1;
        while self
            .bytes
            .get(self.offset)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            self.offset += 1;
        }
        Ok(self.text[start..self.offset].to_owned())
    }

    fn parse_integer(&mut self) -> Result<i32, MacroParseError> {
        let start = self.offset;
        if matches!(self.peek_byte(), Some(b'+' | b'-')) {
            self.offset += 1;
        }
        let negative = self.bytes.get(start) == Some(&b'-');
        let digits_start = self.offset;
        let hexadecimal = self.bytes.get(self.offset) == Some(&b'0')
            && matches!(self.bytes.get(self.offset + 1), Some(b'x' | b'X'));
        if hexadecimal {
            self.offset += 2;
            let hex_start = self.offset;
            while self.bytes.get(self.offset).is_some_and(u8::is_ascii_hexdigit) {
                self.offset += 1;
            }
            if self.offset == hex_start {
                return Err(MacroParseError::new(start, "hexadecimal integer has no digits"));
            }
        } else {
            while self.bytes.get(self.offset).is_some_and(u8::is_ascii_digit) {
                self.offset += 1;
            }
            if self.offset == digits_start {
                return Err(MacroParseError::new(start, "integer has no digits"));
            }
        }
        let token = &self.text[start..self.offset];
        if hexadecimal {
            let unsigned = token.strip_prefix('+').or_else(|| token.strip_prefix('-')).unwrap_or(token);
            let digits = unsigned
                .strip_prefix("0x")
                .or_else(|| unsigned.strip_prefix("0X"))
                .unwrap_or(unsigned);
            let value = u32::from_str_radix(digits, 16)
                .map_err(|_| MacroParseError::new(start, "hexadecimal integer is outside 32-bit range"))?;
            if negative {
                let magnitude = i64::from(value);
                i32::try_from(-magnitude).map_err(|_| {
                    MacroParseError::new(start, "integer is outside signed 32-bit range")
                })
            } else {
                // WinHelp stores these values in a 32-bit LONG. Hash constants are commonly
                // written as unsigned hexadecimal, so preserve their bit pattern.
                Ok(i32::from_ne_bytes(value.to_ne_bytes()))
            }
        } else {
            token
                .parse::<i32>()
                .map_err(|_| MacroParseError::new(start, "integer is outside signed 32-bit range"))
        }
    }

    /// Mirrors WinHelp's permissive quote stack while preserving non-ASCII text losslessly.
    fn parse_string(&mut self) -> Result<String, MacroParseError> {
        let start = self.offset;
        let opener = self.peek_char().ok_or_else(|| MacroParseError::new(start, "expected string"))?;
        self.offset += opener.len_utf8();
        let mut stack = vec![opener];
        let mut result = String::new();

        while let Some(ch) = self.peek_char() {
            self.offset += ch.len_utf8();
            if ch == '\\' {
                let escaped = self.peek_char().ok_or_else(|| {
                    MacroParseError::new(self.offset - 1, "trailing escape in macro string")
                })?;
                self.offset += escaped.len_utf8();
                result.push(escaped);
                continue;
            }
            if matches!(ch, '`' | '\'' | '"') {
                let top = *stack.last().expect("quote stack is nonempty while parsing");
                let opens_nested = ch == '`' || (ch == '"' && top != '"');
                if opens_nested {
                    if stack.len() >= MAX_MACRO_DEPTH {
                        return Err(MacroParseError::new(
                            self.offset - ch.len_utf8(),
                            format!("quote nesting exceeds the {MAX_MACRO_DEPTH}-level safety limit"),
                        ));
                    }
                    stack.push(ch);
                    result.push(ch);
                } else {
                    stack.pop();
                    if stack.is_empty() {
                        if result.len() > MAX_MACRO_STRING {
                            return Err(MacroParseError::new(
                                start,
                                format!("macro string exceeds the {MAX_MACRO_STRING}-byte safety limit"),
                            ));
                        }
                        return Ok(result);
                    }
                    result.push(ch);
                }
            } else {
                result.push(ch);
                if result.len() > MAX_MACRO_STRING {
                    return Err(MacroParseError::new(
                        start,
                        format!("macro string exceeds the {MAX_MACRO_STRING}-byte safety limit"),
                    ));
                }
            }
        }
        Err(MacroParseError::new(start, "unterminated WinHelp macro string"))
    }

    fn peek_char(&self) -> Option<char> {
        self.text.get(self.offset..)?.chars().next()
    }

    fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn expect_byte(&mut self, expected: u8, message: &'static str) -> Result<(), MacroParseError> {
        if self.peek_byte() == Some(expected) {
            self.offset += 1;
            Ok(())
        } else {
            Err(MacroParseError::new(self.offset, message))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn associative_link_alias_is_allowlisted_with_semicolon_names() {
        let parsed = HelpMacroProgram::parse(r#"AL("A_CALC_LIST_EQUIV;A_CALC_KEYB_SEQ")"#)
            .expect("ALink macro parses");
        assert_eq!(
            parsed.macros,
            vec![HelpMacro::Allowed(SafeHelpMacro::ALink {
                keywords: "A_CALC_LIST_EQUIV;A_CALC_KEYB_SEQ".to_owned(),
            })]
        );
    }

    #[test]
    fn parses_semicolon_program_and_legacy_quotes() {
        let parsed = HelpMacroProgram::parse(
            "BrowseButtons();JI(`other.hlp>secondary',`Getting_Started');Back()",
        )
        .expect("valid macro program");
        assert_eq!(parsed.macros.len(), 3);
        assert!(matches!(
            &parsed.macros[1],
            HelpMacro::Allowed(SafeHelpMacro::JumpId { path_window, topic_id })
                if path_window == "other.hlp>secondary" && topic_id == "Getting_Started"
        ));
    }

    #[test]
    fn preserves_non_ascii_macro_strings() {
        let parsed = HelpMacroProgram::parse("JI(`ajuda.hlp',`Introdução')")
            .expect("Unicode macro string should remain intact");
        assert!(matches!(
            &parsed.macros[0],
            HelpMacro::Allowed(SafeHelpMacro::JumpId { path_window, topic_id })
                if path_window == "ajuda.hlp" && topic_id == "Introdução"
        ));
    }

    #[test]
    fn parses_signed_decimal_and_hexadecimal_numbers() {
        let decimal = HelpMacroProgram::parse("JC(`',`main',-42)").expect("decimal macro");
        assert!(matches!(
            &decimal.macros[0],
            HelpMacro::Allowed(SafeHelpMacro::JumpContext { context: -42, .. })
        ));
        let hex = HelpMacroProgram::parse("JH(`',`main',0x2A)").expect("hex macro");
        assert!(matches!(
            &hex.macros[0],
            HelpMacro::Allowed(SafeHelpMacro::JumpHash { hash: 42, .. })
        ));
        let full_width = HelpMacroProgram::parse("JH(`',`main',0xFFFFFFFF)")
            .expect("full-width hash macro");
        assert!(matches!(
            &full_width.macros[0],
            HelpMacro::Allowed(SafeHelpMacro::JumpHash { hash: -1, .. })
        ));
    }

    #[test]
    fn aliases_are_case_insensitive() {
        let parsed = HelpMacroProgram::parse("bf();fd();spc(12,34,56)").expect("aliases parse");
        assert!(matches!(parsed.macros[0], HelpMacro::Allowed(SafeHelpMacro::BackFlush)));
        assert!(matches!(parsed.macros[1], HelpMacro::Allowed(SafeHelpMacro::Finder)));
        assert!(matches!(
            parsed.macros[2],
            HelpMacro::Allowed(SafeHelpMacro::SetPopupColor { red: 12, green: 34, blue: 56 })
        ));
    }

    #[test]
    fn dangerous_macros_are_explicitly_blocked() {
        let program = HelpMacroProgram::parse(
            "EF(`calc.exe',`',1,`');RR(`evil.dll',`Entry',`S');SE(`open',`x',1,0,`',`');ControlPanel(`desk.cpl',`',0)",
        )
        .expect("dangerous macros still parse");
        assert!(matches!(
            &program.macros[0],
            HelpMacro::Blocked(BlockedHelpMacro { reason: MacroBlockReason::ExternalExecution, .. })
        ));
        assert!(matches!(
            &program.macros[1],
            HelpMacro::Blocked(BlockedHelpMacro { reason: MacroBlockReason::DynamicCode, .. })
        ));
        assert!(matches!(
            &program.macros[2],
            HelpMacro::Blocked(BlockedHelpMacro { reason: MacroBlockReason::ExternalExecution, .. })
        ));
        assert!(matches!(
            &program.macros[3],
            HelpMacro::Blocked(BlockedHelpMacro { reason: MacroBlockReason::ExternalExecution, .. })
        ));
    }

    #[test]
    fn helpscribble_http_execfile_is_reduced_to_browser_only_navigation() {
        let program = HelpMacroProgram::parse(r#"EF("http://www.helpscribble.com/",`',1)"#)
            .expect("HelpScribble Internet Link macro parses");
        assert_eq!(
            program.macros,
            vec![HelpMacro::Allowed(SafeHelpMacro::OpenUrl {
                url: "http://www.helpscribble.com/".to_owned(),
            })]
        );

        let https = HelpMacroProgram::parse(r#"ExecFile("https://example.com/path")"#)
            .expect("HTTPS ExecFile parses");
        assert!(matches!(
            &https.macros[0],
            HelpMacro::Allowed(SafeHelpMacro::OpenUrl { url }) if url == "https://example.com/path"
        ));
    }

    #[test]
    fn execfile_browser_exception_does_not_enable_general_host_execution() {
        for text in [
            r#"EF("file:///C:/Windows/notepad.exe",`',1)"#,
            r#"EF("calc.exe",`',1)"#,
            r#"EF("https://example.com",`--argument',1)"#,
        ] {
            let program = HelpMacroProgram::parse(text).expect("ExecFile syntax parses");
            assert!(matches!(
                &program.macros[0],
                HelpMacro::Blocked(BlockedHelpMacro {
                    reason: MacroBlockReason::ExternalExecution,
                    ..
                })
            ));
        }
    }

    #[test]
    fn unknown_and_badly_typed_macros_are_blocked_not_executed() {
        let unknown = HelpMacroProgram::parse("Mystery(`x')").expect("unknown syntax still parses");
        assert!(matches!(
            &unknown.macros[0],
            HelpMacro::Blocked(BlockedHelpMacro { reason: MacroBlockReason::UnknownOperation, .. })
        ));
        let wrong = HelpMacroProgram::parse("JumpID(12,34)").expect("generic syntax parses");
        assert!(matches!(
            &wrong.macros[0],
            HelpMacro::Blocked(BlockedHelpMacro { reason: MacroBlockReason::InvalidArguments, .. })
        ));
    }

    #[test]
    fn nested_boolean_syntax_is_preserved_for_diagnostics_and_blocked() {
        let program = HelpMacroProgram::parse("IF(IsBook(),`Contents()')").expect("nested invocation parses");
        assert!(matches!(
            &program.macros[0],
            HelpMacro::Blocked(BlockedHelpMacro { reason: MacroBlockReason::UnsupportedViewerOperation, .. })
        ));
    }

    #[test]
    fn malformed_program_reports_the_byte_offset() {
        let error = HelpMacroProgram::parse("Back();JumpID(`unterminated,`topic')")
            .expect_err("unterminated quote must fail");
        assert!(error.offset >= 7);
        assert!(error.message.contains("unterminated"));
    }
}
