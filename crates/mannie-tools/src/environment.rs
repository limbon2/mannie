//! The environment of commands (docs/features/system-tools/commands.md,
//! environment). A command receives only the variables of an allow list and
//! the names that the owner added, with their values from Mannie's own
//! environment, so a credential in Mannie's environment never reaches a
//! command by mistake.
//!
//! An installed program that a plugin runs receives the same variables, the
//! names that its plugin's manifest inherits when Mannie's environment has
//! them, and the variables that the plugin sets (ADR-0038, decision 7).

use std::ffi::{OsStr, OsString};
use std::fmt;

use mannie_types::tools::{EnvName, ToolsError, is_loader_name};

/// The variables that every command receives on Windows. `PSModulePath` is
/// left out, so Windows PowerShell finds its own modules even when Mannie was
/// started from PowerShell 7. `PSModuleAnalysisCachePath` passes, so Windows
/// PowerShell reads the module cache that the system prepared instead of
/// analyzing every installed module again for each command.
#[cfg(windows)]
const ALLOWED: [&str; 29] = [
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
    "PSModuleAnalysisCachePath",
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
    pub fn variables(&self) -> &[(OsString, OsString)] {
        &self.variables
    }

    /// The names of the variables.
    pub fn names(&self) -> impl Iterator<Item = &OsStr> {
        self.variables.iter().map(|(name, _)| name.as_os_str())
    }

    /// The environment of an installed program: these variables, then each
    /// name of `inherit` that `source` has, then the variables that `set`
    /// gives, which replace a variable of the same name. A name that looks
    /// like a secret or loads libraries into programs is refused, and so is a
    /// set name of the allow list, which every command receives as it is.
    pub fn for_program(
        &self,
        inherit: &[String],
        set: &[(String, String)],
        source: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Vec<(OsString, OsString)>, ProgramEnvironmentError> {
        for name in inherit {
            check_program_name(name)?;
        }
        for (name, value) in set {
            check_program_name(name)?;
            if is_allowed(OsStr::new(name), &[]) {
                return Err(ProgramEnvironmentError::Reserved { name: name.clone() });
            }
            if value.contains('\0') {
                return Err(ProgramEnvironmentError::Value { name: name.clone() });
            }
        }
        let mut variables = self.variables.clone();
        for (name, value) in source {
            let inherited = name
                .to_str()
                .is_some_and(|name| inherit.iter().any(|wanted| same_name(wanted, name)));
            if inherited
                && !variables
                    .iter()
                    .any(|(present, _)| same_os_name(present, &name))
            {
                variables.push((name, value));
            }
        }
        for (name, value) in set {
            variables.retain(|(present, _)| !same_os_name(present, OsStr::new(name)));
            variables.push((OsString::from(name), OsString::from(value)));
        }
        Ok(variables)
    }
}

/// Why the environment of an installed program could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProgramEnvironmentError {
    /// The name is not the name of a variable.
    #[error("{name:?} is not the name of a variable")]
    Invalid {
        /// The name, shortened.
        name: String,
    },
    /// The name looks like a secret's, which never reaches a child process.
    #[error("the variable {name} looks like a secret")]
    Secret {
        /// The name.
        name: String,
    },
    /// The name makes programs load libraries, such as `LD_PRELOAD`.
    #[error("the variable {name} loads libraries into programs")]
    Loader {
        /// The name.
        name: String,
    },
    /// A plugin may not set a variable that every command receives.
    #[error("the variable {name} comes from Mannie's environment")]
    Reserved {
        /// The name.
        name: String,
    },
    /// The value holds a NUL character, which no environment can carry.
    #[error("the value of {name} holds a NUL character")]
    Value {
        /// The name.
        name: String,
    },
}

