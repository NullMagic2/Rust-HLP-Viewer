//! Command-line, console, and diagnostic support kept out of the wxDragon UI module.


/// Allows only relative local help-file references for automatic catalog/export discovery.
///
/// User-activated hyperlinks retain their explicit cross-file behavior; this guard applies to
/// metadata-driven background loading so an untrusted CNT/GID/HLP cannot trigger absolute, UNC,
/// or drive-qualified access.
pub(crate) fn automatic_relative_help_reference_allowed(target: &str) -> bool {
    let normalized = target.trim().replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') || normalized.starts_with("//") {
        return false;
    }
    let bytes = normalized.as_bytes();
    !bytes.get(1).is_some_and(|byte| *byte == b':')
}


/// Returns a case-folded, slash-normalized identity for comparing help-document paths.
pub(crate) fn path_identity(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

/// Identifies semantic symbol/decorative faces that must not be modernized as ordinary prose.
pub(crate) fn is_semantic_symbol_face(face_name: &str) -> bool {
    let normalized = face_name.to_ascii_lowercase();
    normalized.contains("symbol")
        || normalized.contains("wingdings")
        || normalized.contains("webdings")
        || normalized.contains("dingbats")
        || normalized == "marlett"
}

pub(crate) mod cli {
//! Minimal command-line parsing shared by GUI and diagnostic launch modes.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Requested process mode before wxWidgets has been initialized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchMode {
    Gui { initial_file: Option<PathBuf> },
    Dump { file: PathBuf, verbose: bool },
    ExportHtml { source: PathBuf, target: Option<PathBuf> },
    Help,
    Version,
}

/// A concise command-line error suitable for printing before process exit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError(pub String);

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CliError {}

pub fn parse() -> Result<LaunchMode, CliError> {
    parse_from(std::env::args_os().skip(1))
}

fn parse_from(arguments: impl IntoIterator<Item = OsString>) -> Result<LaunchMode, CliError> {
    parse_resolved(arguments, &|path: &Path| path.is_file())
}

/// Command-line extensions that identify a WinHelp source file by name alone.
const HELP_EXTENSIONS: [&str; 2] = ["hlp", "mvb"];

/// Parses with an injectable filesystem probe so path reassembly can be tested hermetically.
fn parse_resolved(
    arguments: impl IntoIterator<Item = OsString>,
    exists: &dyn Fn(&Path) -> bool,
) -> Result<LaunchMode, CliError> {
    let mut dump_tokens: Option<Vec<OsString>> = None;
    let mut export_tokens: Option<Vec<OsString>> = None;
    let mut verbose = false;
    let mut positional: Vec<OsString> = Vec::new();
    let mut arguments = arguments.into_iter().peekable();

    while let Some(argument) = arguments.next() {
        if let Some(text) = argument.to_str() {
            match text {
                "--help" | "-h" => return Ok(LaunchMode::Help),
                "--version" | "-V" => return Ok(LaunchMode::Version),
                "--verbose" | "-v" => {
                    verbose = true;
                    continue;
                }
                "--dump-file" => {
                    let tokens = collect_path_tokens(&mut arguments);
                    if tokens.is_empty() {
                        return Err(missing_path_error("--dump-file", "a .hlp", &mut arguments));
                    }
                    if dump_tokens.replace(tokens).is_some() {
                        return Err(CliError("--dump-file may be specified only once".to_owned()));
                    }
                    continue;
                }
                "--export-html" => {
                    let tokens = collect_path_tokens(&mut arguments);
                    if tokens.is_empty() {
                        return Err(missing_path_error("--export-html", "a source .hlp", &mut arguments));
                    }
                    if export_tokens.replace(tokens).is_some() {
                        return Err(CliError("--export-html may be specified only once".to_owned()));
                    }
                    continue;
                }
                _ => {}
            }
            if let Some(value) = text.strip_prefix("--dump-file=") {
                if value.is_empty() {
                    return Err(CliError("--dump-file requires a .hlp pathname".to_owned()));
                }
                let mut tokens = vec![OsString::from(value)];
                tokens.extend(collect_path_tokens(&mut arguments));
                if dump_tokens.replace(tokens).is_some() {
                    return Err(CliError("--dump-file may be specified only once".to_owned()));
                }
                continue;
            }
            if let Some(value) = text.strip_prefix("--export-html=") {
                if value.is_empty() {
                    return Err(CliError("--export-html requires a source .hlp pathname".to_owned()));
                }
                let mut tokens = vec![OsString::from(value)];
                tokens.extend(collect_path_tokens(&mut arguments));
                if export_tokens.replace(tokens).is_some() {
                    return Err(CliError("--export-html may be specified only once".to_owned()));
                }
                continue;
            }
            if text.starts_with('-') {
                return Err(CliError(format!("unknown option: {text}")));
            }
        }
        positional.push(argument);
    }

    if let Some(tokens) = export_tokens {
        if dump_tokens.is_some() || !positional.is_empty() {
            return Err(CliError(
                "do not combine --export-html with --dump-file or a positional HLP pathname".to_owned(),
            ));
        }
        if verbose {
            return Err(CliError("--verbose is valid only with --dump-file".to_owned()));
        }
        let (source, target) = split_source_and_target(&tokens, exists);
        return Ok(LaunchMode::ExportHtml { source, target });
    }

    if let Some(tokens) = dump_tokens {
        if !positional.is_empty() {
            return Err(CliError(
                "do not combine --dump-file with a positional HLP pathname".to_owned(),
            ));
        }
        Ok(LaunchMode::Dump { file: joined_path(&tokens), verbose })
    } else {
        if verbose {
            return Err(CliError("--verbose is valid only with --dump-file".to_owned()));
        }
        let initial_file = if positional.is_empty() {
            None
        } else {
            let joined = joined_path(&positional);
            // Two pathnames that both name real files are still two documents, not one pathname
            // that a shell split apart.
            if positional.len() > 1 && !exists(&joined) && exists(Path::new(&positional[0])) {
                return Err(CliError("only one HLP pathname may be opened at startup".to_owned()));
            }
            Some(joined)
        };
        Ok(LaunchMode::Gui { initial_file })
    }
}

/// Takes every following argument that is not itself an option.
fn collect_path_tokens<I: Iterator<Item = OsString>>(
    arguments: &mut std::iter::Peekable<I>,
) -> Vec<OsString> {
    let mut tokens = Vec::new();
    while arguments
        .peek()
        .is_some_and(|value| !value.to_str().is_some_and(|text| text.starts_with('-')))
    {
        if let Some(token) = arguments.next() {
            tokens.push(token);
        }
    }
    tokens
}

fn missing_path_error<I: Iterator<Item = OsString>>(
    option: &str,
    article: &str,
    arguments: &mut std::iter::Peekable<I>,
) -> CliError {
    if arguments.peek().is_some() {
        CliError(format!("{option} requires {article} pathname before another option"))
    } else {
        CliError(format!("{option} requires {article} pathname"))
    }
}

/// Rejoins the pathname tokens that follow a path option.
///
/// A Windows shell splits an unquoted `D:\Rusty HLP viewer\CALC.HLP` into several arguments, so the
/// tokens are rejoined with the single space that separated them. A correctly quoted path still
/// arrives as one token and is used unchanged.
fn joined_path(tokens: &[OsString]) -> PathBuf {
    if let [single] = tokens {
        return PathBuf::from(single);
    }
    let mut joined = OsString::new();
    for (index, token) in tokens.iter().enumerate() {
        if index > 0 {
            joined.push(" ");
        }
        joined.push(token);
    }
    PathBuf::from(joined)
}

fn has_help_extension(token: &OsString) -> bool {
    Path::new(token)
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| HELP_EXTENSIONS.iter().any(|known| value.eq_ignore_ascii_case(known)))
}

