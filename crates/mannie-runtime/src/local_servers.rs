//! The runtime's provider of the service `local-servers` (ADR-0043, decision
//! 5): programs that the owner named in a plugin's settings, which run as
//! servers under the process supervisor until the plugin stops them, the
//! plugin goes off or Mannie quits.
//!
//! The provider binds the service to one plugin, and keeps the servers of
//! each plugin, so the end of the plugin ends every server that it started
//! and no server outlives it (T-59). Each server has a task of its own,
//! which writes what the plugin sends to the server's input, reads its
//! output and its error output, and ends its tree.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::stream::{self, BoxStream as FuturesBoxStream};
use futures_util::{FutureExt, StreamExt};
use mannie_plugin_sdk::host::{BindRefusal, PluginBinding, ServiceProvider};
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::service::{BoundService, Service};
use mannie_plugin_sdk::services::local_servers::{
    LocalServer, LocalServerError, LocalServerSpec, LocalServers, LocalServersApi, LocatedProgram,
    MAX_SERVER_ARGUMENT_BYTES, MAX_SERVER_ARGUMENTS, MAX_SERVER_ERROR_LINE_BYTES,
    MAX_SERVER_SEND_BYTES, MAX_SERVER_STOP_GRACE, MAX_SERVER_VARIABLES, MAX_SERVERS_PER_PLUGIN,
    ServerEnding, ServerOutput,
};
use mannie_plugin_sdk::{BoxFuture, BoxStream};
use mannie_tools::{
    CommandEnvironment, ServerProcess, ServerSpec, canonical_folder, find_server_program,
    path_is_inside,
};
use mannie_types::tools::ToolsSettings;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::{Runtime, lock};

/// How many messages to a server wait for its input at once.
const INPUT_QUEUE: usize = 16;

/// How many chunks of a server's output wait for its plugin at once. A
/// plugin that reads none holds the server back, as a full pipe would.
const OUTPUT_QUEUE: usize = 64;

/// How many lines of a server's error output wait for its plugin at once.
/// Lines past them are dropped, so a server never waits on its error output.
const ERROR_QUEUE: usize = 256;

/// The bytes that one read of a server's output takes.
const OUTPUT_CHUNK_BYTES: usize = 64 * 1024;

/// How long the end of a plugin, or of Mannie, lets each of its servers end
/// on its own after its input closed.
const RELEASE_GRACE: Duration = Duration::from_secs(2);

/// How long the end of a plugin waits for its servers in all.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

/// The servers that run, by plugin, with what ends each one.
#[derive(Debug, Default)]
pub(crate) struct Servers {
    running: Mutex<HashMap<String, HashMap<u64, mpsc::Sender<Stop>>>>,
    next_id: AtomicU64,
}

/// A request to end a server, with where its end goes.
#[derive(Debug)]
struct Stop {
    grace: Duration,
    ended: Option<oneshot::Sender<ServerEnding>>,
}

impl Servers {
    /// Ends every server of a plugin, or of every plugin, and waits for them
    /// for at most [`RELEASE_TIMEOUT`].
    async fn end(&self, plugin: Option<&str>) {
        let stops: Vec<mpsc::Sender<Stop>> = {
            let running = lock(&self.running);
            running
                .iter()
                .filter(|(owner, _servers)| plugin.is_none_or(|plugin| plugin == owner.as_str()))
                .flat_map(|(_owner, servers)| servers.values().cloned())
                .collect()
        };
        if stops.is_empty() {
            return;
        }
        tracing::info!(plugin, servers = stops.len(), "ending the local servers");
        let ended = stops.into_iter().map(|stop| async move {
            let (sender, receiver) = oneshot::channel();
            let request = Stop {
                grace: RELEASE_GRACE,
                ended: Some(sender),
            };
            if stop.send(request).await.is_ok() {
                let _ending = receiver.await;
            }
        });
        if tokio::time::timeout(RELEASE_TIMEOUT, futures_util::future::join_all(ended))
            .await
            .is_err()
        {
            tracing::warn!(plugin, "local servers did not end in time");
        }
    }

