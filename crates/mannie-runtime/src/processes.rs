//! The runtime's provider of the service `processes` (ADR-0038, decision 7):
//! the installed programs that a plugin's manifest declares, found on
//! Mannie's `PATH` and run without a shell under the process supervisor.
//!
//! The provider binds the service to one plugin with the programs of its
//! manifest, so a plugin runs only what its manifest declares, with the
//! arguments that the manifest fixes first. A run for a command or a mention
//! provider names the folders that it reads, and the provider checks them as
//! the policy would check a model's reads in that place before anything
//! starts (decision 10).
//!
//! The runtime keeps what each plugin's manifest declares and where it found
//! each program, so the adapter of the plugins' tools adds the program's
//! path and its fixed arguments to a program access (decision 5).

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::FutureExt;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::host::{BindRefusal, PluginBinding, ServiceProvider};
use mannie_plugin_sdk::manifest::ProcessSection;
use mannie_plugin_sdk::service::{BoundService, Service};
use mannie_plugin_sdk::services::processes::{
    MAX_PROGRAM_INPUT_BYTES, MAX_PROGRAM_OUTPUT_BYTES, MAX_PROGRAM_TIMEOUT_SECONDS,
    OwnerOperationOutcome, OwnerOperationRequest, OwnerOperationTicket, ProcessError, Processes,
    ProcessesApi, ProgramEnding, ProgramOutput, ProgramRun, ReadPlace,
};
use mannie_store::{NewProgramOperation, ProgramOperationError};
use mannie_tools::{
    CommandEnvironment, ProgramEnvironmentError, Verdict, canonical_folder, find_program,
    path_is_inside, plugin_path_access, run_program,
};
use mannie_types::ids::{ProgramOperationId, ProjectId};
use mannie_types::mentions::MentionPlace;
use mannie_types::tools::{AccessMode, ToolsSettings};
use sha2::{Digest, Sha256};

use crate::{Runtime, lock};

/// The most target folders that one owner program operation may name.
const MAX_OWNER_FOLDERS: usize = 16;

/// The longest action code that an owner program operation may carry.
const MAX_ACTION_BYTES: usize = 64;

/// An active owner program operation authorized by a ticket (ADR-0078).
#[derive(Debug, Clone)]
struct ActiveOwnerOperation {
    id: ProgramOperationId,
    plugin: String,
    program: String,
    folders: Vec<PathBuf>,
}

/// The installed programs of the plugins: the tables of each plugin's
/// manifest under `[processes]`, which the provider records when it binds
/// the service, and where the core found each program.
#[derive(Debug, Default)]
pub(crate) struct Programs {
    /// The programs of each plugin's manifest, by plugin and by name.
    declared: Mutex<HashMap<String, BTreeMap<String, ProcessSection>>>,
    /// Where each program was found, by name. Mannie's `PATH` does not
    /// change while it runs.
    located: Mutex<HashMap<String, PathBuf>>,
    /// Active journaled owner operations, keyed by their opaque ticket text.
    owner_operations: Mutex<HashMap<String, ActiveOwnerOperation>>,
}

impl Programs {
    /// Records the programs that a plugin's manifest declares.
    fn declare(&self, plugin: &str, programs: BTreeMap<String, ProcessSection>) {
        lock(&self.declared).insert(plugin.to_owned(), programs);
    }

    /// The table of a program that a plugin's manifest declares.
    pub(crate) fn declared(&self, plugin: &str, program: &str) -> Option<ProcessSection> {
        lock(&self.declared).get(plugin)?.get(program).cloned()
    }
}

impl Runtime {
    /// Where an installed program is: on Mannie's `PATH`, outside the owner's
    /// projects and Mannie's own folders (T-46). A program found before is
    /// checked against the projects of now, which may have grown around it.
    pub(crate) async fn locate_program(&self, program: &str) -> Result<PathBuf, ProcessError> {
        let not_found = || ProcessError::NotFound {
            program: program.to_owned(),
        };
        let projects = self.inner.store.projects().await.map_err(|error| {
            tracing::error!(error = %error, error.code = error.code(), "the projects could not be read to find a program");
            not_found()
        })?;
        let mut excluded: Vec<PathBuf> = self.inner.tools.protected.clone();
        excluded.extend(
            projects
                .into_iter()
                .map(|project| PathBuf::from(project.folder)),
        );
        let known = lock(&self.inner.programs.located).get(program).cloned();
        if let Some(path) = known
            && !excluded.iter().any(|folder| path_is_inside(&path, folder))
        {
            return Ok(path);
        }
        let path_variable = std::env::var_os("PATH").unwrap_or_default();
        let name = program.to_owned();
        let path =
            tokio::task::spawn_blocking(move || find_program(&name, &path_variable, &excluded))
                .await
                .map_err(|_join_error| not_found())?
                .map_err(|_not_on_path| not_found())?;
        lock(&self.inner.programs.located).insert(program.to_owned(), path.clone());
        Ok(path)
    }
}

