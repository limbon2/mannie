//! Installed programs that plugins run (ADR-0038): finding a program on the
//! `PATH`, and running it with its arguments exactly, without a shell, under
//! the process supervisor, which contains its tree as it contains a
//! command's.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::Metadata;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::paths::{self, FinalLink};

/// The most bytes of input that a program's run receives.
pub const MAX_PROGRAM_INPUT_BYTES: usize = 16 * 1024 * 1024;

/// The largest limit of a program's output, in bytes.
pub const MAX_PROGRAM_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

/// The bytes of a program's error output that a run keeps. The rest is
/// counted and dropped, so the program never blocks on a full pipe.
pub const PROGRAM_ERROR_BYTES: usize = 256 * 1024;

/// The longest that a program's run may last, as a command's.
pub const MAX_PROGRAM_TIMEOUT: Duration = Duration::from_secs(600);

/// The longest name of a program, in characters.
const MAX_PROGRAM_NAME_CHARACTERS: usize = 64;

/// Why an installed program was not found.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProgramNotFound {
    /// The name is not a plain name of a program, such as `git`.
    #[error("{name:?} is not the name of a program")]
    InvalidName {
        /// The name, as it was given.
        name: String,
    },
    /// No entry of the `PATH` holds the program, or every one that does
    /// breaks a rule of [`find_program`].
    #[error("{name} was not found on the PATH")]
    NotOnPath {
        /// The program's name.
        name: String,
    },
}

/// Finds an installed program on a `PATH` (ADR-0038, decision 7), and returns
/// its canonical path with its links resolved.
///
/// Only absolute entries count, so a relative entry never makes Mannie's own
/// working folder a place to find programs. An entry that could reach a
/// network host, such as a UNC path, is skipped before any access to the file
/// system (T-17). A program inside one of the `excluded` folders, such as a
/// project's folder, where a model's changes could have planted it, or
/// Mannie's own folders, is skipped (T-46). On Windows only `.exe` files
/// count: `cmd.exe` would read a `.bat` or `.cmd` file as text, with the
/// arguments in it.
pub fn find_program(
    name: &str,
    path_variable: &OsStr,
    excluded: &[PathBuf],
) -> Result<PathBuf, ProgramNotFound> {
    if !is_program_name(name) {
        return Err(ProgramNotFound::InvalidName {
            name: name.to_owned(),
        });
    }
    let file_name = program_file_name(name);
    std::env::split_paths(path_variable)
        .find_map(|entry| program_in(&entry, &file_name, excluded))
        .ok_or_else(|| ProgramNotFound::NotOnPath {
            name: name.to_owned(),
        })
}

/// Whether `name` is a plain name of a program: a letter or a digit, then
/// letters, digits, dots, underscores and hyphens, never ending in a dot,
/// which Windows would drop.
fn is_program_name(name: &str) -> bool {
    name.chars().count() <= MAX_PROGRAM_NAME_CHARACTERS
        && name
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphanumeric())
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
        && !name.ends_with('.')
}

#[cfg(windows)]
fn program_file_name(name: &str) -> String {
    format!("{name}.exe")
}

#[cfg(not(windows))]
fn program_file_name(name: &str) -> String {
    name.to_owned()
}

/// The program in one entry of the `PATH`, when the entry and the program
/// keep the rules of [`find_program`].
fn program_in(entry: &Path, file_name: &str, excluded: &[PathBuf]) -> Option<PathBuf> {
    if !entry.is_absolute() {
        return None;
    }
    paths::check_text(entry.to_str()?).ok()?;
    let resolved = paths::resolve(&entry.join(file_name), FinalLink::Follow).ok()?;
    let metadata = resolved.metadata.as_ref()?;
    if !metadata.is_file() || !is_runnable(&resolved.path, metadata) {
        return None;
    }
    if excluded
        .iter()
        .any(|folder| paths::is_inside(&resolved.path, folder))
    {
        return None;
    }
    Some(resolved.path)
}

/// Whether the file that a name resolved to runs as a program. On Windows a
/// link named `git.exe` could lead to a script, so the target's own name
/// decides.
#[cfg(windows)]
fn is_runnable(path: &Path, _metadata: &Metadata) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
}

