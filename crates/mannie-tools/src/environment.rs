//! The environment of commands (docs/features/system-tools/commands.md,
//! environment). A command receives only the variables of an allow list and
//! the names that the owner added, with their values from Mannie's own
//! environment, so a credential in Mannie's environment never reaches a
//! command by mistake.

use std::ffi::{OsStr, OsString};
use std::fmt;

use mannie_types::tools::EnvName;

/// The variables that every command receives on Windows. `PSModulePath` is
/// left out, so Windows PowerShell finds its own modules even when Mannie was
/// started from PowerShell 7.
#[cfg(windows)]
const ALLOWED: [&str; 28] = [
    "ALLUSERSPROFILE",
    "APPDATA",
    "CommonProgramFiles",
    "CommonProgramFiles(x86)",
    "CommonProgramW6432",
    "COMPUTERNAME",
    "ComSpec",
    "HOMEDRIVE",
    "HOMEPATH",
    "LOCALAPPDATA",
    "NUMBER_OF_PROCESSORS",
    "OS",
    "PATH",
    "PATHEXT",
    "PROCESSOR_ARCHITECTURE",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "PUBLIC",
    "SystemDrive",
    "SystemRoot",
    "TEMP",
    "TMP",
    "USERDOMAIN",
    "USERNAME",
    "USERPROFILE",
    "windir",
];

/// The variables that every command receives on Unix.
#[cfg(not(windows))]
const ALLOWED: [&str; 12] = [
    "DBUS_SESSION_BUS_ADDRESS",
    "HOME",
    "LANG",
    "LANGUAGE",
    "LOGNAME",
    "PATH",
    "SHELL",
    "SSH_AUTH_SOCK",
    "TMPDIR",
    "TZ",
    "USER",
    "XDG_RUNTIME_DIR",
];

/// The prefix of the locale variables, which every command receives on Unix.
#[cfg(not(windows))]
const LOCALE_PREFIX: Option<&str> = Some("LC_");
#[cfg(windows)]
const LOCALE_PREFIX: Option<&str> = None;

/// The variables a command receives.
#[derive(Clone, Default)]
pub struct CommandEnvironment {
    variables: Vec<(OsString, OsString)>,
}

impl fmt::Debug for CommandEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The names only: values stay out of logs.
        formatter
            .debug_list()
            .entries(self.variables.iter().map(|(name, _)| name))
            .finish()
    }
}

impl CommandEnvironment {
    /// The allowed variables of `source`, and those that the owner added.
    pub fn new(extra: &[EnvName], source: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        let variables = source
            .into_iter()
            .filter(|(name, _)| is_allowed(name, extra))
            .collect();
        Self { variables }
    }

    /// The allowed variables of Mannie's own environment.
    pub fn from_process(extra: &[EnvName]) -> Self {
        Self::new(extra, std::env::vars_os())
    }

    /// The variables, with their values.
    #[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
    pub(crate) fn variables(&self) -> &[(OsString, OsString)] {
        &self.variables
    }

    /// The names of the variables.
    pub fn names(&self) -> impl Iterator<Item = &OsStr> {
        self.variables.iter().map(|(name, _)| name.as_os_str())
    }
}

/// Whether a command receives a variable. Windows compares the names of
/// variables without regard to case.
fn is_allowed(name: &OsStr, extra: &[EnvName]) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let same = |allowed: &str| {
        if cfg!(windows) {
            allowed.eq_ignore_ascii_case(name)
        } else {
            allowed == name
        }
    };
    ALLOWED.iter().any(|allowed| same(allowed))
        || LOCALE_PREFIX.is_some_and(|prefix| name.starts_with(prefix))
        || extra.iter().any(|added| same(added.as_str()))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use mannie_types::tools::EnvName;

    use super::CommandEnvironment;

    fn variables(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect()
    }

    #[test]
    fn only_allowed_and_added_names_pass() {
        let source = variables(&[
            ("PATH", "/usr/bin"),
            ("OPENAI_API_KEY", "sk-planted"),
            ("MANNIE_SECRET", "planted"),
            ("RUST_LOG", "debug"),
            ("CARGO_HOME", "/cargo"),
        ]);
        let added = [EnvName::parse("CARGO_HOME").unwrap()];

        let environment = CommandEnvironment::new(&added, source);

        let names: Vec<_> = environment.names().collect();
        assert_eq!(names, ["PATH", "CARGO_HOME"]);
        assert!(!format!("{environment:?}").contains("/usr/bin"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_compares_names_without_regard_to_case() {
        let source = variables(&[("Path", "C:\\Windows"), ("psmodulepath", "C:\\modules")]);

        let environment = CommandEnvironment::new(&[], source);

        assert_eq!(environment.names().collect::<Vec<_>>(), ["Path"]);
    }

    #[cfg(unix)]
    #[test]
    fn unix_passes_the_locale_variables() {
        let source = variables(&[("LC_ALL", "C.UTF-8"), ("LD_PRELOAD", "/tmp/evil.so")]);

        let environment = CommandEnvironment::new(&[], source);

        assert_eq!(environment.names().collect::<Vec<_>>(), ["LC_ALL"]);
    }
}