/// The runtime's provider of the service `processes`.
#[derive(Debug)]
pub(crate) struct ProcessesProvider {
    runtime: Runtime,
}

impl ProcessesProvider {
    /// The provider of `runtime`'s installed programs.
    pub(crate) fn new(runtime: &Runtime) -> Self {
        Self {
            runtime: runtime.clone(),
        }
    }
}

impl ServiceProvider for ProcessesProvider {
    fn name(&self) -> &'static str {
        Processes::NAME
    }

    fn major(&self) -> u16 {
        Processes::MAJOR
    }

    /// Binds the service to the programs that the plugin's manifest
    /// declares, and to nothing else, and records them for the plugin's
    /// program accesses.
    fn bind(&self, binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        let plugin = binding.id.as_str().to_owned();
        let programs = binding.manifest.processes.clone();
        self.runtime
            .inner
            .programs
            .declare(&plugin, programs.clone());
        Ok(BoundService::new::<Processes>(Arc::new(BoundProcesses {
            runtime: self.runtime.clone(),
            plugin,
            programs,
        })))
    }
}

/// The service bound to one plugin.
#[derive(Debug)]
struct BoundProcesses {
    runtime: Runtime,
    plugin: String,
    /// The programs of the plugin's manifest, by name.
    programs: BTreeMap<String, ProcessSection>,
}

