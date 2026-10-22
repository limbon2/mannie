//! V4 (docs/features/system-tools/commands.md, tests): when a command ends,
//! on its own, at its timeout or by Stop, every process it started ends with
//! it, including processes that detach on purpose. A command that runs in the
//! background ends the same way, on its own, when it is ended, and when it is
//! dropped (ADR-0041).
//!
//! Each test runs a command that starts a grandchild of one shape, which
//! writes its process ID to a file and then sleeps. Once the command ended,
//! the grandchild must be gone.

#![cfg(test)]
#![cfg(any(windows, target_os = "linux", target_os = "freebsd"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mannie_store::{Store, StoreOptions};
use mannie_tools::{
    BackgroundProcess, BackgroundSpec, CommandEnvironment, Done, Execution, MutationLock,
    ProjectFacts, StopReason, StopSignal, SystemFacts, ToolContext, ToolRegistry,
};
use mannie_types::clock::{Clock, SystemClock};
use mannie_types::ids::{ArchiveId, ConversationId};
use mannie_types::tools::{CommandEnding, ToolOutcome};
use serde_json::json;
use tempfile::TempDir;

/// The timeout of a command that ends by it. The grandchild must start
/// before it, and Windows PowerShell 5.1 can take seconds to start when the
/// machine is busy, as when the tests run in parallel.
const TIMEOUT_SECONDS: u64 = 10;

/// How a test ends its command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// The command ends on its own once the grandchild runs.
    Normal,
    /// The command sleeps past its timeout, [`TIMEOUT_SECONDS`].
    Timeout,
    /// The command sleeps until Stop.
    Stop,
}

/// How a test ends a command that runs in the background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackgroundEnd {
    /// The command ends on its own once the grandchild runs.
    Normal,
    /// The command sleeps until it is ended.
    Stop,
    /// The command sleeps until it is dropped.
    Drop,
}

/// A workspace, a store and the system tools.
struct Harness {
    _folder: TempDir,
    root: PathBuf,
    context: ToolContext,
    registry: ToolRegistry,
}

impl Harness {
    fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        let workspace = folder.path().join("work");
        fs::create_dir(&workspace).unwrap();
        let root = PathBuf::from(mannie_tools::canonical_folder(&workspace).unwrap());
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let options = StoreOptions::new(folder.path().join("backups"), clock);
        let store = Store::open(&folder.path().join("mannie.sqlite"), &options).unwrap();
        let context = ToolContext {
            conversation_id: ConversationId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap(),
            workspace: Some(root.clone()),
            store,
            result_limit: mannie_tools::RESULT_LIMIT_BYTES,
            environment: Arc::new(CommandEnvironment::from_process(&[])),
            mutations: MutationLock::new(),
            archive_ids: Arc::new(|| {
                ArchiveId::parse("0199e0f2-0000-7000-a000-000000000001").unwrap()
            }),
            system: Arc::new(SystemFacts::detect()),
            projects: ProjectFacts::none(),
            background: mannie_tools::no_background_commands(),
        };
        Self {
            _folder: folder,
            root,
            context,
            registry: ToolRegistry::with_system_tools(),
        }
    }

    /// Runs a command that starts a grandchild, ends it the way `end` says,
    /// and returns the result with the grandchild's process ID.
    async fn run(&self, start_grandchild: &str, end: End) -> (Done, u32) {
        let pid_file = self.root.join("grandchild.pid");
        let wait = wait_for_file(&pid_file);
        let rest = match end {
            End::Normal => "",
            End::Timeout | End::Stop => sleep_long(),
        };
        let command = format!("{start_grandchild}\n{wait}\n{rest}");
        let timeout = if end == End::Timeout {
            TIMEOUT_SECONDS
        } else {
            60
        };
        let arguments = json!({ "command": command, "timeout_seconds": timeout });
        let tool = self.registry.get("run_command").unwrap();
        let prepared = tool
            .prepare(self.context.clone(), arguments.to_string())
            .await
            .unwrap();
        let stop = StopSignal::new();
        let execution = tool.execute(Execution {
            context: self.context.clone(),
            arguments: arguments.to_string(),
            accesses: prepared.accesses,
            stop: stop.clone(),
            process: None,
            part_id: None,
        });
        let stopper = async {
            if end == End::Stop {
                // Stop once the grandchild runs.
                let deadline = Instant::now() + Duration::from_secs(30);
                while !pid_file_written(&pid_file) && Instant::now() < deadline {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                let stopped_at = Instant::now();
                stop.stop(StopReason::Stopped);
                Some(stopped_at)
            } else {
                None
            }
        };
        let (done, stopped_at) = tokio::join!(execution, stopper);
        if let Some(stopped_at) = stopped_at {
            assert!(
                stopped_at.elapsed() < Duration::from_secs(1),
                "Stop took {:?}",
                stopped_at.elapsed()
            );
        }
        let ending = match &done.outcome {
            ToolOutcome::CommandRan { ending, .. } => *ending,
            other => panic!("the command did not run: {other:?}: {}", done.output),
        };
        let expected = match end {
            End::Normal => CommandEnding::Exited,
            End::Timeout => CommandEnding::TimedOut,
            End::Stop => CommandEnding::Stopped,
        };
        assert_eq!(ending, expected, "{}", done.output);
        let pid = fs::read_to_string(&pid_file)
            .unwrap_or_else(|error| panic!("no process ID: {error}: {}", done.output))
            .trim()
            .parse()
            .unwrap();
        (done, pid)
    }
}