    fn count(&self, plugin: &str) -> usize {
        lock(&self.running).get(plugin).map_or(0, HashMap::len)
    }

    fn add(&self, plugin: &str, stop: mpsc::Sender<Stop>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        lock(&self.running)
            .entry(plugin.to_owned())
            .or_default()
            .insert(id, stop);
        id
    }

    fn remove(&self, plugin: &str, id: u64) {
        let mut running = lock(&self.running);
        if let Some(servers) = running.get_mut(plugin) {
            servers.remove(&id);
            if servers.is_empty() {
                running.remove(plugin);
            }
        }
    }
}

impl Runtime {
    /// Ends every local server, as Mannie quits.
    pub(crate) async fn end_local_servers(&self) {
        self.inner.local_servers.end(None).await;
    }
}

/// The runtime's provider of the service `local-servers`.
#[derive(Debug)]
pub(crate) struct LocalServersProvider {
    runtime: Runtime,
}

impl LocalServersProvider {
    /// The provider of `runtime`'s local servers.
    pub(crate) fn new(runtime: &Runtime) -> Self {
        Self {
            runtime: runtime.clone(),
        }
    }
}

impl ServiceProvider for LocalServersProvider {
    fn name(&self) -> &'static str {
        LocalServers::NAME
    }

    fn major(&self) -> u16 {
        LocalServers::MAJOR
    }

    fn bind(&self, binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        Ok(BoundService::new::<LocalServers>(std::sync::Arc::new(
            BoundServers {
                runtime: self.runtime.clone(),
                plugin: binding.id.as_str().to_owned(),
            },
        )))
    }

    /// Ends the servers that the plugin started, once it stopped or failed
    /// to start, so none outlives it (T-59).
    fn release(&self, plugin: &PluginId) -> BoxFuture<'static, ()> {
        let runtime = self.runtime.clone();
        let plugin = plugin.as_str().to_owned();
        async move { runtime.inner.local_servers.end(Some(&plugin)).await }.boxed()
    }
}

/// The service bound to one plugin.
#[derive(Debug)]
struct BoundServers {
    runtime: Runtime,
    plugin: String,
}

impl LocalServersApi for BoundServers {
    fn locate(
        &self,
        program: &str,
    ) -> BoxFuture<'static, Result<LocatedProgram, LocalServerError>> {
        let runtime = self.runtime.clone();
        let program = program.to_owned();
        async move {
            let (path, in_project) = locate(&runtime, &program).await?;
            Ok(LocatedProgram::new(path, in_project))
        }
        .boxed()
    }

    fn start(
        &self,
        spec: LocalServerSpec,
    ) -> BoxFuture<'static, Result<Box<dyn LocalServer>, LocalServerError>> {
        let runtime = self.runtime.clone();
        let plugin = self.plugin.clone();
        async move { start(runtime, plugin, spec).await }.boxed()
    }
}

/// Where a server's program is, and whether it lies in one of the owner's
/// projects: an absolute path as it is, or a name on Mannie's `PATH`
/// outside the projects and Mannie's own folders. A program inside Mannie's
/// own folders is refused.
async fn locate(runtime: &Runtime, program: &str) -> Result<(PathBuf, bool), LocalServerError> {
    let projects: Vec<PathBuf> = runtime
        .inner
        .store
        .projects()
        .await
        .map_err(|error| {
            tracing::error!(error = %error, error.code = error.code(), "the projects could not be read to find a server's program");
            LocalServerError::ProgramNotFound {
                program: program.to_owned(),
            }
        })?
        .into_iter()
        .map(|project| PathBuf::from(project.folder))
        .collect();
    let protected = runtime.inner.tools.protected.clone();
    let path_variable = std::env::var_os("PATH").unwrap_or_default();
    let name = program.to_owned();
    let found = tokio::task::spawn_blocking(move || {
        find_server_program(&name, &path_variable, &projects, &protected)
    })
    .await
    .map_err(|_join_error| LocalServerError::ProgramNotFound {
        program: program.to_owned(),
    })?;
    match found {
        Ok(found) => Ok((found.path, found.in_project)),
        Err(mannie_tools::ServerProgramError::NotFound) => Err(LocalServerError::ProgramNotFound {
            program: program.to_owned(),
        }),
        Err(mannie_tools::ServerProgramError::Refused) => Err(LocalServerError::ProgramRefused {
            program: program.to_owned(),
        }),
    }
}