/// Splits `--export-html` tokens into the source pathname and the optional target pathname.
///
/// The source is a leading run of tokens and the target is whatever follows it, so the split point
/// is found by asking the filesystem first: the shortest leading run that names a real file is the
/// source. When nothing matches - a mistyped path, or a source about to be reported as missing -
/// the authored `.hlp`/`.mvb` extension terminates the source instead, and the caller's own
/// "could not open" message then names the full reassembled path.
fn split_source_and_target(
    tokens: &[OsString],
    exists: &dyn Fn(&Path) -> bool,
) -> (PathBuf, Option<PathBuf>) {
    if tokens.len() <= 1 {
        return (joined_path(tokens), None);
    }
    let remainder = |split: usize| (split < tokens.len()).then(|| joined_path(&tokens[split..]));
    for split in 1..=tokens.len() {
        let candidate = joined_path(&tokens[..split]);
        if exists(&candidate) {
            return (candidate, remainder(split));
        }
    }
    for split in 1..=tokens.len() {
        if has_help_extension(&tokens[split - 1]) {
            return (joined_path(&tokens[..split]), remainder(split));
        }
    }
    if tokens.len() == 2 {
        return (joined_path(&tokens[..1]), remainder(1));
    }
    (joined_path(tokens), None)
}

pub const fn usage() -> &'static str {
    concat!(
        "Rust HLP Viewer ", env!("CARGO_PKG_VERSION"), "\n\n",
        "Usage:\n",
        "  hlp-viewer.exe [file.hlp]\n",
        "  hlp-viewer.exe --dump-file <file.hlp> [--verbose]\n",
        "  hlp-viewer.exe --export-html <source.hlp> [target.html]\n",
        "  hlp-viewer.exe --help\n",
        "  hlp-viewer.exe --version\n\n",
        "Options:\n",
        "  --dump-file <file>  Decode and print HLP diagnostics without starting wxDragon.\n",
        "  --export-html <source> [target]\n",
        "                      Export directly to self-contained HTML without starting wxDragon.\n",
        "                      If target is omitted, source.hlp becomes source.html.\n",
        "  --verbose, -v       Add per-topic/per-record formatting diagnostics to a dump.\n",
        "  --help, -h          Show this help.\n",
        "  --version, -V       Show the program version.\n\n",
        "Pathnames:\n",
        "  Quoting a pathname that contains spaces is always accurate:\n",
        "    hlp-viewer.exe --export-html \"D:\\Rusty HLP viewer\\CALC.HLP\" \"D:\\out\\calc.html\"\n",
        "  An unquoted pathname arrives split into several arguments, so the spaces are put back\n",
        "  and the source is taken to be the leading run of them that names a real file.\n",
        "  --dump-file=<file> and --export-html=<source> are also accepted.\n",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn dump_mode_does_not_need_a_positional_file() {
        assert_eq!(
            parse_from(args(&["--dump-file", "manual.hlp", "--verbose"])),
            Ok(LaunchMode::Dump {
                file: PathBuf::from("manual.hlp"),
                verbose: true,
            })
        );
    }

    #[test]
    fn a_single_positional_path_opens_in_gui_mode() {
        assert_eq!(
            parse_from(args(&["manual.hlp"])),
            Ok(LaunchMode::Gui {
                initial_file: Some(PathBuf::from("manual.hlp")),
            })
        );
    }

    #[test]
    fn verbose_without_dump_is_rejected() {
        assert!(parse_from(args(&["--verbose"])).is_err());
    }

    #[test]
    fn export_html_accepts_source_and_target() {
        assert_eq!(
            parse_from(args(&["--export-html", "manual.hlp", "manual-export.html"])),
            Ok(LaunchMode::ExportHtml {
                source: PathBuf::from("manual.hlp"),
                target: Some(PathBuf::from("manual-export.html")),
            })
        );
    }

    #[test]
    fn export_html_target_is_optional() {
        assert_eq!(
            parse_from(args(&["--export-html", "manual.hlp"])),
            Ok(LaunchMode::ExportHtml {
                source: PathBuf::from("manual.hlp"),
                target: None,
            })
        );
    }

    #[test]
    fn export_html_requires_a_source() {
        assert!(parse_from(args(&["--export-html"])).is_err());
    }

    #[test]
    fn export_html_is_not_combined_with_dump_mode() {
        assert!(
            parse_from(args(&[
                "--export-html",
                "manual.hlp",
                "manual.html",
                "--dump-file",
                "other.hlp",
            ]))
            .is_err()
        );
    }

    /// Filesystem stub: only the listed pathnames are real files.
    fn only(paths: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |path: &Path| paths.iter().any(|known| Path::new(known) == path)
    }

    #[test]
    fn export_html_rejoins_an_unquoted_windows_path() {
        // cmd.exe splits this into six arguments; the source is the leading run that is a real file.
        let source = r"D:\Work\Rusty HLP viewer\Backups\HLP file examples\CALC.HLP";
        assert_eq!(
            parse_resolved(
                args(&[
                    "--export-html",
                    r"D:\Work\Rusty",
                    "HLP",
                    r"viewer\Backups\HLP",
                    "file",
                    r"examples\CALC.HLP",
                    "calc3",
                ]),
                &only(&[r"D:\Work\Rusty HLP viewer\Backups\HLP file examples\CALC.HLP"]),
            ),
            Ok(LaunchMode::ExportHtml {
                source: PathBuf::from(source),
                target: Some(PathBuf::from("calc3")),
            })
        );
    }

    #[test]
    fn export_html_rejoins_an_unquoted_path_without_a_target() {
        assert_eq!(
            parse_resolved(
                args(&["--export-html", r"D:\Rusty", r"HLP viewer\CALC.HLP"]),
                &only(&[r"D:\Rusty HLP viewer\CALC.HLP"]),
            ),
            Ok(LaunchMode::ExportHtml {
                source: PathBuf::from(r"D:\Rusty HLP viewer\CALC.HLP"),
                target: None,
            })
        );
    }

    #[test]
    fn export_html_falls_back_to_the_help_extension_when_nothing_is_on_disk() {
        // A mistyped or not-yet-created path still splits sensibly, so the failure the user sees is
        // "could not open <full path>" rather than a usage error.
        assert_eq!(
            parse_resolved(
                args(&["--export-html", r"D:\Rusty", r"HLP viewer\CALC.HLP", "out", "file.html"]),
                &only(&[]),
            ),
            Ok(LaunchMode::ExportHtml {
                source: PathBuf::from(r"D:\Rusty HLP viewer\CALC.HLP"),
                target: Some(PathBuf::from("out file.html")),
            })
        );
    }

    #[test]
    fn export_html_accepts_an_inline_equals_source() {
        assert_eq!(
            parse_from(args(&["--export-html=manual.hlp", "manual-export.html"])),
            Ok(LaunchMode::ExportHtml {
                source: PathBuf::from("manual.hlp"),
                target: Some(PathBuf::from("manual-export.html")),
            })
        );
    }

    #[test]
    fn dump_file_rejoins_an_unquoted_path() {
        assert_eq!(
            parse_from(args(&["--dump-file", r"D:\Rusty", r"HLP viewer\CALC.HLP", "--verbose"])),
            Ok(LaunchMode::Dump {
                file: PathBuf::from(r"D:\Rusty HLP viewer\CALC.HLP"),
                verbose: true,
            })
        );
    }

    #[test]
    fn gui_rejoins_an_unquoted_path_but_still_rejects_two_documents() {
        assert_eq!(
            parse_resolved(args(&[r"D:\Rusty", r"HLP viewer\CALC.HLP"]), &only(&[])),
            Ok(LaunchMode::Gui {
                initial_file: Some(PathBuf::from(r"D:\Rusty HLP viewer\CALC.HLP")),
            })
        );
        assert!(
            parse_resolved(args(&["one.hlp", "two.hlp"]), &only(&["one.hlp", "two.hlp"])).is_err()
        );
    }
}
}