impl Harness {
    /// Runs a command in the background that starts a grandchild, ends it the
    /// way `end` says, and returns the grandchild's process ID.
    async fn run_background(&self, start_grandchild: &str, end: BackgroundEnd) -> u32 {
        let pid_file = self.root.join("grandchild.pid");
        let wait = wait_for_file(&pid_file);
        let rest = match end {
            BackgroundEnd::Normal => "",
            BackgroundEnd::Stop | BackgroundEnd::Drop => sleep_long(),
        };
        let spec = BackgroundSpec {
            shell: self.context.system.shell.clone().unwrap().path,
            text: format!(
                "{start_grandchild}
{wait}
{rest}"
            ),
            working_directory: self.root.clone(),
            environment: self.context.environment.variables().to_vec(),
        };
        let mut process = BackgroundProcess::spawn(&spec).await.unwrap();
        if end == BackgroundEnd::Normal {
            let code = tokio::time::timeout(Duration::from_secs(60), process.finished())
                .await
                .expect("the command ends on its own");
            process.end(true).await;
            assert_eq!(code, Some(0));
        } else {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !pid_file_written(&pid_file) && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if end == BackgroundEnd::Stop {
                let stopped_at = Instant::now();
                process.end(false).await;
                assert!(
                    stopped_at.elapsed() < Duration::from_secs(1),
                    "ending took {:?}",
                    stopped_at.elapsed()
                );
            } else {
                drop(process);
            }
        }
        fs::read_to_string(&pid_file)
            .unwrap_or_else(|error| panic!("no process ID: {error}"))
            .trim()
            .parse()
            .unwrap()
    }
}

fn pid_file_written(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|text| text.trim().parse::<u32>().is_ok())
}