#[cfg(unix)]
fn is_runnable(_path: &Path, metadata: &Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

/// A local server's program, as the core found it (ADR-0043, decision 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerProgram {
    /// Its canonical path, with its links resolved.
    pub path: PathBuf,
    /// Whether it lies inside one of the owner's projects, where a model's
    /// changes could replace it (T-63).
    pub in_project: bool,
}

/// Why a local server's program was not found or is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ServerProgramError {
    /// No program with this name, or no file at this path, runs as a
    /// program.
    #[error("the program was not found")]
    NotFound,
    /// The program is a relative path, a path on a network share, a script
    /// of another kind, or a file in Mannie's own folders.
    #[error("the program is not one that a server may run")]
    Refused,
}

/// Finds a local server's program (ADR-0043, decision 5): an absolute path
/// as the owner gave it, which may lie inside a project, or a name on a
/// `PATH` with the rules of [`find_program`], never inside a project or
/// Mannie's own folders. On Windows a server's program may also be a
/// `.cmd` or `.bat` file, such as `npx.cmd`, which the standard library
/// starts through `cmd.exe` and refuses arguments that `cmd.exe` would read
/// as code; a name finds the `.exe` file first.
pub fn find_server_program(
    program: &str,
    path_variable: &OsStr,
    projects: &[PathBuf],
    protected: &[PathBuf],
) -> Result<ServerProgram, ServerProgramError> {
    let given = Path::new(program);
    if given.is_absolute() {
        paths::check_text(program).map_err(|_refusal| ServerProgramError::Refused)?;
        let resolved = paths::resolve(given, FinalLink::Follow)
            .map_err(|_refusal| ServerProgramError::NotFound)?;
        let metadata = resolved
            .metadata
            .as_ref()
            .ok_or(ServerProgramError::NotFound)?;
        if !metadata.is_file() {
            return Err(ServerProgramError::NotFound);
        }
        if !is_server_runnable(&resolved.path, metadata)
            || protected
                .iter()
                .any(|folder| paths::is_inside(&resolved.path, folder))
        {
            return Err(ServerProgramError::Refused);
        }
        let in_project = projects
            .iter()
            .any(|folder| paths::is_inside(&resolved.path, folder));
        return Ok(ServerProgram {
            path: resolved.path,
            in_project,
        });
    }
    if program.contains(['/', '\\']) {
        return Err(ServerProgramError::Refused);
    }
    if !is_program_name(program) {
        return Err(ServerProgramError::NotFound);
    }
    let excluded: Vec<PathBuf> = projects.iter().chain(protected).cloned().collect();
    let found = server_file_names(program)
        .into_iter()
        .find_map(|file_name| {
            std::env::split_paths(path_variable)
                .find_map(|entry| server_program_in(&entry, &file_name, &excluded))
        });
    found
        .map(|path| ServerProgram {
            path,
            in_project: false,
        })
        .ok_or(ServerProgramError::NotFound)
}

/// The files that a name of a server's program may be, in the order that
/// they are tried: the name as it is when it ends with a kind that a server
/// may run, such as `npx.cmd`, and otherwise the name with each kind.
#[cfg(windows)]
fn server_file_names(name: &str) -> Vec<String> {
    let kinds = ["exe", "cmd", "bat"];
    let has_kind = Path::new(name).extension().is_some_and(|extension| {
        kinds
            .iter()
            .any(|kind| extension.eq_ignore_ascii_case(kind))
    });
    if has_kind {
        return vec![name.to_owned()];
    }
    kinds.iter().map(|kind| format!("{name}.{kind}")).collect()
}

#[cfg(not(windows))]
fn server_file_names(name: &str) -> Vec<String> {
    vec![name.to_owned()]
}