/// Checks a name of a program's environment with the rules of the names that
/// the owner adds, and refuses the names that load libraries.
fn check_program_name(name: &str) -> Result<(), ProgramEnvironmentError> {
    EnvName::parse(name).map_err(|error| match error {
        ToolsError::EnvironmentNameSecret { name } => ProgramEnvironmentError::Secret { name },
        _ => ProgramEnvironmentError::Invalid {
            name: name.chars().take(64).collect(),
        },
    })?;
    if is_loader_name(name) {
        return Err(ProgramEnvironmentError::Loader {
            name: name.to_owned(),
        });
    }
    Ok(())
}

/// Whether two names of variables are the same. Windows compares them
/// without regard to case.
fn same_name(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn same_os_name(left: &OsStr, right: &OsStr) -> bool {
    match (left.to_str(), right.to_str()) {
        (Some(left), Some(right)) => same_name(left, right),
        _ => left == right,
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

    #[cfg(windows)]
    #[test]
    fn windows_passes_the_module_analysis_cache_and_not_the_module_path() {
        let source = variables(&[
            (
                "PSModuleAnalysisCachePath",
                "C:\\cache\\ModuleAnalysisCache",
            ),
            ("PSModulePath", "C:\\modules"),
        ]);

        let environment = CommandEnvironment::new(&[], source);

        assert_eq!(
            environment.names().collect::<Vec<_>>(),
            ["PSModuleAnalysisCachePath"]
        );
    }

    #[test]
    fn a_program_receives_inherited_and_set_variables_over_the_allowed_ones() {
        let source = variables(&[
            ("PATH", "/usr/bin"),
            ("GIT_SSH_COMMAND", "ssh -i key"),
            ("GNUPGHOME", "/gnupg"),
            ("UNRELATED", "x"),
        ]);
        let environment = CommandEnvironment::new(&[], source.clone());

        let program = environment
            .for_program(
                &["GIT_SSH_COMMAND".to_owned(), "XDG_CONFIG_HOME".to_owned()],
                &[("GIT_TERMINAL_PROMPT".to_owned(), "0".to_owned())],
                source,
            )
            .unwrap();

        let names: Vec<_> = program
            .iter()
            .map(|(name, _)| name.to_str().unwrap())
            .collect();
        // An inherited name that Mannie's environment lacks stays absent.
        assert_eq!(names, ["PATH", "GIT_SSH_COMMAND", "GIT_TERMINAL_PROMPT"]);
    }

    #[test]
    fn a_program_s_environment_refuses_secrets_loaders_and_allowed_names() {
        use super::ProgramEnvironmentError;

        let environment = CommandEnvironment::new(&[], variables(&[("PATH", "/usr/bin")]));
        let set = |name: &str| {
            environment.for_program(&[], &[(name.to_owned(), "1".to_owned())], Vec::new())
        };

        assert!(matches!(
            set("GITHUB_TOKEN"),
            Err(ProgramEnvironmentError::Secret { .. })
        ));
        assert!(matches!(
            set("LD_PRELOAD"),
            Err(ProgramEnvironmentError::Loader { .. })
        ));
        assert!(matches!(
            set("DYLD_INSERT_LIBRARIES"),
            Err(ProgramEnvironmentError::Loader { .. })
        ));
        assert!(matches!(
            set("PATH"),
            Err(ProgramEnvironmentError::Reserved { .. })
        ));
        assert!(matches!(
            set("not a name"),
            Err(ProgramEnvironmentError::Invalid { .. })
        ));
        assert!(matches!(
            environment.for_program(&["AWS_SECRET_ACCESS_KEY".to_owned()], &[], Vec::new()),
            Err(ProgramEnvironmentError::Secret { .. })
        ));
        assert!(matches!(
            environment.for_program(
                &[],
                &[("GIT_EDITOR".to_owned(), "a\0b".to_owned())],
                Vec::new()
            ),
            Err(ProgramEnvironmentError::Value { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn unix_passes_the_locale_variables() {
        let source = variables(&[("LC_ALL", "C.UTF-8"), ("LD_PRELOAD", "/tmp/evil.so")]);

        let environment = CommandEnvironment::new(&[], source);

        assert_eq!(environment.names().collect::<Vec<_>>(), ["LC_ALL"]);
    }
}
