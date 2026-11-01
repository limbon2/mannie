//! The shells of this machine that a terminal runs (ADR-0053, decision 2),
//! and how each one runs a setup after the owner's own start-up files
//! (decision 1).
//!
//! Only listed shells start. On Windows they come from the system folder,
//! from the folders where PowerShell 7 and Git install themselves, and from
//! absolute entries of Mannie's `PATH` that the caller accepts. On Linux
//! they are the account's login shell and the existing entries of
//! `/etc/shells`.
//!
//! A setup reaches each shell through the shell's own means: a start-up
//! file that first reads the owner's files for bash, zsh and `sh`, an
//! initial command for fish, the command that PowerShell runs before its
//! prompt, and the prompt variable of `cmd.exe`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The kind of a shell, which decides how a setup reaches it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShellKind {
    /// GNU bash, also Git's bash on Windows.
    Bash,
    /// zsh.
    Zsh,
    /// fish.
    Fish,
    /// A POSIX `sh`, such as dash or FreeBSD's `sh`.
    Sh,
    /// Windows PowerShell or PowerShell 7.
    PowerShell,
    /// Windows' `cmd.exe`.
    Cmd,
    /// Any other shell, which a setup does not reach.
    Other,
}

impl ShellKind {
    /// The kind of the shell whose program has this path, by its file name.
    pub fn of(program: &Path) -> Self {
        let name = program
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match name.as_str() {
            "bash" => Self::Bash,
            "zsh" => Self::Zsh,
            "fish" => Self::Fish,
            "sh" | "dash" | "ash" | "ksh" | "mksh" => Self::Sh,
            "powershell" | "pwsh" => Self::PowerShell,
            "cmd" => Self::Cmd,
            _ => Self::Other,
        }
    }
}

/// A shell that a terminal can run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalShell {
    /// The canonical path of the shell's program.
    pub program: PathBuf,
    /// The shell's name, such as `Windows PowerShell` or `bash`.
    pub name: String,
    /// Its kind.
    pub kind: ShellKind,
    /// Whether a terminal runs it when no shell is named.
    pub default: bool,
}

impl LocalShell {
    fn new(program: PathBuf, name: impl Into<String>) -> Self {
        let kind = ShellKind::of(&program);
        Self {
            program,
            name: name.into(),
            kind,
            default: false,
        }
    }
}

/// The most bytes of a setup.
pub const MAX_SETUP_BYTES: usize = 16 * 1024;

/// How a shell starts with a setup: the files to write into the terminal's
/// own folder, the shell's arguments and the variables to add.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShellStart {
    /// Files to write before the shell starts, by name inside the folder.
    pub files: Vec<(String, String)>,
    /// The shell's arguments.
    pub arguments: Vec<OsString>,
    /// Variables to add to the shell's environment.
    pub environment: Vec<(OsString, OsString)>,
}

/// Why a setup cannot reach its shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SetupRefused {
    /// The setup is larger than [`MAX_SETUP_BYTES`], holds NUL, or holds a
    /// character that its shell's way of taking it cannot carry: a double
    /// quote or a line break for PowerShell's command, a line break for the
    /// prompt of `cmd.exe`.
    #[error("the setup cannot be given to this shell")]
    Refused,
}