/// A server's program in one entry of the `PATH`, with the rules of
/// [`program_in`] and the files that a server may run.
fn server_program_in(entry: &Path, file_name: &str, excluded: &[PathBuf]) -> Option<PathBuf> {
    if !entry.is_absolute() {
        return None;
    }
    paths::check_text(entry.to_str()?).ok()?;
    let resolved = paths::resolve(&entry.join(file_name), FinalLink::Follow).ok()?;
    let metadata = resolved.metadata.as_ref()?;
    if !metadata.is_file() || !is_server_runnable(&resolved.path, metadata) {
        return None;
    }
    if excluded
        .iter()
        .any(|folder| paths::is_inside(&resolved.path, folder))
    {
        return None;
    }
    Some(resolved.path)
}

/// Whether a file runs as a server's program. On Windows the target's own
/// name decides: an `.exe`, `.cmd` or `.bat` file, never a `.ps1` script or
/// a file of any other kind.
#[cfg(windows)]
fn is_server_runnable(path: &Path, _metadata: &Metadata) -> bool {
    path.extension().is_some_and(|extension| {
        ["exe", "cmd", "bat"]
            .iter()
            .any(|kind| extension.eq_ignore_ascii_case(kind))
    })
}

#[cfg(unix)]
fn is_server_runnable(path: &Path, metadata: &Metadata) -> bool {
    is_runnable(path, metadata)
}

/// A run of an installed program.
#[derive(Clone)]
pub struct ProgramRun {
    /// The program's canonical path, as [`find_program`] found it.
    pub program: PathBuf,
    /// The arguments, which reach the program exactly.
    pub arguments: Vec<String>,
    /// The canonical working directory, which exists.
    pub working_directory: PathBuf,
    /// The variables that the program receives, and no others.
    pub environment: Vec<(OsString, OsString)>,
    /// The program's standard input, at most [`MAX_PROGRAM_INPUT_BYTES`];
    /// it closes once written, and at once when it is empty.
    pub input: Vec<u8>,
    /// How long the run may last, at most [`MAX_PROGRAM_TIMEOUT`].
    pub timeout: Duration,
    /// The bytes of standard output that the run keeps, at most
    /// [`MAX_PROGRAM_OUTPUT_BYTES`]. Output past it ends the run.
    pub output_limit: usize,
}

impl fmt::Debug for ProgramRun {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The names of the variables only, and the size of the input: values
        // and content stay out of logs.
        formatter
            .debug_struct("ProgramRun")
            .field("program", &self.program)
            .field("arguments", &self.arguments)
            .field("working_directory", &self.working_directory)
            .field(
                "environment",
                &self
                    .environment
                    .iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>(),
            )
            .field("input_bytes", &self.input.len())
            .field("timeout", &self.timeout)
            .field("output_limit", &self.output_limit)
            .finish()
    }
}

/// How a program's run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramEnding {
    /// The program exited on its own.
    Exited,
    /// Its timeout passed, and its tree was ended.
    TimedOut,
    /// Its output went past the run's limit, and its tree was ended.
    OutputFull,
}

/// What a program's run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramOutput {
    /// How it ended.
    pub ending: ProgramEnding,
    /// The program's exit code, when it exited on its own with one.
    pub exit_code: Option<i32>,
    /// How long it ran.
    pub duration: Duration,
    /// Its standard output, at most the run's limit.
    pub output: Vec<u8>,
    /// The bytes that it wrote to standard output in all.
    pub output_total: u64,
    /// Its error output, at most [`PROGRAM_ERROR_BYTES`].
    pub errors: Vec<u8>,
    /// The bytes that it wrote to error output in all.
    pub errors_total: u64,
}

impl ProgramOutput {
    /// Whether the run kept less standard output than the program wrote.
    pub fn output_cut(&self) -> bool {
        u64::try_from(self.output.len()).unwrap_or(u64::MAX) < self.output_total
    }
}