/// Asserts that a process ended. A process may take a moment to disappear
/// after it was killed.
fn assert_ended(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while is_alive(pid) {
        assert!(
            Instant::now() < deadline,
            "process {pid} is still alive after its command ended"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Asserts that a process ended, while the runtime goes on running tasks,
/// such as the end that dropping a command that runs in the background
/// spawns.
async fn assert_ended_soon(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while is_alive(pid) {
        assert!(
            Instant::now() < deadline,
            "process {pid} is still alive after its command ended"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(windows)]
fn is_alive(pid: u32) -> bool {
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
}

#[cfg(target_os = "linux")]
fn is_alive(pid: u32) -> bool {
    // A zombie has ended; only its entry waits to be reaped.
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .is_some_and(|state| state != "Z"),
        Err(_) => false,
    }
}

#[cfg(target_os = "freebsd")]
fn is_alive(pid: u32) -> bool {
    // FreeBSD mounts no `/proc` by default, so `ps` answers. A zombie has
    // ended; only its entry waits to be reaped.
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "state="])
        .output()
        .unwrap();
    let state = String::from_utf8_lossy(&output.stdout);
    let state = state.trim();
    !state.is_empty() && !state.starts_with('Z')
}

#[cfg(windows)]
fn wait_for_file(path: &Path) -> String {
    format!(
        "while (-not ((Test-Path '{0}') -and (Get-Content '{0}'))) {{ Start-Sleep -Milliseconds 50 }}",
        path.display()
    )
}

#[cfg(unix)]
fn wait_for_file(path: &Path) -> String {
    format!("while [ ! -s '{}' ]; do sleep 0.05; done", path.display())
}

#[cfg(windows)]
fn sleep_long() -> &'static str {
    "Start-Sleep -Seconds 60"
}

#[cfg(unix)]
fn sleep_long() -> &'static str {
    "sleep 60"
}

/// Runs a shape with every way to end, in the foreground and in the
/// background, and checks that the grandchild ended each time.
async fn every_end_ends(start_grandchild: impl Fn(&Path) -> String) {
    for end in [End::Normal, End::Timeout, End::Stop] {
        let harness = Harness::new();
        let start = start_grandchild(&harness.root.join("grandchild.pid"));
        let (_, pid) = harness.run(&start, end).await;
        assert_ended(pid);
    }
    for end in [
        BackgroundEnd::Normal,
        BackgroundEnd::Stop,
        BackgroundEnd::Drop,
    ] {
        let harness = Harness::new();
        let start = start_grandchild(&harness.root.join("grandchild.pid"));
        let pid = harness.run_background(&start, end).await;
        assert_ended_soon(pid).await;
    }
}

#[cfg(windows)]
mod windows {
    use std::path::Path;

    use super::{End, Harness, assert_ended, every_end_ends};

    /// Windows PowerShell, as a PowerShell expression.
    const POWERSHELL: &str = r"$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe";

    /// The arguments of a sleeping Windows PowerShell that writes its process
    /// ID to a file, as a PowerShell string literal.
    fn sleeper_arguments(pid_file: &Path) -> String {
        format!(
            "'-NoProfile -NonInteractive -Command \"$PID | Out-File -Encoding ascii ''{}''; Start-Sleep -Seconds 60\"'",
            pid_file.display()
        )
    }

    /// The whole command line of the sleeper, as a PowerShell expression.
    fn sleeper_command_line(pid_file: &Path) -> String {
        format!(
            "('\"' + \"{POWERSHELL}\" + '\" ' + {})",
            sleeper_arguments(pid_file)
        )
    }