pub(crate) mod console {
//! Windows console bridge for the GUI-subsystem executable's diagnostic mode.

/// Attaches a `/SUBSYSTEM:WINDOWS` process to the shell that launched it, when possible.
///
/// This is intentionally called only for command-line output modes, before the first stdout or
/// stderr access. Failure is harmless: inherited/redirection handles can still be usable, and the
/// caller will simply write through Rust's standard streams.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
pub fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

    // SAFETY: AttachConsole takes no pointers. ATTACH_PARENT_PROCESS asks Windows to attach this
    // process to its parent's console. The return value only reports success/failure and there is
    // no Rust-owned memory or resource whose validity depends on it.
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(target_os = "windows"))]
pub const fn attach_parent_console() {}
}

pub(crate) mod recent {
//! Persistent most-recently-used HLP history stored in a small human-readable `.cfg` file.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const MAX_RECENT_DOCUMENTS: usize = 5;
const CONFIG_FILE_NAME: &str = "hlp-viewer.cfg";
const RECENT_KEY: &str = "recent_document=";

/// Returns the persistent configuration file used by the native viewer.
///
/// The configuration is deliberately portable: `hlp-viewer.cfg` lives beside the running
/// executable on every platform. If the executable location cannot be determined, configuration
/// loading/saving fails rather than silently writing to a per-user or working-directory fallback.
pub fn config_path() -> io::Result<PathBuf> {
    let executable = env::current_exe()?;
    let directory = executable.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "the running executable has no parent directory",
        )
    })?;
    Ok(directory.join(CONFIG_FILE_NAME))
}