/// Runs an installed program without a shell, contained like a command
/// (ADR-0038, decisions 7 and 8). Dropping the future ends the program's
/// whole tree. A run past the limits of programs fails with
/// [`io::ErrorKind::InvalidInput`] before anything starts.
pub async fn run_program(run: ProgramRun) -> io::Result<ProgramOutput> {
    if run.input.len() > MAX_PROGRAM_INPUT_BYTES
        || run.output_limit > MAX_PROGRAM_OUTPUT_BYTES
        || run.timeout > MAX_PROGRAM_TIMEOUT
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the run passes the limits of programs",
        ));
    }
    crate::supervisor::run_program(run).await
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{ProgramNotFound, find_program, is_program_name, program_file_name};

    /// Writes a program file that the resolver accepts, executable on Unix.
    fn plant(folder: &Path, name: &str) -> PathBuf {
        let path = folder.join(name);
        fs::write(&path, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn canonical(path: &Path) -> PathBuf {
        PathBuf::from(crate::canonical_folder(path).unwrap())
    }

    fn path_of(entries: &[&Path]) -> OsString {
        std::env::join_paths(entries).unwrap()
    }

    #[test]
    fn plain_names_pass_and_others_are_refused() {
        for name in ["git", "git-lfs", "python3.12", "a_b"] {
            assert!(is_program_name(name), "{name}");
        }
        for name in [
            "",
            ".git",
            "-git",
            "git.",
            "bin/git",
            r"bin\git",
            "git exe",
            "gi\u{e9}t",
        ] {
            assert!(!is_program_name(name), "{name}");
        }
        assert_eq!(
            find_program("../git", &OsString::new(), &[]),
            Err(ProgramNotFound::InvalidName {
                name: "../git".to_owned()
            })
        );
    }

    #[test]
    fn the_first_entry_that_holds_the_program_wins() {
        let root = tempfile::tempdir().unwrap();
        let empty = root.path().join("empty");
        let first = root.path().join("first");
        let second = root.path().join("second");
        for folder in [&empty, &first, &second] {
            fs::create_dir_all(folder).unwrap();
        }
        plant(&empty, "unrelated");
        let program = plant(&first, &program_file_name("tool"));
        plant(&second, &program_file_name("tool"));

        let found = find_program("tool", &path_of(&[&empty, &first, &second]), &[]).unwrap();

        assert_eq!(found, canonical(&first).join(program.file_name().unwrap()));
    }

    #[test]
    fn a_program_inside_an_excluded_folder_is_skipped() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let system = root.path().join("system");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&system).unwrap();
        plant(&project, &program_file_name("tool"));
        plant(&system, &program_file_name("tool"));

        let found = find_program(
            "tool",
            &path_of(&[&project, &system]),
            &[canonical(&project)],
        )
        .unwrap();

        assert!(found.starts_with(canonical(&system)), "{}", found.display());
    }

    #[test]
    fn relative_entries_are_skipped() {
        let root = tempfile::tempdir().unwrap();
        plant(root.path(), &program_file_name("tool"));
        let relative = Path::new("relative-entry");

        assert_eq!(
            find_program("tool", &path_of(&[relative]), &[]),
            Err(ProgramNotFound::NotOnPath {
                name: "tool".to_owned()
            })
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_finds_only_exe_files_and_skips_unc_entries() {
        let root = tempfile::tempdir().unwrap();
        let scripts = root.path().join("scripts");
        let programs = root.path().join("programs");
        fs::create_dir_all(&scripts).unwrap();
        fs::create_dir_all(&programs).unwrap();
        plant(&scripts, "tool.cmd");
        plant(&scripts, "tool.bat");
        plant(&programs, "tool.exe");
        let unc = Path::new(r"\\server\share\bin");

        let found = find_program("tool", &path_of(&[unc, &scripts, &programs]), &[]).unwrap();

        assert_eq!(found, canonical(&programs).join("tool.exe"));
    }

    #[cfg(unix)]
    #[test]
    fn unix_skips_files_that_do_not_run() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let plain = root.path().join("plain");
        let runnable = root.path().join("runnable");
        fs::create_dir_all(&plain).unwrap();
        fs::create_dir_all(&runnable).unwrap();
        let data = plain.join("tool");
        fs::write(&data, b"").unwrap();
        fs::set_permissions(&data, fs::Permissions::from_mode(0o644)).unwrap();
        plant(&runnable, "tool");

        let found = find_program("tool", &path_of(&[&plain, &runnable]), &[]).unwrap();

        assert_eq!(found, canonical(&runnable).join("tool"));
    }
}
