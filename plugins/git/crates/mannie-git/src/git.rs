//! Runs the owner's installed git through the service `processes`
//! (ADR-0038, decision 7; plugins/git/docs/adr/0001-run-the-owners-
//! installed-git.md). The core finds git, starts it without a shell with the
//! manifest's fixed arguments first, and ends its whole process tree when a
//! run is dropped. The plugin passes arguments exactly, never a shell's text.
//!
//! Every run sets variables that keep git from waiting for a person: no
//! prompt for credentials on a terminal, no editor, no pager, no edit of a
//! merge's message, and only the transports that the plugin allows, so a
//! remote such as `ext::` never starts a program (T-49). Hooks inherit them.
//!
//! The version is checked once, before the first run that succeeds: the
//! plugin needs git 2.40 or later.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mannie_plugin_sdk::contracts::ProjectId;
use mannie_plugin_sdk::services::processes::{
    ProcessError, ProcessesApi, ProgramEnding, ProgramRun, ReadPlace,
};
use tokio::sync::OnceCell;

use crate::failure::GitFailure;
use crate::redact::redact;
use crate::version::Version;

/// The program's name in the manifest.
pub(crate) const GIT: &str = "git";

/// The variables that every run sets.
const VARIABLES: [(&str, &str); 5] = [
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_EDITOR", ":"),
    ("GIT_SEQUENCE_EDITOR", ":"),
    ("GIT_PAGER", "cat"),
    ("GIT_MERGE_AUTOEDIT", "no"),
];

/// The protocols of a run that reaches a remote, as `GIT_ALLOW_PROTOCOL`
/// names them: never `ext`, which runs a command that the URL names (T-49).
const REMOTE_PROTOCOLS: &str = "file:git:http:https:ssh";

/// The protocols of every other run: a name that no URL gives its transport,
/// since a URL's scheme holds no underscore, so git refuses every remote. A
/// read or a local change in a partial clone then fails where git would
/// fetch a missing object from the promisor remote, which would start the
/// programs that the repository's configuration names for it. A remote's
/// `vcs` setting can name any helper, this one too, but git would then start
/// a program `git-remote-no_protocol`, which git does not ship.
const NO_PROTOCOL: &str = "no_protocol";

/// The time of a run that names none, in seconds.
const DEFAULT_TIMEOUT_SECONDS: u32 = 60;

/// The output that a run keeps unless it names another limit.
const DEFAULT_OUTPUT_LIMIT: u32 = 4 * 1024 * 1024;

/// One run of git.
#[derive(Debug, Clone)]
pub(crate) struct GitRun {
    folder: PathBuf,
    arguments: Vec<String>,
    input: Vec<u8>,
    variables: Vec<(String, String)>,
    timeout_seconds: u32,
    output_limit: u32,
    reads: Option<ReadPlace>,
    /// Whether the run may reach a remote, which only fetches, pulls and
    /// pushes do.
    remote: bool,
}

impl GitRun {
    /// A run in `folder` with `arguments`, after the fixed ones.
    pub(crate) fn new<S: Into<String>>(
        folder: impl Into<PathBuf>,
        arguments: impl IntoIterator<Item = S>,
    ) -> Self {
        Self {
            folder: folder.into(),
            arguments: arguments.into_iter().map(Into::into).collect(),
            input: Vec::new(),
            variables: Vec::new(),
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            output_limit: DEFAULT_OUTPUT_LIMIT,
            reads: None,
            remote: false,
        }
    }

    /// The same run, allowed to reach a remote, for a fetch, a pull or a push
    /// that the owner allowed with its endpoint.
    #[must_use]
    pub(crate) fn reaching_remote(mut self) -> Self {
        self.remote = true;
        self
    }

    /// The same run, with git's standard input.
    #[must_use]
    pub(crate) fn with_input(mut self, input: Vec<u8>) -> Self {
        self.input = input;
        self
    }

    /// The same run, with one more variable, such as an author's date.
    #[must_use]
    pub(crate) fn with_variable(mut self, name: &str, value: &str) -> Self {
        self.variables.push((name.to_owned(), value.to_owned()));
        self
    }