impl ProcessesApi for BoundProcesses {
    fn locate(&self, program: &str) -> BoxFuture<'static, Result<PathBuf, ProcessError>> {
        let program = program.to_owned();
        let runtime = self.runtime.clone();
        let declared = self.programs.contains_key(&program);
        async move {
            if !declared {
                return Err(ProcessError::NotDeclared { program });
            }
            runtime.locate_program(&program).await
        }
        .boxed()
    }

    fn run(&self, run: ProgramRun) -> BoxFuture<'static, Result<ProgramOutput, ProcessError>> {
        let runtime = self.runtime.clone();
        let plugin = self.plugin.clone();
        let section = self.programs.get(&run.program).cloned();
        async move {
            let Some(section) = section else {
                return Err(ProcessError::NotDeclared {
                    program: run.program,
                });
            };
            check_limits(&run)?;
            let program = runtime.locate_program(&run.program).await?;
            let working_directory = working_directory(&runtime, &run.working_directory)?;

            if run.reads.is_some() && run.owner_operation.is_some() {
                return Err(ProcessError::InvalidRun {
                    detail: "a run cannot combine read checks and an owner operation ticket"
                        .to_owned(),
                });
            }

            if let Some(place) = &run.reads {
                check_reads(&runtime, place).await?;
            }

            if let Some(ticket) = &run.owner_operation {
                check_owner_ticket(&runtime, &plugin, &run.program, &working_directory, ticket)?;
            }

            let environment = environment(&runtime, &section, &run).await?;
            let mut arguments = section.fixed.clone();
            arguments.extend(run.arguments);
            let started = run_program(mannie_tools::ProgramRun {
                program,
                arguments,
                working_directory,
                environment,
                input: run.input,
                timeout: Duration::from_secs(u64::from(run.timeout_seconds)),
                output_limit: usize::try_from(run.output_limit).unwrap_or(usize::MAX),
            })
            .await;
            match started {
                Ok(output) => {
                    // Arguments can hold the owner's content, so the log
                    // names only the program and how its run ended.
                    tracing::info!(
                        plugin,
                        program = run.program,
                        ending = ?output.ending,
                        exit_code = output.exit_code,
                        duration_ms = output.duration.as_millis(),
                        "a plugin ran an installed program"
                    );
                    Ok(sdk_output(output))
                }
                Err(error) => {
                    tracing::warn!(plugin, program = run.program, %error, "a plugin's installed program did not start");
                    Err(ProcessError::StartFailed {
                        detail: error.to_string(),
                    })
                }
            }
        }
        .boxed()
    }

    fn project_folder(
        &self,
        project: ProjectId,
    ) -> BoxFuture<'static, Result<PathBuf, ProcessError>> {
        let runtime = self.runtime.clone();
        async move {
            runtime
                .mention_project_folder(&project)
                .await
                .map(PathBuf::from)
                .map_err(|_error| ProcessError::ProjectNotFound)
        }
        .boxed()
    }

    fn begin_owner_operation(
        &self,
        request: OwnerOperationRequest,
    ) -> BoxFuture<'static, Result<OwnerOperationTicket, ProcessError>> {
        let runtime = self.runtime.clone();
        let plugin = self.plugin.clone();
        let declared = self.programs.contains_key(&request.program);
        async move {
            if !declared {
                return Err(ProcessError::NotDeclared {
                    program: request.program,
                });
            }

            check_owner_action(&request.action)?;

            let project_folder = runtime
                .mention_project_folder(&request.project)
                .await
                .map_err(|_error| ProcessError::ProjectNotFound)?;
            let folders = check_owner_folders(
                &runtime,
                &request.project,
                &project_folder,
                &request.folders,
            )
            .await?;

            let digest = arguments_digest(
                &request.program,
                &project_folder,
                &request.action,
                &request.arguments,
            );
            let effects = serde_json::to_string(&request.effects).unwrap_or_else(|_| "[]".into());
            let record = NewProgramOperation {
                id: request.id.clone(),
                plugin: plugin.clone(),
                project_id: request.project.clone(),
                program: request.program.clone(),
                action: request.action.clone(),
                arguments_digest: digest,
                effects,
            };

            runtime
                .inner
                .store
                .begin_program_operation(&record)
                .await
                .map_err(|error| match error {
                    ProgramOperationError::AlreadyJournaled => ProcessError::AlreadyRun,
                    ProgramOperationError::Store(error) => {
                        tracing::error!(
                            error.code = error.code(),
                            "the journal of owner program operations failed"
                        );
                        ProcessError::StartFailed {
                            detail: "the operation journal could not be written".to_owned(),
                        }
                    }
                })?;

            let token = uuid::Uuid::now_v7().to_string();
            lock(&runtime.inner.programs.owner_operations).insert(
                token.clone(),
                ActiveOwnerOperation {
                    id: request.id,
                    plugin,
                    program: request.program,
                    folders,
                },
            );

            Ok(OwnerOperationTicket::new(token))
        }
        .boxed()
    }

    fn finish_owner_operation(
        &self,
        ticket: OwnerOperationTicket,
        outcome: OwnerOperationOutcome,
    ) -> BoxFuture<'static, Result<(), ProcessError>> {
        let runtime = self.runtime.clone();
        let plugin = self.plugin.clone();
        async move {
            let active = {
                let mut map = lock(&runtime.inner.programs.owner_operations);
                let matches = map
                    .get(ticket.as_str())
                    .is_some_and(|entry| entry.plugin == plugin);
                if !matches {
                    return Err(ProcessError::InvalidOperationTicket);
                }
                map.remove(ticket.as_str())
            };

            let Some(active) = active else {
                return Err(ProcessError::InvalidOperationTicket);
            };

            let store = &runtime.inner.store;
            let recorded = match outcome {
                OwnerOperationOutcome::Completed { exit_code } => {
                    store
                        .complete_program_operation(&active.id, exit_code)
                        .await
                }
                OwnerOperationOutcome::Failed { code } => {
                    store.fail_program_operation(&active.id, &code).await
                }
                OwnerOperationOutcome::Uncertain => {
                    store.mark_program_operation_uncertain(&active.id).await
                }
            };

            if let Err(error) = recorded {
                tracing::error!(
                    error.code = error.code(),
                    plugin,
                    program = active.program,
                    "an owner program operation finished and its journal record did not update"
                );
            }

            Ok(())
        }
        .boxed()
    }
}

/// Validates an owner operation's action code.
fn check_owner_action(action: &str) -> Result<(), ProcessError> {
    let valid_chars = action.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
    });

    if action.is_empty() || action.len() > MAX_ACTION_BYTES || !valid_chars {
        return Err(ProcessError::InvalidRun {
            detail: "the owner operation action code is invalid".to_owned(),
        });
    }

    Ok(())
}