/// Checks a server's spec, finds its program and starts it.
async fn start(
    runtime: Runtime,
    plugin: String,
    spec: LocalServerSpec,
) -> Result<Box<dyn LocalServer>, LocalServerError> {
    if runtime.is_shutting_down() {
        return Err(LocalServerError::ShuttingDown);
    }
    check_arguments(&spec.arguments)?;
    let servers = &runtime.inner.local_servers;
    if servers.count(&plugin) >= usize::try_from(MAX_SERVERS_PER_PLUGIN).unwrap_or(usize::MAX) {
        return Err(LocalServerError::TooMany {
            limit: MAX_SERVERS_PER_PLUGIN,
        });
    }
    let (program, _in_project) = locate(&runtime, &spec.program).await?;
    let working_directory = working_directory(&runtime, spec.working_directory.as_deref())?;
    let environment = environment(&runtime, &spec).await?;
    let process = ServerProcess::spawn(&ServerSpec {
        program,
        arguments: spec.arguments,
        working_directory,
        environment,
    })
    .map_err(|error| {
        // The standard library refuses an argument of a batch file that
        // cmd.exe cannot take quoted.
        if error.kind() == std::io::ErrorKind::InvalidInput {
            return LocalServerError::ArgumentRefused;
        }
        // Arguments and variables can hold the owner's content and secrets,
        // so the log names only the program.
        tracing::warn!(plugin, program = spec.program, %error, "a local server did not start");
        LocalServerError::SpawnFailed
    })?;
    tracing::info!(
        plugin,
        program = spec.program,
        pid = process.record().pid,
        "a local server started"
    );
    Ok(Box::new(RunningServer::start(&runtime, plugin, process)))
}

/// Refuses arguments with NUL or past their limits before anything starts.
fn check_arguments(arguments: &[String]) -> Result<(), LocalServerError> {
    let bytes: usize = arguments.iter().map(String::len).sum();
    let fits = arguments.len() <= MAX_SERVER_ARGUMENTS
        && bytes <= MAX_SERVER_ARGUMENT_BYTES
        && !arguments.iter().any(|argument| argument.contains('\0'));
    if fits {
        Ok(())
    } else {
        Err(LocalServerError::ArgumentRefused)
    }
}

/// The canonical working directory of a server: an absolute folder that
/// exists, outside Mannie's own folders, or the owner's home folder.
fn working_directory(
    runtime: &Runtime,
    folder: Option<&Path>,
) -> Result<PathBuf, LocalServerError> {
    let folder = match folder {
        Some(folder) => folder.to_path_buf(),
        None => std::env::home_dir().ok_or(LocalServerError::WorkingDirectoryInvalid)?,
    };
    if !folder.is_absolute() {
        return Err(LocalServerError::WorkingDirectoryInvalid);
    }
    let canonical = PathBuf::from(
        canonical_folder(&folder).map_err(|_error| LocalServerError::WorkingDirectoryInvalid)?,
    );
    let in_mannie = runtime
        .inner
        .tools
        .protected
        .iter()
        .any(|mannie| path_is_inside(&canonical, mannie));
    if in_mannie {
        return Err(LocalServerError::WorkingDirectoryInvalid);
    }
    Ok(canonical)
}