    /// C# that starts a process the way the shapes below need: in a job of
    /// its own, or asking to break away from the command's job.
    const SPAWNER: &str = r#"Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class MannieSpawner {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct StartupInfo {
        public int cb; public string reserved; public string desktop; public string title;
        public int x; public int y; public int xSize; public int ySize; public int xChars; public int yChars;
        public int fill; public int flags; public short show; public short reserved2Size;
        public IntPtr reserved2; public IntPtr input; public IntPtr output; public IntPtr error;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInformation { public IntPtr process; public IntPtr thread; public int processId; public int threadId; }
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool CreateProcessW(string application, StringBuilder commandLine, IntPtr processAttributes, IntPtr threadAttributes, bool inherit, uint flags, IntPtr environment, string directory, ref StartupInfo startup, out ProcessInformation information);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr CreateJobObjectW(IntPtr attributes, IntPtr name);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern uint ResumeThread(IntPtr thread);
    public static int Spawn(string commandLine, bool breakaway, bool nestedJob) {
        StartupInfo startup = new StartupInfo();
        startup.cb = Marshal.SizeOf(typeof(StartupInfo));
        ProcessInformation information;
        uint flags = 0x08000000 | 0x00000004;
        if (breakaway) { flags |= 0x01000000; }
        if (!CreateProcessW(null, new StringBuilder(commandLine), IntPtr.Zero, IntPtr.Zero, false, flags, IntPtr.Zero, null, ref startup, out information)) {
            return -Marshal.GetLastWin32Error();
        }
        if (nestedJob) {
            IntPtr job = CreateJobObjectW(IntPtr.Zero, IntPtr.Zero);
            if (!AssignProcessToJobObject(job, information.process)) { return -Marshal.GetLastWin32Error(); }
        }
        ResumeThread(information.thread);
        return information.processId;
    }
}
'@"#;

    #[tokio::test]
    async fn a_process_started_with_start_process_ends_with_the_command() {
        every_end_ends(|pid_file| {
            format!(
                "Start-Process -WindowStyle Hidden -FilePath \"{POWERSHELL}\" -ArgumentList {}",
                sleeper_arguments(pid_file)
            )
        })
        .await;
    }

    #[tokio::test]
    async fn a_process_in_a_nested_job_ends_with_the_command() {
        every_end_ends(|pid_file| {
            format!(
                "{SPAWNER}\n$started = [MannieSpawner]::Spawn({}, $false, $true)\nif ($started -lt 0) {{ throw \"nested job: $started\" }}",
                sleeper_command_line(pid_file)
            )
        })
        .await;
    }

    #[tokio::test]
    async fn a_process_cannot_break_away_from_the_command() {
        let harness = Harness::new();
        let refused = harness.root.join("refused.txt");
        let pid_file = harness.root.join("grandchild.pid");
        let start = format!(
            "{SPAWNER}\n$started = [MannieSpawner]::Spawn({}, $true, $false)\n$started | Out-File -Encoding ascii '{}'\nif ($started -lt 0) {{ $PID | Out-File -Encoding ascii '{}' }}",
            sleeper_command_line(&pid_file),
            refused.display(),
            pid_file.display()
        );

        let (done, _) = harness.run(&start, End::Normal).await;

        let result: i32 = std::fs::read_to_string(&refused)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // ERROR_ACCESS_DENIED: the job lets no process break away.
        assert_eq!(result, -5, "{}", done.output);
        if result > 0 {
            assert_ended(u32::try_from(result).unwrap());
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::every_end_ends;

    #[tokio::test]
    async fn a_background_process_ends_with_the_command() {
        every_end_ends(|pid_file| {
            format!(
                "sleep 60 &
echo $! > '{}'",
                pid_file.display()
            )
        })
        .await;
    }

    /// Linux has `setsid(1)`; FreeBSD detaches a process with `daemon(8)`,
    /// which forks and calls `setsid` before it runs the command.
    #[tokio::test]
    async fn a_process_in_a_new_session_ends_with_the_command() {
        let detach = if cfg!(target_os = "freebsd") {
            "daemon -f"
        } else {
            "setsid"
        };
        every_end_ends(|pid_file| {
            format!(
                "{detach} sh -c 'echo $$ > \"{}\"; exec sleep 60' &",
                pid_file.display()
            )
        })
        .await;
    }

    #[tokio::test]
    async fn a_double_forked_process_ends_with_the_command() {
        every_end_ends(|pid_file| {
            format!("sh -c 'sleep 60 & echo $! > \"{}\"' &", pid_file.display())
        })
        .await;
    }

    #[tokio::test]
    async fn a_nohup_process_ends_with_the_command() {
        every_end_ends(|pid_file| {
            format!(
                "nohup sh -c 'echo $$ > \"{}\"; exec sleep 60' >/dev/null 2>&1 &",
                pid_file.display()
            )
        })
        .await;
    }

    #[tokio::test]
    async fn a_process_that_ignores_sigterm_ends_with_the_command() {
        every_end_ends(|pid_file| {
            format!(
                "sh -c 'trap \"\" TERM; echo $$ > \"{}\"; while :; do sleep 1; done' &",
                pid_file.display()
            )
        })
        .await;
    }
}