    /// The same run, with its time in seconds.
    #[must_use]
    pub(crate) fn with_timeout(mut self, seconds: u32) -> Self {
        self.timeout_seconds = seconds;
        self
    }

    /// The same run, keeping at most `bytes` of its output.
    #[must_use]
    pub(crate) fn with_output_limit(mut self, bytes: u32) -> Self {
        self.output_limit = bytes;
        self
    }

    /// The same run, checked as reads of `place` before it starts, for a
    /// command or a mention (ADR-0038, decision 10).
    #[must_use]
    pub(crate) fn checked_as(mut self, place: Option<ReadPlace>) -> Self {
        self.reads = place;
        self
    }
}

/// What a run of git produced.
#[derive(Debug, Clone)]
pub(crate) struct Ran {
    /// The standard output, up to the run's limit.
    pub(crate) output: Vec<u8>,
    /// Whether the output went past the limit, which ended the run.
    pub(crate) cut: bool,
    /// The error output, with credentials removed.
    pub(crate) errors: String,
    /// The exit code, when git exited on its own with one.
    pub(crate) exit_code: Option<i32>,
}

impl Ran {
    /// Whether git exited with 0.
    pub(crate) fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }

    /// The output as text, with any byte that is not UTF-8 replaced.
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.output).into_owned()
    }

    /// Why git failed, from its error output: a repository that another
    /// user owns names `safe.directory` in every language, and a lock file
    /// that exists names its path ending in `.lock`.
    pub(crate) fn failure(&self, folder: &Path) -> GitFailure {
        let folder = folder.display().to_string();
        if self.errors.contains("safe.directory") {
            return GitFailure::DubiousOwnership { folder };
        }
        if let Some(path) = locked_path(&self.errors) {
            return GitFailure::Locked { path };
        }
        if self.errors.contains("not a git repository") {
            return GitFailure::NotARepository { folder };
        }
        let message = if self.errors.trim().is_empty() {
            format!(
                "git ended with the exit code {:?} and no message",
                self.exit_code
            )
        } else {
            self.errors.clone()
        };
        GitFailure::failed(&message)
    }
}

/// The path of a lock file that git's message names in quotes, such as
/// `'D:/work/.git/index.lock'`.
fn locked_path(errors: &str) -> Option<String> {
    errors.split(['\'', '"', '`']).find_map(|part| {
        let path = Path::new(part);
        let has_name = path.file_stem().is_some_and(|stem| !stem.is_empty());
        let is_lock = path
            .extension()
            .is_some_and(|extension| extension == "lock")
            && has_name;
        is_lock.then(|| part.to_owned())
    })
}

/// The installed git, as the service `processes` runs it.
#[derive(Debug, Clone)]
pub(crate) struct Git {
    processes: Arc<dyn ProcessesApi>,
    version: Arc<OnceCell<Version>>,
}

impl Git {
    pub(crate) fn new(processes: Arc<dyn ProcessesApi>) -> Self {
        Self {
            processes,
            version: Arc::new(OnceCell::new()),
        }
    }

    /// git's version, found once with a run in `folder`. A git that is
    /// missing or too old fails every run; neither is kept, so installing a
    /// newer git helps at once.
    pub(crate) async fn version(&self, folder: &Path) -> Result<Version, GitFailure> {
        let found = self
            .version
            .get_or_try_init(|| async {
                let ran = self
                    .run_unchecked(GitRun::new(folder, ["version"]).with_timeout(30))
                    .await?;
                let version = Version::parse(&ran.text()).ok_or_else(|| {
                    GitFailure::failed(&format!("git version printed {}", ran.text()))
                })?;
                if version < Version::MINIMUM {
                    return Err(GitFailure::TooOld { found: version });
                }
                Ok(version)
            })
            .await?;
        Ok(*found)
    }

    /// The canonical folder of one of the owner's projects, for a command of
    /// the side panel, which names the project by its ID.
    pub(crate) async fn project_folder(&self, project: ProjectId) -> Result<PathBuf, ProcessError> {
        self.processes.project_folder(project).await
    }

    /// Runs git and returns what it produced, whatever its exit code.
    pub(crate) async fn run(&self, run: GitRun) -> Result<Ran, GitFailure> {
        self.version(&run.folder).await?;
        self.run_unchecked(run).await
    }