/// Canonicalizes and authorizes the folders that an owner program operation
/// writes to (ADR-0078, decision 2): folders inside the project are allowed
/// directly, Mannie's protected folders are always refused (T-18), and folders
/// outside the project require an existing project or shared write grant.
async fn check_owner_folders(
    runtime: &Runtime,
    project: &ProjectId,
    project_folder: &str,
    folders: &[PathBuf],
) -> Result<Vec<PathBuf>, ProcessError> {
    if folders.is_empty() || folders.len() > MAX_OWNER_FOLDERS {
        return Err(ProcessError::InvalidRun {
            detail: "the owner operation must name between 1 and 16 folders".to_owned(),
        });
    }

    let project_canonical = working_directory(runtime, Path::new(project_folder))?;
    let mut canonicals = Vec::with_capacity(folders.len());
    let mut external = Vec::new();

    for folder in folders {
        let canonical = working_directory(runtime, folder)?;

        if !path_is_inside(&canonical, &project_canonical) {
            external.push(canonical.clone());
        }

        canonicals.push(canonical);
    }

    if !external.is_empty() {
        let context = runtime
            .mention_context_for(None, Some(project), Some(project_folder), false)
            .await
            .map_err(|_error| ProcessError::InvalidRun {
                detail: "the project's policy context could not be read".to_owned(),
            })?;

        for folder in &external {
            let refused = || ProcessError::WriteNotAllowed {
                path: folder.display().to_string(),
            };
            let text = folder.to_str().ok_or_else(refused)?;
            let access =
                plugin_path_access(text, AccessMode::Write, None).map_err(|_failure| refused())?;

            match context.decide(&[access]) {
                Verdict::Unrestricted | Verdict::Allowed(_) => {}
                Verdict::Ask(_) | Verdict::Refused(_) => return Err(refused()),
            }
        }
    }

    Ok(canonicals)
}

/// Verifies that an owner operation ticket belongs to `plugin` and `program`
/// and authorizes `working_directory`.
fn check_owner_ticket(
    runtime: &Runtime,
    plugin: &str,
    program: &str,
    working_directory: &Path,
    ticket: &OwnerOperationTicket,
) -> Result<(), ProcessError> {
    let operations = lock(&runtime.inner.programs.owner_operations);
    let Some(active) = operations.get(ticket.as_str()) else {
        return Err(ProcessError::InvalidOperationTicket);
    };

    if active.plugin != plugin || active.program != program {
        return Err(ProcessError::InvalidOperationTicket);
    }

    let allowed = active
        .folders
        .iter()
        .any(|folder| path_is_inside(working_directory, folder));

    if !allowed {
        return Err(ProcessError::WriteNotAllowed {
            path: working_directory.display().to_string(),
        });
    }

    Ok(())
}

/// Computes the SHA-256 hex digest of an owner program operation's arguments so
/// raw arguments never reach the journal or logs.
fn arguments_digest(
    program: &str,
    project_folder: &str,
    action: &str,
    arguments: &[String],
) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let payload =
        serde_json::to_string(&(program, project_folder, action, arguments)).unwrap_or_default();
    let digest = Sha256::digest(payload.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);

    for byte in digest {
        hex.push(char::from(DIGITS[usize::from(byte >> 4)]));
        hex.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }

    hex
}

/// Refuses a run past the limits of programs before anything starts.
fn check_limits(run: &ProgramRun) -> Result<(), ProcessError> {
    let detail = if run.input.len() > MAX_PROGRAM_INPUT_BYTES {
        Some("the input is larger than 16 MiB")
    } else if run.output_limit == 0 || run.output_limit > MAX_PROGRAM_OUTPUT_BYTES {
        Some("the output limit is not between 1 byte and 16 MiB")
    } else if run.timeout_seconds == 0 || run.timeout_seconds > MAX_PROGRAM_TIMEOUT_SECONDS {
        Some("the timeout is not between 1 and 600 seconds")
    } else {
        None
    };
    match detail {
        Some(detail) => Err(ProcessError::InvalidRun {
            detail: detail.to_owned(),
        }),
        None => Ok(()),
    }
}

