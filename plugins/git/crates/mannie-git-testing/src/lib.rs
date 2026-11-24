//! Repositories for the git plugin's tests: a repository in a temporary
//! folder, which the installed git builds. The tests' own runs of git read an
//! empty global configuration and no system configuration, so the owner's
//! settings change no result, and the repository's own configuration names
//! an author and turns signing off, which also holds for the plugin's runs.
//!
//! [`HostGit`] stands in for the core's service `processes` in the plugin's
//! unit tests: it runs the installed git with the fixed arguments of the
//! plugin's manifest and the same empty configuration. Nothing that the
//! plugin ships reaches this crate.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{ProgramOperationId, ProjectId};
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::services::processes::{
    OwnerOperationOutcome, OwnerOperationRequest, OwnerOperationTicket, ProcessError, ProcessesApi,
    ProgramEnding, ProgramOutput, ProgramRun,
};
use tempfile::TempDir;

/// The plugin's manifest, whose fixed arguments every run starts with.
const MANIFEST: &str = include_str!("../../../plugin.toml");

/// The variables of the tests' environment that git needs to start, which
/// the core's environment of commands holds too.
const PASSED_VARIABLES: [&str; 6] = ["PATH", "SystemRoot", "TEMP", "TMP", "HOME", "USERPROFILE"];

/// An empty configuration file in a temporary folder, which the tests' runs
/// read as their global configuration.
#[derive(Debug)]
struct EmptyConfiguration {
    folder: TempDir,
}

impl EmptyConfiguration {
    fn new() -> Self {
        let folder = tempfile::tempdir().expect("a temporary folder");
        fs::write(folder.path().join("global.gitconfig"), b"").expect("an empty configuration");
        Self { folder }
    }

    fn path(&self) -> PathBuf {
        self.folder.path().join("global.gitconfig")
    }
}