/// How `shell` starts in a terminal whose own folder is `folder`, running
/// `setup` after the owner's start-up files. `owner` reads a variable of the
/// owner's environment, such as `ZDOTDIR`.
pub fn shell_start(
    shell: &LocalShell,
    setup: Option<&str>,
    folder: &Path,
    owner: &dyn Fn(&str) -> Option<OsString>,
) -> Result<ShellStart, SetupRefused> {
    if let Some(text) = setup
        && (text.len() > MAX_SETUP_BYTES || text.contains('\0'))
    {
        return Err(SetupRefused::Refused);
    }
    let file = |name: &str| path_argument(&folder.join(name), shell.kind);
    let mut start = ShellStart::default();
    match (shell.kind, setup) {
        (ShellKind::Bash, Some(text)) => {
            // A login shell on Windows, as Git's bash starts, reads the
            // system's profile, which sets its `PATH`; Linux's terminals
            // start interactive shells that read `~/.bashrc`.
            let owner_files = if cfg!(windows) {
                "[ -f /etc/profile ] && . /etc/profile\n\
                 for __mannie_profile in ~/.bash_profile ~/.bash_login ~/.profile; do\n\
                 \x20 [ -f \"$__mannie_profile\" ] && { . \"$__mannie_profile\"; break; }\n\
                 done\n\
                 unset __mannie_profile\n"
            } else {
                "[ -f ~/.bashrc ] && . ~/.bashrc\n"
            };
            start
                .files
                .push(("bashrc".to_owned(), format!("{owner_files}{text}\n")));
            start.arguments = vec!["--rcfile".into(), file("bashrc"), "-i".into()];
        }
        (ShellKind::Zsh, Some(text)) => {
            // zsh reads its start-up files from `ZDOTDIR`, which points at
            // the terminal's folder; its files read the owner's from where
            // they were, and give `ZDOTDIR` back.
            let home = "${MANNIE_OWNER_ZDOTDIR:-$HOME}";
            start.files.push((
                ".zshenv".to_owned(),
                format!("[ -f \"{home}/.zshenv\" ] && . \"{home}/.zshenv\"\n"),
            ));
            start.files.push((
                ".zshrc".to_owned(),
                format!(
                    "ZDOTDIR=\"{home}\"\n\
                     unset MANNIE_OWNER_ZDOTDIR\n\
                     [ -f \"$ZDOTDIR/.zshrc\" ] && . \"$ZDOTDIR/.zshrc\"\n\
                     {text}\n"
                ),
            ));
            start
                .environment
                .push(("ZDOTDIR".into(), folder.as_os_str().to_owned()));
            if let Some(owner_zdotdir) = owner("ZDOTDIR") {
                start
                    .environment
                    .push(("MANNIE_OWNER_ZDOTDIR".into(), owner_zdotdir));
            }
            start.arguments = vec!["-i".into()];
        }
        (ShellKind::Fish, Some(text)) => {
            // fish reads its own configuration first, then the command.
            start
                .files
                .push(("setup.fish".to_owned(), format!("{text}\n")));
            let mut command = OsString::from("source ");
            command.push(quoted_for_fish(&folder.join("setup.fish")));
            start.arguments = vec!["--init-command".into(), command];
        }
        (ShellKind::Sh, Some(text)) => {
            // An interactive `sh` reads the file that `ENV` names; the file
            // reads the owner's first.
            start.files.push((
                "env.sh".to_owned(),
                format!(
                    "[ -n \"$MANNIE_OWNER_ENV\" ] && [ -f \"$MANNIE_OWNER_ENV\" ] && . \"$MANNIE_OWNER_ENV\"\n\
                     unset MANNIE_OWNER_ENV\n\
                     {text}\n"
                ),
            ));
            start.environment.push(("ENV".into(), file("env.sh")));
            if let Some(owner_env) = owner("ENV") {
                start
                    .environment
                    .push(("MANNIE_OWNER_ENV".into(), owner_env));
            }
            start.arguments = vec!["-i".into()];
        }
        (ShellKind::PowerShell, Some(text)) => {
            // The command runs after the owner's profile, before the first
            // prompt. A script file would meet the execution policy, which
            // refuses scripts by default; a command does not.
            if text.contains(['"', '\r', '\n']) {
                return Err(SetupRefused::Refused);
            }
            start.arguments = vec![
                "-NoLogo".into(),
                "-NoExit".into(),
                "-Command".into(),
                text.into(),
            ];
        }
        (ShellKind::Cmd, Some(text)) => {
            if text.contains(['\r', '\n']) {
                return Err(SetupRefused::Refused);
            }
            start.environment.push(("PROMPT".into(), text.into()));
        }
        (ShellKind::Bash | ShellKind::Zsh | ShellKind::Sh, None) => {
            start.arguments = vec!["-i".into()];
        }
        (ShellKind::Fish | ShellKind::Cmd | ShellKind::Other, None)
        | (ShellKind::Other, Some(_)) => {}
        (ShellKind::PowerShell, None) => {
            start.arguments = vec!["-NoLogo".into()];
        }
    }
    Ok(start)
}

/// A file's path as an argument of a shell: with forward slashes for Git's
/// bash on Windows, whose runtime reads them as the same path.
fn path_argument(path: &Path, kind: ShellKind) -> OsString {
    if cfg!(windows) && kind == ShellKind::Bash {
        return path.to_string_lossy().replace('\\', "/").into();
    }
    path.as_os_str().to_owned()
}

/// A path in fish's single quotes, where only `\` and `'` need escapes.
fn quoted_for_fish(path: &Path) -> OsString {
    let text = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('\'', "\\'");
    format!("'{text}'").into()
}