/// The environment of a server: the environment of commands, with the names
/// that the owner added in the Tools settings, and the server's variables
/// and secrets, which reach this process alone (ADR-0043, decision 6).
async fn environment(
    runtime: &Runtime,
    spec: &LocalServerSpec,
) -> Result<Vec<(OsString, OsString)>, LocalServerError> {
    if spec.variables.len() + spec.secrets.len() > MAX_SERVER_VARIABLES {
        let name = spec
            .variables
            .iter()
            .map(|variable| variable.name.clone())
            .chain(spec.secrets.iter().map(|secret| secret.name.clone()))
            .nth(MAX_SERVER_VARIABLES)
            .unwrap_or_default();
        return Err(LocalServerError::VariableRefused { name });
    }
    let settings = runtime
        .inner
        .store
        .load_settings::<ToolsSettings>()
        .await
        .unwrap_or_default();
    let plain: Vec<(String, String)> = spec
        .variables
        .iter()
        .map(|variable| (variable.name.clone(), variable.value.clone()))
        .collect();
    let secrets: Vec<(String, &mannie_types::secret::Secret)> = spec
        .secrets
        .iter()
        .map(|secret| (secret.name.clone(), &secret.value))
        .collect();
    CommandEnvironment::from_process(&settings.command_environment)
        .for_server(&plain, &secrets)
        .map_err(|error| LocalServerError::VariableRefused {
            name: error.name().to_owned(),
        })
}

/// A server that runs, as its plugin holds it.
#[derive(Debug)]
struct RunningServer {
    input: mpsc::Sender<Vec<u8>>,
    output: Option<mpsc::Receiver<ServerOutput>>,
    errors: Option<mpsc::Receiver<String>>,
    stop: mpsc::Sender<Stop>,
    stopped: bool,
}

impl RunningServer {
    /// Hands the process to a task of its own and keeps what reaches it.
    fn start(runtime: &Runtime, plugin: String, mut process: ServerProcess) -> Self {
        let (input, input_queue) = mpsc::channel(INPUT_QUEUE);
        let (output_sender, output) = mpsc::channel(OUTPUT_QUEUE);
        let (errors_sender, errors) = mpsc::channel(ERROR_QUEUE);
        let (stop, stops) = mpsc::channel(2);
        let id = runtime.inner.local_servers.add(&plugin, stop.clone());
        let writer = process.take_input();
        let reader = process.take_output();
        let error_reader = process.take_errors();
        let runtime = runtime.clone();
        tokio::spawn(async move {
            let errors_task =
                error_reader.map(|reader| tokio::spawn(read_errors(reader, errors_sender)));
            let output_task =
                reader.map(|reader| tokio::spawn(read_output(reader, output_sender.clone())));
            let ending = serve(process, writer, input_queue, stops).await;
            // The output that the server wrote before it ended reaches the
            // plugin before its end.
            if let Some(task) = output_task {
                let _read = tokio::time::timeout(RELEASE_GRACE, task).await;
            }
            if let Some(task) = errors_task {
                task.abort();
            }
            let _sent = output_sender.send(ServerOutput::Ended { ending }).await;
            runtime.inner.local_servers.remove(&plugin, id);
            tracing::info!(plugin, ?ending, "a local server ended");
        });
        Self {
            input,
            output: Some(output),
            errors: Some(errors),
            stop,
            stopped: false,
        }
    }
}

/// Runs a server until it ends on its own or a stop arrives: writes what
/// the plugin sends to its input, in order. Returns how it ended. The queue
/// of its input closes when the plugin drops the server or stops it, and
/// either sends a stop of its own, so the closed queue only ends the
/// writing.
async fn serve(
    mut process: ServerProcess,
    mut writer: Option<mannie_tools::ServerInput>,
    mut input_queue: mpsc::Receiver<Vec<u8>>,
    mut stops: mpsc::Receiver<Stop>,
) -> ServerEnding {
    let mut queue_open = true;
    loop {
        tokio::select! {
            exited = process.finished() => {
                // What the server started may still run after it exited.
                process.end(Duration::ZERO).await;
                return exited.map_or(ServerEnding::Ended, |code| ServerEnding::Exited { code });
            }
            stop = stops.recv() => {
                let (grace, ended) = match stop {
                    Some(stop) => (stop.grace.min(MAX_SERVER_STOP_GRACE), stop.ended),
                    None => (Duration::ZERO, None),
                };
                // Closing the input first lets a server that reads to the
                // end exit by itself.
                drop(writer.take());
                let ending = match process.end(grace).await {
                    Some(code) => ServerEnding::Exited { code },
                    None => ServerEnding::Ended,
                };
                if let Some(ended) = ended {
                    let _told = ended.send(ending);
                }
                // Stops that arrived meanwhile get the same end.
                stops.close();
                while let Some(stop) = stops.recv().await {
                    if let Some(ended) = stop.ended {
                        let _told = ended.send(ending);
                    }
                }
                return ending;
            }
            bytes = input_queue.recv(), if queue_open && writer.is_some() => {
                let Some(bytes) = bytes else {
                    queue_open = false;
                    continue;
                };
                if let Some(input) = writer.as_mut() {
                    let written = async {
                        input.write_all(&bytes).await?;
                        input.flush().await
                    }
                    .await;
                    if let Err(error) = written {
                        tracing::debug!(%error, "a local server's input closed");
                        writer = None;
                    }
                }
            }
        }
    }
}