/// Loads at most five recent document paths. A missing config file is the normal first-run case.
pub fn load() -> io::Result<Vec<PathBuf>> {
    match fs::read_to_string(config_path()?) {
        Ok(text) => Ok(parse(&text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

/// Persists the current MRU list beside the running executable.
pub fn save(paths: &[PathBuf]) -> io::Result<()> {
    fs::write(config_path()?, serialize(paths))
}

/// Converts a successfully opened command-line/dialog path into a stable absolute display path
/// without Windows' `\\?\\` canonicalization prefix.
pub fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

/// Moves a successfully opened document to the front of the MRU list and caps it at five entries.
pub fn record(paths: &mut Vec<PathBuf>, path: PathBuf) {
    paths.retain(|existing| !same_path(existing, &path));
    paths.insert(0, path);
    paths.truncate(MAX_RECENT_DOCUMENTS);
}

fn parse(text: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for line in text.lines() {
        let Some(value) = line.strip_prefix(RECENT_KEY) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        let path = PathBuf::from(value);
        if paths.iter().any(|existing| same_path(existing, &path)) {
            continue;
        }
        paths.push(path);
        if paths.len() == MAX_RECENT_DOCUMENTS {
            break;
        }
    }
    paths
}

fn serialize(paths: &[PathBuf]) -> String {
    let mut output = String::from("# Rust HLP Viewer configuration\n# Most recent document first; maximum 5 entries.\n");
    for path in paths.iter().take(MAX_RECENT_DOCUMENTS) {
        let value = path.to_string_lossy().replace('\r', " ").replace('\n', " ");
        if value.is_empty() {
            continue;
        }
        output.push_str(RECENT_KEY);
        output.push_str(&value);
        output.push('\n');
    }
    output
}

fn same_path(left: &Path, right: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(target_os = "windows"))]
    {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cfg_path_is_beside_the_running_executable() {
        let executable = env::current_exe().expect("current executable path");
        let expected = executable
            .parent()
            .expect("current executable parent")
            .join(CONFIG_FILE_NAME);
        assert_eq!(config_path().expect("config path"), expected);
    }

    #[test]
    fn cfg_parser_ignores_comments_unknown_keys_and_caps_recent_documents() {
        let text = concat!(
            "# comment\n",
            "other=value\n",
            "recent_document=one.hlp\n",
            "recent_document=two.hlp\n",
            "recent_document=three.hlp\n",
            "recent_document=four.hlp\n",
            "recent_document=five.hlp\n",
            "recent_document=six.hlp\n",
        );
        let parsed = parse(text);
        assert_eq!(parsed.len(), MAX_RECENT_DOCUMENTS);
        assert_eq!(parsed[0], PathBuf::from("one.hlp"));
        assert_eq!(parsed[4], PathBuf::from("five.hlp"));
    }

    #[test]
    fn recording_moves_an_existing_document_to_front_and_keeps_five() {
        let mut paths = vec![
            PathBuf::from("one.hlp"),
            PathBuf::from("two.hlp"),
            PathBuf::from("three.hlp"),
            PathBuf::from("four.hlp"),
            PathBuf::from("five.hlp"),
        ];
        record(&mut paths, PathBuf::from("three.hlp"));
        assert_eq!(paths[0], PathBuf::from("three.hlp"));
        assert_eq!(paths.len(), 5);
        record(&mut paths, PathBuf::from("six.hlp"));
        assert_eq!(paths[0], PathBuf::from("six.hlp"));
        assert_eq!(paths.len(), 5);
        assert!(!paths.contains(&PathBuf::from("five.hlp")));
    }

    #[test]
    fn serialized_cfg_round_trips_in_mru_order() {
        let paths = vec![PathBuf::from("alpha.hlp"), PathBuf::from("beta.hlp")];
        assert_eq!(parse(&serialize(&paths)), paths);
    }
}
}


pub(crate) mod bookmarks {
//! Persistent file-qualified bookmarks stored beside the executable.

use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

const BOOKMARK_KEY: &str = "bookmark=";

/// One bookmark serialized independently of the loaded `HelpDocument`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBookmark {
    pub label: String,
    pub source_path: PathBuf,
    pub topic_index: usize,
    pub topic_offset: Option<i32>,
    pub window_name: Option<String>,
}

/// Returns `<program-name>.bookmarks` beside the running executable.
///
/// For the normal `hlp-viewer.exe` build this is `hlp-viewer.bookmarks`. Deriving the stem from
/// the executable also keeps the behavior intuitive when a portable build is renamed.
pub fn bookmarks_path() -> io::Result<PathBuf> {
    let executable = env::current_exe()?;
    let directory = executable.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "the running executable has no parent directory",
        )
    })?;
    let program_name = executable
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("hlp-viewer");
    Ok(directory.join(format!("{program_name}.bookmarks")))
}