/// A command that runs the installed git with the tests' configuration and
/// none of the owner's environment beyond what git needs to start.
fn git_command(global: &Path) -> Command {
    let mut command = Command::new("git");
    command.env_clear();
    for name in PASSED_VARIABLES {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("GIT_CONFIG_GLOBAL", global)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostBehavior {
    Normal,
    Missing,
    RefusesReads,
    RefusesWrites,
    TimeoutOwnerRuns,
}

/// The service `processes` of the plugin's unit tests, which runs the
/// installed git.
#[derive(Debug)]
pub struct HostGit {
    fixed: Vec<String>,
    configuration: EmptyConfiguration,
    runs: Mutex<Vec<ProgramRun>>,
    owner_operations: Mutex<Vec<(ProgramOperationId, Option<OwnerOperationOutcome>)>>,
    owner_requests: Mutex<Vec<OwnerOperationRequest>>,
    behavior: HostBehavior,
    projects: Vec<(ProjectId, PathBuf)>,
}

impl Default for HostGit {
    fn default() -> Self {
        Self::new()
    }
}

impl HostGit {
    /// The service, with the fixed arguments of the plugin's manifest.
    ///
    /// # Panics
    ///
    /// When the manifest does not parse or declares no git, which fails the
    /// test.
    pub fn new() -> Self {
        let manifest = Manifest::parse(MANIFEST).expect("the plugin's manifest parses");
        let fixed = manifest
            .processes
            .get("git")
            .expect("the manifest declares git")
            .fixed
            .clone();
        Self {
            fixed,
            configuration: EmptyConfiguration::new(),
            runs: Mutex::new(Vec::new()),
            owner_operations: Mutex::new(Vec::new()),
            owner_requests: Mutex::new(Vec::new()),
            behavior: HostBehavior::Normal,
            projects: Vec::new(),
        }
    }

    /// The service of a place that refuses owner writes (e.g. external git dir
    /// without a write grant).
    pub fn refusing_writes() -> Self {
        Self {
            behavior: HostBehavior::RefusesWrites,
            ..Self::new()
        }
    }

    /// Makes every run carrying an owner operation ticket time out.
    #[must_use]
    pub fn timing_out_owner_runs(mut self) -> Self {
        self.behavior = HostBehavior::TimeoutOwnerRuns;
        self
    }

    /// The owner operations started through this test double and their outcomes.
    pub fn owner_operations(&self) -> Vec<(ProgramOperationId, Option<OwnerOperationOutcome>)> {
        self.owner_operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The raw owner operation requests sent to the journal.
    pub fn owner_requests(&self) -> Vec<OwnerOperationRequest> {
        self.owner_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The service with the project `project` in `folder`, as the core finds
    /// a project's folder for the panel's commands.
    #[must_use]
    pub fn with_project(mut self, project: ProjectId, folder: &Path) -> Self {
        self.projects.push((project, folder.to_path_buf()));
        self
    }

    /// The service of a machine without git, which finds it nowhere.
    pub fn missing() -> Self {
        Self {
            behavior: HostBehavior::Missing,
            ..Self::new()
        }
    }

    /// The service of a place whose policy would ask about every checked
    /// read, as the core refuses a mention's run there.
    pub fn refusing_reads() -> Self {
        Self {
            behavior: HostBehavior::RefusesReads,
            ..Self::new()
        }
    }

    /// The runs that the plugin asked for, in their order.
    pub fn runs(&self) -> Vec<ProgramRun> {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Runs git as the core's service would, without a timeout: the tests'
    /// git ends on its own.
    fn run_now(&self, run: &ProgramRun) -> Result<ProgramOutput, ProcessError> {
        let mut command = git_command(&self.configuration.path());
        command
            .args(&self.fixed)
            .args(&run.arguments)
            .current_dir(&run.working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for variable in &run.environment {
            command.env(&variable.name, &variable.value);
        }
        let started = Instant::now();
        let mut child = command.spawn().map_err(|error| ProcessError::StartFailed {
            detail: error.to_string(),
        })?;
        let input = run.input.clone();
        let mut stdin = child.stdin.take().expect("the input is piped");
        let writer = std::thread::spawn(move || {
            // git may exit before it reads all of its input.
            let _written = stdin.write_all(&input);
        });
        let output = child
            .wait_with_output()
            .map_err(|error| ProcessError::StartFailed {
                detail: error.to_string(),
            })?;
        writer.join().expect("the input's writer ends");
        let limit = usize::try_from(run.output_limit).unwrap_or(usize::MAX);
        let total = u64::try_from(output.stdout.len()).unwrap_or(u64::MAX);
        let errors_total = u64::try_from(output.stderr.len()).unwrap_or(u64::MAX);
        let mut kept = output.stdout;
        let ending = if kept.len() > limit {
            kept.truncate(limit);
            ProgramEnding::OutputFull
        } else {
            ProgramEnding::Exited
        };
        let exit_code = match ending {
            ProgramEnding::Exited => output.status.code(),
            ProgramEnding::OutputFull | ProgramEnding::TimedOut => None,
        };
        let duration = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(ProgramOutput::new(ending, exit_code, duration)
            .with_output(kept, total)
            .with_errors(output.stderr, errors_total))
    }
}

impl ProcessesApi for HostGit {
    fn locate(&self, program: &str) -> BoxFuture<'static, Result<PathBuf, ProcessError>> {
        let located = if self.behavior == HostBehavior::Missing {
            Err(ProcessError::NotFound {
                program: program.to_owned(),
            })
        } else {
            Ok(PathBuf::from(program))
        };
        Box::pin(std::future::ready(located))
    }

    fn run(&self, run: ProgramRun) -> BoxFuture<'static, Result<ProgramOutput, ProcessError>> {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(run.clone());
        let output = if self.behavior == HostBehavior::Missing {
            Err(ProcessError::NotFound {
                program: run.program,
            })
        } else if self.behavior == HostBehavior::RefusesReads && run.reads.is_some() {
            Err(ProcessError::ReadNotAllowed {
                path: run.working_directory.display().to_string(),
            })
        } else if self.behavior == HostBehavior::TimeoutOwnerRuns && run.owner_operation.is_some() {
            Ok(ProgramOutput::new(ProgramEnding::TimedOut, None, 1))
        } else {
            self.run_now(&run)
        };
        Box::pin(std::future::ready(output))
    }

    fn project_folder(
        &self,
        project: ProjectId,
    ) -> BoxFuture<'static, Result<PathBuf, ProcessError>> {
        let folder = self
            .projects
            .iter()
            .find(|(known, _folder)| *known == project)
            .map(|(_project, folder)| folder.clone())
            .ok_or(ProcessError::ProjectNotFound);
        Box::pin(std::future::ready(folder))
    }

    fn begin_owner_operation(
        &self,
        request: OwnerOperationRequest,
    ) -> BoxFuture<'static, Result<OwnerOperationTicket, ProcessError>> {
        let result = if self.behavior == HostBehavior::Missing {
            Err(ProcessError::NotFound {
                program: request.program,
            })
        } else if self.behavior == HostBehavior::RefusesWrites {
            let path = request
                .folders
                .first()
                .map(|folder| folder.display().to_string())
                .unwrap_or_default();
            Err(ProcessError::WriteNotAllowed { path })
        } else {
            let mut ops = self
                .owner_operations
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if ops.iter().any(|(id, _outcome)| *id == request.id) {
                Err(ProcessError::AlreadyRun)
            } else {
                let token = request.id.to_string();
                ops.push((request.id.clone(), None));
                self.owner_requests
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(request);
                Ok(OwnerOperationTicket::new(token))
            }
        };
        Box::pin(std::future::ready(result))
    }

    fn finish_owner_operation(
        &self,
        ticket: OwnerOperationTicket,
        outcome: OwnerOperationOutcome,
    ) -> BoxFuture<'static, Result<(), ProcessError>> {
        let mut ops = self
            .owner_operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let result = ops
            .iter_mut()
            .find(|(id, _recorded)| id.as_str() == ticket.as_str())
            .map(|(_id, recorded)| {
                *recorded = Some(outcome);
            })
            .ok_or(ProcessError::InvalidOperationTicket);
        Box::pin(std::future::ready(result))
    }
}

/// An empty temporary folder, removed when it drops.
#[derive(Debug)]
pub struct TempFolder(TempDir);

impl Default for TempFolder {
    fn default() -> Self {
        Self::new()
    }
}

impl TempFolder {
    /// A new empty folder.
    ///
    /// # Panics
    ///
    /// When the folder cannot be made, which fails the test.
    pub fn new() -> Self {
        Self(tempfile::tempdir().expect("a temporary folder"))
    }

    /// The folder.
    pub fn path(&self) -> &Path {
        self.0.path()
    }
}

/// The hooks that git runs on the client's side, which
/// [`TestRepository::record_hooks`] installs.
pub const CLIENT_HOOKS: [&str; 16] = [
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "pre-auto-gc",
    "post-rewrite",
    "reference-transaction",
    "post-index-change",
];

/// A repository in a temporary folder, on the branch `main`.
#[derive(Debug)]
pub struct TestRepository {
    folder: TempDir,
    root: PathBuf,
    configuration: EmptyConfiguration,
}

impl Default for TestRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl TestRepository {
    /// An empty repository on `main`, whose configuration names the author
    /// `Test Author <test@example.com>` and signs nothing.
    ///
    /// # Panics
    ///
    /// When the temporary folder or the repository cannot be made, which
    /// fails the test.
    pub fn new() -> Self {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let root = folder.path().join("repository");
        fs::create_dir_all(&root).expect("the repository's folder");
        let repository = Self {
            folder,
            root,
            configuration: EmptyConfiguration::new(),
        };
        repository.git(&["init", "--quiet", "--initial-branch=main"]);
        for (key, value) in [
            ("user.name", "Test Author"),
            ("user.email", "test@example.com"),
            ("commit.gpgSign", "false"),
            ("tag.gpgSign", "false"),
            ("core.autocrlf", "false"),
        ] {
            repository.git(&["config", key, value]);
        }
        repository
    }

    /// The repository's folder.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// A folder beside the repository, inside the test's temporary folder,
    /// such as for a bare remote.
    ///
    /// # Panics
    ///
    /// When the folder cannot be made, which fails the test.
    pub fn sibling(&self, name: &str) -> PathBuf {
        let path = self.folder.path().join(name);
        fs::create_dir_all(&path).expect("the sibling folder");
        path
    }

    /// Writes a file of the work tree, with its folders.
    ///
    /// # Panics
    ///
    /// When the file cannot be written, which fails the test.
    pub fn write(&self, path: &str, text: &str) {
        let file = self.root.join(path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).expect("the file's folder");
        }
        fs::write(file, text).expect("the file");
    }

    /// Reads a file of the work tree.
    ///
    /// # Panics
    ///
    /// When the file cannot be read, which fails the test.
    pub fn read(&self, path: &str) -> String {
        fs::read_to_string(self.root.join(path)).expect("the file")
    }

    /// Runs git in the repository and returns its output.
    ///
    /// # Panics
    ///
    /// When git fails, which fails the test with git's error output.
    pub fn git(&self, arguments: &[&str]) -> String {
        self.git_in(&self.root, arguments)
    }

    /// Runs git in `folder`, with the test's configuration, and returns its
    /// output.
    ///
    /// # Panics
    ///
    /// When git fails, which fails the test with git's error output.
    pub fn git_in(&self, folder: &Path, arguments: &[&str]) -> String {
        let output = git_command(&self.configuration.path())
            .args(arguments)
            .current_dir(folder)
            .output()
            .expect("the tests need git on the PATH");
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Runs git in the repository and returns whether it succeeded, for a
    /// command that stops on a conflict, such as a merge.
    ///
    /// # Panics
    ///
    /// When git cannot be started, which fails the test.
    pub fn git_may_fail(&self, arguments: &[&str]) -> bool {
        git_command(&self.configuration.path())
            .args(arguments)
            .current_dir(&self.root)
            .output()
            .expect("the tests need git on the PATH")
            .status
            .success()
    }

    /// Installs `script` as the hook `name` in the hooks folder, executable
    /// on Unix, where git skips a hook without its executable bit.
    ///
    /// # Panics
    ///
    /// When the hook cannot be written, which fails the test.
    pub fn write_hook(&self, name: &str, script: &str) {
        let hooks = self.root.join(".git").join("hooks");
        fs::create_dir_all(&hooks).expect("the hooks folder");
        let path = hooks.join(name);
        fs::write(&path, script).expect("the hook");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("the hook's mode");
        }
    }

    /// Installs every client hook as a recorder that appends its name to a
    /// log and lets git go on, so a test sees which hooks a run started.
    ///
    /// # Panics
    ///
    /// When a hook cannot be written, which fails the test.
    pub fn record_hooks(&self) {
        let log = self.hooks_log().display().to_string().replace('\\', "/");
        for hook in CLIENT_HOOKS {
            self.write_hook(
                hook,
                &format!("#!/bin/sh\necho {hook} >> '{log}'\nexit 0\n"),
            );
        }
    }

    fn hooks_log(&self) -> PathBuf {
        self.folder.path().join("hooks.log")
    }

    /// The hooks that ran since the last call, each once, and empties the
    /// log.
    pub fn hooks_ran(&self) -> Vec<String> {
        let log = self.hooks_log();
        let text = fs::read_to_string(&log).unwrap_or_default();
        // The log may not exist yet.
        let _removed = fs::remove_file(&log);
        let mut ran: Vec<String> = text.lines().map(|line| line.trim().to_owned()).collect();
        ran.sort();
        ran.dedup();
        ran
    }

    /// A command for the shell that git runs a configured program with,
    /// which appends `label` to `marker`, such as to prove that a planted
    /// program ran. `then` follows it, such as `cat` for a filter, which
    /// must pass its input on.
    pub fn marking_command(marker: &Path, label: &str, then: &str) -> String {
        let marker = marker.display().to_string().replace('\\', "/");
        format!("echo {label} >> '{marker}'; {then}")
    }

    /// Stages every change and commits it, and returns the new commit's
    /// hash.
    pub fn commit(&self, message: &str) -> String {
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "--allow-empty", "-m", message]);
        self.head()
    }

    /// The hash of HEAD.
    pub fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"]).trim().to_owned()
    }
}