    /// Runs git and returns its output, or why it failed when it did not exit
    /// with 0.
    pub(crate) async fn output(&self, run: GitRun) -> Result<Ran, GitFailure> {
        let folder = run.folder.clone();
        let ran = self.run(run).await?;
        if ran.succeeded() {
            Ok(ran)
        } else {
            Err(ran.failure(&folder))
        }
    }

    async fn run_unchecked(&self, run: GitRun) -> Result<Ran, GitFailure> {
        let mut program = ProgramRun::new(GIT, &run.folder, run.arguments)
            .with_input(run.input)
            .with_timeout_seconds(run.timeout_seconds)
            .with_output_limit(run.output_limit);
        for (name, value) in VARIABLES {
            program = program.with_variable(name, value);
        }
        let protocols = if run.remote {
            REMOTE_PROTOCOLS
        } else {
            NO_PROTOCOL
        };
        program = program.with_variable("GIT_ALLOW_PROTOCOL", protocols);
        for (name, value) in &run.variables {
            program = program.with_variable(name.as_str(), value.as_str());
        }
        if let Some(place) = run.reads {
            program = program.checked_as(place);
        }
        let output = self
            .processes
            .run(program)
            .await
            .map_err(|error| refused(&run.folder, error))?;
        match output.ending {
            ProgramEnding::TimedOut => return Err(GitFailure::TimedOut),
            ProgramEnding::Exited | ProgramEnding::OutputFull => {}
        }
        let cut = output.output_cut();
        Ok(Ran {
            errors: redact(&String::from_utf8_lossy(&output.errors)),
            output: output.output,
            cut,
            exit_code: output.exit_code,
        })
    }
}

/// The failure of a run that the core refused or could not start.
fn refused(folder: &Path, error: ProcessError) -> GitFailure {
    match error {
        ProcessError::NotFound { .. } => GitFailure::Missing,
        ProcessError::ReadNotAllowed { path } => GitFailure::ReadNotAllowed { folder: path },
        ProcessError::InvalidRun { detail } => {
            tracing::warn!(%detail, "the core refused a run of git");
            GitFailure::failed(&format!(
                "Mannie refused to run git in {}: {detail}",
                folder.display()
            ))
        }
        other => {
            tracing::warn!(error = %other, "git did not start");
            GitFailure::failed(&other.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Ran, locked_path};
    use crate::failure::GitFailure;

    fn ran(errors: &str) -> Ran {
        Ran {
            output: Vec::new(),
            cut: false,
            errors: errors.to_owned(),
            exit_code: Some(128),
        }
    }

    #[test]
    fn a_failure_is_named_by_what_its_message_holds_in_any_language() {
        let folder = Path::new("D:/work");

        assert_eq!(
            ran("fatal: detected dubious ownership in repository at 'D:/work'\nTo add an exception, call:\n\tgit config --global --add safe.directory D:/work").failure(folder),
            GitFailure::DubiousOwnership {
                folder: "D:/work".to_owned()
            }
        );
        assert_eq!(
            ran("fatal: Impossible de créer 'D:/work/.git/index.lock' : le fichier existe.")
                .failure(folder),
            GitFailure::Locked {
                path: "D:/work/.git/index.lock".to_owned()
            }
        );
        assert_eq!(
            ran("fatal: not a git repository (or any of the parent directories): .git")
                .failure(folder),
            GitFailure::NotARepository {
                folder: "D:/work".to_owned()
            }
        );
        assert_eq!(
            ran("error: something else").failure(folder),
            GitFailure::failed("error: something else")
        );
        assert!(matches!(ran("").failure(folder), GitFailure::Failed { .. }));
    }

    #[test]
    fn only_a_quoted_path_ending_in_lock_names_a_lock() {
        assert_eq!(locked_path("no lock here"), None);
        assert_eq!(locked_path("'.lock'"), None);
        assert_eq!(
            locked_path("Unable to create \"/r/.git/refs/heads/main.lock\": File exists")
                .as_deref(),
            Some("/r/.git/refs/heads/main.lock")
        );
    }
}