/// Reads a server's output into its queue until the output ends.
async fn read_output(mut reader: mannie_tools::ServerStream, queue: mpsc::Sender<ServerOutput>) {
    let mut buffer = vec![0_u8; OUTPUT_CHUNK_BYTES];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => {
                let bytes = buffer[..read].to_vec();
                if queue.send(ServerOutput::Bytes { bytes }).await.is_err() {
                    return;
                }
            }
        }
    }
}

/// Reads a server's error output as lines, each cut to
/// [`MAX_SERVER_ERROR_LINE_BYTES`], until it ends. A line that finds the
/// queue full is dropped.
async fn read_errors(reader: mannie_tools::ServerStream, queue: mpsc::Sender<String>) {
    let mut lines = BufReader::new(reader);
    let mut line = Vec::new();
    loop {
        line.clear();
        match lines.read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_read) => {
                let text = String::from_utf8_lossy(&line);
                let text = text.trim_end_matches(['\r', '\n']);
                let mut end = text.len().min(MAX_SERVER_ERROR_LINE_BYTES);
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                if queue.try_send(text[..end].to_owned()).is_err() && queue.is_closed() {
                    return;
                }
            }
        }
    }
}

impl LocalServer for RunningServer {
    fn send(&self, bytes: Vec<u8>) -> BoxFuture<'static, Result<(), LocalServerError>> {
        let input = self.input.clone();
        async move {
            if bytes.len() > MAX_SERVER_SEND_BYTES {
                return Err(LocalServerError::TooLarge);
            }
            input
                .send(bytes)
                .await
                .map_err(|_closed| LocalServerError::Ended)
        }
        .boxed()
    }

    fn output(&mut self) -> BoxStream<'static, ServerOutput> {
        match self.output.take() {
            Some(receiver) => receiver_stream(receiver),
            None => stream::empty().boxed(),
        }
    }

    fn errors(&mut self) -> BoxStream<'static, String> {
        match self.errors.take() {
            Some(receiver) => receiver_stream(receiver),
            None => stream::empty().boxed(),
        }
    }

    fn stop(mut self: Box<Self>, grace: Duration) -> BoxFuture<'static, ServerEnding> {
        self.stopped = true;
        let stop = self.stop.clone();
        async move {
            let (sender, receiver) = oneshot::channel();
            let request = Stop {
                grace,
                ended: Some(sender),
            };
            if stop.send(request).await.is_err() {
                return ServerEnding::Ended;
            }
            receiver.await.unwrap_or(ServerEnding::Ended)
        }
        .boxed()
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        if !self.stopped {
            // A full queue holds a stop already, which ends the server too.
            let _requested = self.stop.try_send(Stop {
                grace: Duration::ZERO,
                ended: None,
            });
        }
    }
}

/// A queue as a stream that ends when the queue closes.
fn receiver_stream<T: Send + 'static>(receiver: mpsc::Receiver<T>) -> FuturesBoxStream<'static, T> {
    stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|item| (item, receiver))
    })
    .boxed()
}