/// The canonical working directory of a run: an absolute folder that exists,
/// outside Mannie's own folders.
fn working_directory(runtime: &Runtime, folder: &Path) -> Result<PathBuf, ProcessError> {
    let invalid = |detail: &str| ProcessError::InvalidRun {
        detail: detail.to_owned(),
    };
    if !folder.is_absolute() {
        return Err(invalid("the working directory is not absolute"));
    }
    let canonical = PathBuf::from(
        canonical_folder(folder)
            .map_err(|_error| invalid("the working directory is not a folder"))?,
    );
    let in_mannie = runtime
        .inner
        .tools
        .protected
        .iter()
        .any(|mannie| path_is_inside(&canonical, mannie));
    if in_mannie {
        return Err(invalid(
            "the working directory is inside Mannie's own folders",
        ));
    }
    Ok(canonical)
}

/// Checks the folders that a run of a command or a mention provider reads,
/// as the policy would check a model's reads in its place (ADR-0038,
/// decision 10). A read that would ask refuses the run.
async fn check_reads(runtime: &Runtime, place: &ReadPlace) -> Result<(), ProcessError> {
    let (context, folders) = match place {
        ReadPlace::Conversation {
            conversation,
            folders,
        } => {
            let place = MentionPlace::Conversation {
                conversation_id: conversation.clone(),
            };
            (runtime.mention_context(&place).await, folders)
        }
        ReadPlace::Project { project, folders } => {
            let folder = runtime
                .mention_project_folder(project)
                .await
                .map_err(|_error| ProcessError::ProjectNotFound)?;
            // The owner's view of a project reads as the project's
            // conversations read, without full access, which belongs to a
            // conversation.
            let context = runtime
                .mention_context_for(None, Some(project), Some(folder.as_str()), false)
                .await;
            (context, folders)
        }
    };
    let context = context.map_err(|_error| ProcessError::InvalidRun {
        detail: "the place of the run could not be read".to_owned(),
    })?;
    for folder in folders {
        let refused = || ProcessError::ReadNotAllowed {
            path: folder.display().to_string(),
        };
        let text = folder.to_str().ok_or_else(refused)?;
        let access =
            plugin_path_access(text, AccessMode::Read, None).map_err(|_failure| refused())?;
        match context.decide(&[access]) {
            Verdict::Unrestricted | Verdict::Allowed(_) => {}
            Verdict::Ask(_) | Verdict::Refused(_) => return Err(refused()),
        }
    }
    Ok(())
}

/// The environment of a run: the environment of commands, with the names
/// that the owner added in the Tools settings, the names that the manifest
/// inherits, and the variables that the plugin sets.
async fn environment(
    runtime: &Runtime,
    section: &ProcessSection,
    run: &ProgramRun,
) -> Result<Vec<(OsString, OsString)>, ProcessError> {
    let settings = runtime
        .inner
        .store
        .load_settings::<ToolsSettings>()
        .await
        .unwrap_or_default();
    let set: Vec<(String, String)> = run
        .environment
        .iter()
        .map(|variable| (variable.name.clone(), variable.value.clone()))
        .collect();
    CommandEnvironment::from_process(&settings.command_environment)
        .for_program(&section.inherit, &set, std::env::vars_os())
        .map_err(|error| ProcessError::VariableRefused {
            name: refused_name(error),
        })
}

/// The name in a refusal of a program's environment.
fn refused_name(error: ProgramEnvironmentError) -> String {
    match error {
        ProgramEnvironmentError::Invalid { name }
        | ProgramEnvironmentError::Secret { name }
        | ProgramEnvironmentError::Loader { name }
        | ProgramEnvironmentError::Reserved { name }
        | ProgramEnvironmentError::Value { name } => name,
    }
}

/// What a run produced, as the SDK gives it to the plugin.
fn sdk_output(output: mannie_tools::ProgramOutput) -> ProgramOutput {
    let ending = match output.ending {
        mannie_tools::ProgramEnding::Exited => ProgramEnding::Exited,
        mannie_tools::ProgramEnding::TimedOut => ProgramEnding::TimedOut,
        mannie_tools::ProgramEnding::OutputFull => ProgramEnding::OutputFull,
    };
    let duration_ms = u64::try_from(output.duration.as_millis()).unwrap_or(u64::MAX);
    ProgramOutput::new(ending, output.exit_code, duration_ms)
        .with_output(output.output, output.output_total)
        .with_errors(output.errors, output.errors_total)
}