/// The shells of this machine, the default first. `accept` decides whether
/// a program found on `PATH` may run, such as one outside the owner's
/// projects and Mannie's own folders.
pub fn local_shells(accept: &dyn Fn(&Path) -> bool) -> Vec<LocalShell> {
    let mut shells = platform_shells(accept);
    let mut seen: Vec<PathBuf> = Vec::new();
    shells.retain(|shell| {
        let fresh = !seen.contains(&shell.program);
        seen.push(shell.program.clone());
        fresh
    });
    if let Some(first) = shells.first_mut() {
        first.default = true;
    }
    shells
}

#[cfg(windows)]
fn platform_shells(accept: &dyn Fn(&Path) -> bool) -> Vec<LocalShell> {
    let mut shells = Vec::new();
    let installed = |relative: &str| -> Option<PathBuf> {
        ["ProgramW6432", "ProgramFiles"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|folder| PathBuf::from(folder).join(relative))
            .find(|path| path.is_file())
            .and_then(|path| dunce_canonical(&path))
    };
    let on_path = |name: &str| -> Option<PathBuf> {
        std::env::split_paths(&std::env::var_os("PATH")?)
            .filter(|folder| folder.is_absolute())
            .map(|folder| folder.join(name))
            .filter(|path| path.is_file())
            .filter_map(|path| dunce_canonical(&path))
            .find(|path| accept(path))
    };
    let pwsh = installed(r"PowerShell\7\pwsh.exe").or_else(|| on_path("pwsh.exe"));
    if let Some(program) = pwsh {
        shells.push(LocalShell::new(program, "PowerShell 7"));
    }
    if let Ok(system) = crate::supervisor::system_folder() {
        let windows_powershell = system.join(r"WindowsPowerShell\v1.0\powershell.exe");
        if windows_powershell.is_file() {
            shells.push(LocalShell::new(windows_powershell, "Windows PowerShell"));
        }
        let cmd = system.join("cmd.exe");
        if cmd.is_file() {
            shells.push(LocalShell::new(cmd, "Command Prompt"));
        }
    }
    if let Some(program) = installed(r"Git\bin\bash.exe") {
        shells.push(LocalShell::new(program, "Git Bash"));
    }
    shells
}

#[cfg(windows)]
fn dunce_canonical(path: &Path) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(path).ok()?;
    // `canonicalize` gives a verbatim path, `\\?\C:\...`, which some
    // programs mishandle as their own path; the plain form names the same
    // file.
    let text = canonical.to_string_lossy();
    Some(match text.strip_prefix(r"\\?\") {
        Some(plain) if !plain.starts_with("UNC\\") => PathBuf::from(plain),
        _ => canonical,
    })
}

#[cfg(not(windows))]
fn platform_shells(_accept: &dyn Fn(&Path) -> bool) -> Vec<LocalShell> {
    let mut programs: Vec<PathBuf> = Vec::new();
    if let Some(login) = login_shell() {
        programs.push(login);
    }
    if let Ok(listed) = std::fs::read_to_string("/etc/shells") {
        programs.extend(
            listed
                .lines()
                .map(str::trim)
                .filter(|line| line.starts_with('/'))
                .map(PathBuf::from),
        );
    }
    programs
        .into_iter()
        .filter(|program| !is_no_login(program))
        .filter_map(|program| {
            std::fs::canonicalize(&program)
                .ok()
                .map(|canonical| (program, canonical))
        })
        .filter(|(_, canonical)| canonical.is_file())
        .map(|(listed, canonical)| {
            let name = listed
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut shell = LocalShell::new(canonical, name);
            // `/bin/sh` may link to dash; its listed name decides its kind.
            shell.kind = ShellKind::of(&listed);
            shell
        })
        .collect()
}

/// The account's login shell: `SHELL` as the login set it, else the
/// account's entry in `/etc/passwd`.
#[cfg(not(windows))]
fn login_shell() -> Option<PathBuf> {
    if let Some(shell) = std::env::var_os("SHELL").map(PathBuf::from)
        && shell.is_absolute()
    {
        return Some(shell);
    }
    let uid = rustix::process::getuid().as_raw().to_string();
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        (fields.len() >= 7 && fields[2] == uid).then(|| PathBuf::from(fields[6]))
    })
}