/// Loads every valid persisted bookmark. Missing storage is the normal first-run case.
pub fn load() -> io::Result<Vec<StoredBookmark>> {
    match fs::read_to_string(bookmarks_path()?) {
        Ok(text) => Ok(parse(&text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

/// Writes the complete bookmark set beside the executable.
pub fn save(bookmarks: &[StoredBookmark]) -> io::Result<()> {
    fs::write(bookmarks_path()?, serialize(bookmarks))
}

fn serialize(bookmarks: &[StoredBookmark]) -> String {
    let mut output = String::from(
        "# Rust HLP Viewer bookmarks\n# label\\tsource_path\\ttopic_index\\ttopic_offset\\twindow_name\n",
    );
    for bookmark in bookmarks {
        if bookmark.source_path.as_os_str().is_empty() {
            continue;
        }
        output.push_str(BOOKMARK_KEY);
        output.push_str(&escape_field(&bookmark.label));
        output.push('\t');
        output.push_str(&escape_field(&bookmark.source_path.to_string_lossy()));
        output.push('\t');
        output.push_str(&bookmark.topic_index.to_string());
        output.push('\t');
        if let Some(offset) = bookmark.topic_offset {
            output.push_str(&offset.to_string());
        }
        output.push('\t');
        if let Some(window_name) = &bookmark.window_name {
            output.push_str(&escape_field(window_name));
        }
        output.push('\n');
    }
    output
}

fn parse(text: &str) -> Vec<StoredBookmark> {
    let mut bookmarks = Vec::new();
    for line in text.lines() {
        let Some(value) = line.strip_prefix(BOOKMARK_KEY) else {
            continue;
        };
        let mut fields = value.split('\t');
        let (Some(label), Some(source_path), Some(topic_index), Some(topic_offset), Some(window_name)) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            continue;
        };
        if fields.next().is_some() {
            continue;
        }
        let (Some(label), Some(source_path), Some(window_name)) = (
            unescape_field(label),
            unescape_field(source_path),
            unescape_field(window_name),
        ) else {
            continue;
        };
        if source_path.is_empty() {
            continue;
        }
        let Ok(topic_index) = topic_index.parse::<usize>() else {
            continue;
        };
        let topic_offset = if topic_offset.is_empty() {
            None
        } else {
            let Ok(offset) = topic_offset.parse::<i32>() else {
                continue;
            };
            Some(offset)
        };
        let window_name = (!window_name.is_empty()).then_some(window_name);
        let bookmark = StoredBookmark {
            label,
            source_path: PathBuf::from(source_path),
            topic_index,
            topic_offset,
            window_name,
        };
        if bookmarks.iter().any(|existing: &StoredBookmark| {
            existing.source_path == bookmark.source_path
                && existing.topic_index == bookmark.topic_index
                && existing.topic_offset == bookmark.topic_offset
                && existing.window_name == bookmark.window_name
        }) {
            continue;
        }
        bookmarks.push(bookmark);
    }
    bookmarks
}

fn escape_field(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '\t' => output.push_str("\\t"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            _ => output.push(character),
        }
    }
    output
}

fn unescape_field(value: &str) -> Option<String> {
    let mut output = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        match characters.next()? {
            '\\' => output.push('\\'),
            't' => output.push('\t'),
            'n' => output.push('\n'),
            'r' => output.push('\r'),
            _ => return None,
        }
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmark_path_uses_the_running_program_name() {
        let executable = env::current_exe().expect("current executable path");
        let directory = executable.parent().expect("current executable parent");
        let stem = executable
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("hlp-viewer");
        assert_eq!(
            bookmarks_path().expect("bookmark path"),
            directory.join(format!("{stem}.bookmarks"))
        );
    }

    #[test]
    fn bookmarks_round_trip_escaped_fields_and_optional_values() {
        let bookmarks = vec![
            StoredBookmark {
                label: "Intro\\tab\tline".to_owned(),
                source_path: PathBuf::from(r"C:\\Help Files\\manual.hlp"),
                topic_index: 7,
                topic_offset: Some(-32),
                window_name: Some("secondary\\pane".to_owned()),
            },
            StoredBookmark {
                label: "Contents".to_owned(),
                source_path: PathBuf::from("other.hlp"),
                topic_index: 0,
                topic_offset: None,
                window_name: None,
            },
        ];
        assert_eq!(parse(&serialize(&bookmarks)), bookmarks);
    }

    #[test]
    fn parser_ignores_malformed_and_duplicate_rows() {
        let text = concat!(
            "# comment\n",
            "bookmark=One\tmanual.hlp\t1\t42\t\n",
            "bookmark=Duplicate label\tmanual.hlp\t1\t42\t\n",
            "bookmark=bad\tmissing-fields\n",
            "bookmark=bad\tmanual.hlp\tnot-a-number\t\t\n",
        );
        let parsed = parse(text);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].label, "One");
    }
}
}