#[cfg(not(windows))]
fn is_no_login(program: &Path) -> bool {
    program
        .file_name()
        .is_some_and(|name| ["nologin", "false", "true"].contains(&name.to_string_lossy().as_ref()))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use super::{LocalShell, SetupRefused, ShellKind, shell_start};

    fn shell(program: &str) -> LocalShell {
        LocalShell::new(PathBuf::from(program), "test")
    }

    fn no_owner(_name: &str) -> Option<OsString> {
        None
    }

    #[test]
    fn a_shell_s_kind_comes_from_its_program_name() {
        assert_eq!(ShellKind::of(Path::new("/bin/bash")), ShellKind::Bash);
        assert_eq!(
            ShellKind::of(Path::new(r"C:\Program Files\Git\bin\bash.exe")),
            ShellKind::Bash
        );
        assert_eq!(ShellKind::of(Path::new("/usr/bin/dash")), ShellKind::Sh);
        assert_eq!(
            ShellKind::of(Path::new(r"C:\Windows\System32\cmd.exe")),
            ShellKind::Cmd
        );
        assert_eq!(ShellKind::of(Path::new("pwsh.exe")), ShellKind::PowerShell);
        assert_eq!(ShellKind::of(Path::new("/usr/bin/tcsh")), ShellKind::Other);
    }

    #[test]
    fn bash_reads_the_owner_s_files_before_the_setup() {
        let folder = Path::new("/tmp/terminal-1");

        let start =
            shell_start(&shell("/bin/bash"), Some("setup-line"), folder, &no_owner).unwrap();

        let (name, text) = &start.files[0];
        assert_eq!(name, "bashrc");
        assert!(text.ends_with("setup-line\n"));
        let owner_file = if cfg!(windows) {
            "/etc/profile"
        } else {
            "~/.bashrc"
        };
        assert!(text.find(owner_file).unwrap() < text.find("setup-line").unwrap());
        assert_eq!(start.arguments[0], OsString::from("--rcfile"));
        assert_eq!(start.arguments[2], OsString::from("-i"));
    }

    #[test]
    fn zsh_gets_the_terminal_s_folder_and_gives_back_the_owner_s() {
        let owner = |name: &str| (name == "ZDOTDIR").then(|| OsString::from("/home/o/.config/zsh"));

        let start = shell_start(
            &shell("/bin/zsh"),
            Some("setup-line"),
            Path::new("/tmp/t"),
            &owner,
        )
        .unwrap();

        assert!(
            start
                .environment
                .contains(&("ZDOTDIR".into(), "/tmp/t".into()))
        );
        assert!(
            start
                .environment
                .contains(&("MANNIE_OWNER_ZDOTDIR".into(), "/home/o/.config/zsh".into()))
        );
        let names: Vec<&str> = start.files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, [".zshenv", ".zshrc"]);
    }

    #[test]
    fn powershell_takes_its_setup_as_a_command_on_one_line_without_double_quotes() {
        let powershell = shell(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");

        let start =
            shell_start(&powershell, Some("$a = 'b'"), Path::new("C:/t"), &no_owner).unwrap();

        assert_eq!(start.arguments.last(), Some(&OsString::from("$a = 'b'")));
        assert!(start.arguments.contains(&OsString::from("-NoExit")));
        assert_eq!(
            shell_start(
                &powershell,
                Some("say \"hi\""),
                Path::new("C:/t"),
                &no_owner
            ),
            Err(SetupRefused::Refused)
        );
        assert_eq!(
            shell_start(&powershell, Some("a\nb"), Path::new("C:/t"), &no_owner),
            Err(SetupRefused::Refused)
        );
    }

    #[test]
    fn cmd_takes_its_setup_as_its_prompt() {
        let start = shell_start(
            &shell("cmd.exe"),
            Some("$P$G"),
            Path::new("C:/t"),
            &no_owner,
        )
        .unwrap();

        assert_eq!(start.environment, vec![("PROMPT".into(), "$P$G".into())]);
        assert_eq!(start.arguments, Vec::<OsString>::new());
    }

    #[test]
    fn a_setup_past_its_limit_or_with_nul_is_refused() {
        let long = "x".repeat(super::MAX_SETUP_BYTES + 1);

        assert_eq!(
            shell_start(&shell("/bin/bash"), Some(&long), Path::new("/t"), &no_owner),
            Err(SetupRefused::Refused)
        );
        assert_eq!(
            shell_start(
                &shell("/bin/bash"),
                Some("a\0b"),
                Path::new("/t"),
                &no_owner
            ),
            Err(SetupRefused::Refused)
        );
    }
}