pub(crate) mod dump {
//! Text diagnostics used by `hlp-viewer --dump-file` without initializing wxDragon.

use std::io::{self, Write};
use std::path::Path;

use hlp::{FormattedRecord, HelpMacro, HelpMacroProgram, HlpFile, HotspotTarget, Inline};

/// Writes a deterministic HLP structural dump to `output`.
pub fn inspect(path: &Path, verbose: bool, output: &mut impl Write) -> Result<(), Box<dyn std::error::Error>> {
    let hlp = HlpFile::open(path)?;
    let header = hlp.header();
    writeln!(output, "File: {}", path.display())?;
    writeln!(output, "Directory offset: 0x{:08X}", header.directory_start)?;
    writeln!(output, "Free-list offset: {}", header.first_free_block)?;
    writeln!(output, "Logical file size: {} bytes", header.entire_file_size)?;
    writeln!(output)?;
    writeln!(output, "Internal files ({}):", hlp.directory().len())?;
    for entry in hlp.directory() {
        let internal = hlp.internal_file(&entry.name)?;
        writeln!(
            output,
            "  {:<24} @ 0x{:08X}  used={} reserved={} flags=0x{:02X}",
            entry.name, entry.file_offset, internal.used_space, internal.reserved_space, internal.flags
        )?;
    }

    writeln!(output)?;
    let system = hlp.system_info()?;
    writeln!(output, "|SYSTEM:")?;
    writeln!(output, "  version: {:?} ({}.{})", system.version, system.major, system.minor)?;
    writeln!(output, "  title: {}", system.title.as_deref().unwrap_or("<none>"))?;
    writeln!(output, "  compression: {:?}", system.compression)?;
    writeln!(output, "  physical topic block: {} bytes", system.topic_block_size)?;
    writeln!(output, "  decoded topic data/block: {} bytes", system.topic_decompressed_block_size)?;
    writeln!(output, "  flags: 0x{:04X}", system.flags)?;
    writeln!(output, "  windows: {}", system.windows.len())?;
    for (index, window) in system.windows.iter().enumerate() {
        writeln!(
            output,
            "    #{index}: name={:?} caption={:?} type={:?} rect=({:?},{:?},{:?},{:?}) topmost={} autosize={}",
            window.name,
            window.caption,
            window.window_type,
            window.x,
            window.y,
            window.width,
            window.height,
            window.always_on_top,
            window.auto_size_height
        )?;
    }
    if !system.config_macros.is_empty() {
        writeln!(output, "  startup macros (safe-policy parsed):")?;
        for macro_text in &system.config_macros {
            writeln!(output, "    {macro_text}")?;
            if verbose {
                inspect_macro_policy(output, macro_text, "      ")?;
            }
        }
    }

    writeln!(output)?;
    let navigation = hlp.navigation_metadata()?;
    writeln!(output, "Navigation:")?;
    writeln!(output, "  |TOMAP entries: {}", navigation.topic_map().len())?;
    writeln!(output, "  |CONTEXT entries: {}", navigation.contexts().len())?;
    writeln!(output, "  |CTXOMAP entries: {}", navigation.context_map().len())?;
    writeln!(output, "  |TopicId entries: {}", navigation.topic_ids().len())?;
    writeln!(output, "  |Viola entries: {}", navigation.default_windows().len())?;
    if verbose {
        for (number, position) in navigation.topic_map().iter().enumerate() {
            writeln!(output, "    HC30 topic #{number:<6} -> TOPICPOS {}", position.0)?;
        }
        for entry in navigation.contexts() {
            writeln!(output, "    context hash={:11} -> TOPICOFFSET {}", entry.hash, entry.offset.0)?;
        }
        for entry in navigation.context_map() {
            writeln!(output, "    map id={:11} -> TOPICOFFSET {}", entry.map_id, entry.offset.0)?;
        }
        for entry in navigation.topic_ids() {
            writeln!(output, "    topic id {:?} -> TOPICOFFSET {}", entry.name, entry.offset.0)?;
        }
        for entry in navigation.default_windows() {
            writeln!(output, "    window {} from TOPICOFFSET {}", entry.window_number, entry.offset.0)?;
        }
    }

    writeln!(output)?;
    let fonts = hlp.fonts()?;
    writeln!(output, "|FONT:")?;
    writeln!(output, "  metric: {:?}", fonts.metric())?;
    writeln!(output, "  faces: {}", fonts.face_names().len())?;
    writeln!(output, "  descriptors: {}", fonts.descriptors().len())?;
    if verbose {
        for (index, font) in fonts.descriptors().iter().enumerate() {
            writeln!(
                output,
                "    #{index:<3} {:<24} {:>3}pt weight={:<4} italic={} underline={} strike={} rgb=({},{},{})",
                font.face_name,
                font.point_size(),
                font.weight,
                font.italic,
                font.underline,
                font.strike_out,
                font.foreground.red,
                font.foreground.green,
                font.foreground.blue
            )?;
        }
    }

    writeln!(output)?;
    let topics = hlp.topics()?;
    writeln!(output, "|TOPIC:")?;
    writeln!(output, "  physical blocks: {}", topics.blocks().len())?;
    writeln!(output, "  phrase compression: {:?}", topics.phrase_compression())?;
    writeln!(output, "  decoded phrases: {}", topics.phrase_count())?;
    writeln!(output, "  topics: {}", topics.topics().len())?;

    writeln!(output)?;
    for (index, topic) in topics.topics().iter().enumerate() {
        writeln!(output, "Topic {index}: pos={} title={:?}", topic.id.0.0, topic.title)?;
        writeln!(
            output,
            "  browse: back={:?} forward={:?} old-back={:?} old-forward={:?}",
            topic.metadata.browse_back.map(|value| value.0),
            topic.metadata.browse_forward.map(|value| value.0),
            topic.metadata.previous_topic_number,
            topic.metadata.next_topic_number
        )?;
        writeln!(
            output,
            "  records: fixed={} scrolling={} other={}",
            topic.non_scrolling.len(), topic.scrolling.len(), topic.unclassified.len()
        )?;
        if verbose {
            inspect_region(output, "fixed", &topic.non_scrolling)?;
            inspect_region(output, "scrolling", &topic.scrolling)?;
        }
        if !topic.macros.is_empty() {
            writeln!(output, "  topic macros (safe-policy parsed): {}", topic.macros.len())?;
            if verbose {
                for macro_text in &topic.macros {
                    writeln!(output, "    {macro_text}")?;
                    inspect_macro_policy(output, macro_text, "      ")?;
                }
            }
        }
    }
    Ok(())
}

fn inspect_macro_policy(
    output: &mut impl Write,
    text: &str,
    indent: &str,
) -> io::Result<()> {
    match HelpMacroProgram::parse(text) {
        Ok(program) => {
            for macro_ in program.macros {
                match macro_ {
                    HelpMacro::Allowed(command) => {
                        writeln!(output, "{indent}ALLOW {command:?}")?;
                    }
                    HelpMacro::Blocked(blocked) => {
                        writeln!(
                            output,
                            "{indent}BLOCK {} — {}",
                            blocked.invocation, blocked.reason
                        )?;
                    }
                }
            }
        }
        Err(error) => writeln!(output, "{indent}BLOCK malformed — {error}")?,
    }
    Ok(())
}

fn inspect_region(
    output: &mut impl Write,
    label: &str,
    records: &[hlp::TopicRecord],
) -> io::Result<()> {
    for (record_index, record) in records.iter().enumerate() {
        match FormattedRecord::decode(record) {
            Ok(formatted) => {
                let mut text_runs = 0_usize;
                let mut hotspots = 0_usize;
                let mut pictures = 0_usize;
                for paragraph in &formatted.paragraphs {
                    for inline in &paragraph.inlines {
                        match inline {
                            Inline::Text(run) => {
                                text_runs += 1;
                                hotspots += usize::from(run.hotspot.is_some());
                                if let Some(hotspot) = &run.hotspot {
                                    writeln!(output, "      hotspot: {}", describe_target(&hotspot.target))?;
                                }
                            }
                            Inline::Picture(_) => pictures += 1,
                            _ => {}
                        }
                    }
                }
                writeln!(
                    output,
                    "  {label}[{record_index}]: pos={} paragraphs={} table={} text-runs={} hotspots={} pictures={} issues={}",
                    record.position.0,
                    formatted.paragraphs.len(),
                    formatted.table.is_some(),
                    text_runs,
                    hotspots,
                    pictures,
                    formatted.issues.len()
                )?;
                for issue in &formatted.issues {
                    writeln!(output, "      warning @ LinkData1+0x{:X}: {}", issue.link_data1_offset, issue.message)?;
                }
            }
            Err(error) => writeln!(output, "  {label}[{record_index}]: formatting decode error: {error}")?,
        }
    }
    Ok(())
}

fn describe_target(target: &HotspotTarget) -> String {
    match target {
        HotspotTarget::Internal { offset, popup } => {
            format!("internal TOPICOFFSET={} popup={popup}", offset.0)
        }
        HotspotTarget::ContextHash { hash, popup } => {
            format!("internal context hash=0x{:08X} popup={popup}", *hash as u32)
        }
        HotspotTarget::External {
            opcode,
            type_code,
            offset,
            window_number,
            help_file,
            window_name,
        } => format!(
            "external opcode=0x{opcode:02X} type={type_code} TOPICOFFSET={} window={window_number:?} file={help_file:?} name={window_name:?}",
            offset.0
        ),
        HotspotTarget::Macro(text) => format!("macro (safe-policy dispatch): {text}"),
    }
}
}
