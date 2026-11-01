//! A terminal's shell in a pseudo console (ADR-0053, decision 1): typed
//! commands run and their output arrives, Ctrl+C interrupts a command, a
//! resize reaches the shell, the shell's exit code arrives, and ending the
//! terminal ends every process that it started, also one outside the pseudo
//! console (V4, ADR-0014).

#![cfg(any(windows, target_os = "linux"))]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mannie_tools::{TerminalLaunch, TerminalProcess};
use tokio::sync::mpsc::Receiver;

/// A terminal with the output that arrived so far.
struct Session {
    terminal: TerminalProcess,
    output: Receiver<Vec<u8>>,
    seen: String,
}

impl Session {
    fn start(program: PathBuf, arguments: Vec<OsString>) -> Self {
        let launch = TerminalLaunch {
            program,
            arguments,
            working_directory: std::env::temp_dir(),
            environment: environment(),
            columns: 120,
            rows: 30,
        };
        let mut terminal = TerminalProcess::spawn(&launch).expect("the shell starts");
        let output = terminal.take_output().expect("the output is taken once");
        Self {
            terminal,
            output,
            seen: String::new(),
        }
    }

    async fn type_line(&self, line: &str) {
        let enter = if cfg!(windows) { "\r" } else { "\n" };
        self.terminal
            .input()
            .write(format!("{line}{enter}").into_bytes())
            .await
            .expect("the terminal takes the line");
    }

    /// Reads output until it holds `needle` after what was seen before, or
    /// fails after `seconds`.
    async fn expect(&mut self, needle: &str, seconds: u64) {
        let start = self.seen.len();
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while !self.seen[start..].contains(needle) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "{needle:?} did not arrive; the terminal showed {:?}",
                &self.seen[start..]
            );
            match tokio::time::timeout(left, self.output.recv()).await {
                Ok(Some(chunk)) => self.seen.push_str(&String::from_utf8_lossy(&chunk)),
                Ok(None) => {
                    assert!(
                        self.seen[start..].contains(needle),
                        "the terminal closed before {needle:?} arrived"
                    );
                }
                Err(_) => {}
            }
        }
    }
}

/// The variables that a shell needs to start, from this process.
fn environment() -> Vec<(OsString, OsString)> {
    let names: &[&str] = if cfg!(windows) {
        &[
            "SystemRoot",
            "PATH",
            "TEMP",
            "TMP",
            "USERPROFILE",
            "COMSPEC",
        ]
    } else {
        &["PATH", "HOME", "USER"]
    };
    let mut variables: Vec<(OsString, OsString)> = names
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value)))
        .collect();
    variables.push(("TERM".into(), "xterm-256color".into()));
    variables
}

#[cfg(windows)]
fn system_program(name: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("System32").join(name)
}

#[cfg(windows)]
fn powershell() -> PathBuf {
    system_program(r"WindowsPowerShell\v1.0\powershell.exe")
}

/// A shell that reads commands from its terminal: `cmd.exe` on Windows, and
/// `sh` elsewhere.
fn plain_shell() -> Session {
    #[cfg(windows)]
    return Session::start(system_program("cmd.exe"), Vec::new());
    #[cfg(not(windows))]
    Session::start(PathBuf::from("/bin/sh"), vec!["-i".into()])
}

/// The arithmetic that a plain shell prints, which the typed line itself
/// does not show.
fn product(a: u32, b: u32) -> String {
    if cfg!(windows) {
        format!("set /a {a}*{b}")
    } else {
        format!("echo $(({a}*{b}))")
    }
}

fn pid_written(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

async fn wait_for_pid(path: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(pid) = pid_written(path) {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "{} was not written",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(windows)]
fn is_alive(pid: u32) -> bool {
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .expect("tasklist runs");
    String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
}

#[cfg(target_os = "linux")]
fn is_alive(pid: u32) -> bool {
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .is_some_and(|state| state != "Z"),
        Err(_) => false,
    }
}

async fn assert_ended(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while is_alive(pid) {
        assert!(
            Instant::now() < deadline,
            "process {pid} is still alive after its terminal ended"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn typed_commands_run_and_their_output_arrives() {
    let mut session = plain_shell();

    session.type_line(&product(6, 7)).await;
    session.expect("42", 20).await;

    let ended = session.terminal.end(Duration::from_secs(2)).await;
    let _ = ended;
    // The reading thread sees the pseudo console close.
    let closed = tokio::time::timeout(Duration::from_secs(10), async {
        while session.output.recv().await.is_some() {}
    })
    .await;
    assert!(
        closed.is_ok(),
        "the output did not end after the terminal ended"
    );
}

#[tokio::test]
async fn ctrl_c_interrupts_a_running_command() {
    let mut session = plain_shell();
    let long = if cfg!(windows) {
        "ping -n 60 127.0.0.1"
    } else {
        "sleep 60"
    };
    session.type_line(long).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;

    session
        .terminal
        .input()
        .write(b"\x03".to_vec())
        .await
        .unwrap();
    session.type_line(&product(6, 8)).await;

    // Without the interrupt, the second command would wait for the first.
    session.expect("48", 10).await;
    session.terminal.end(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn the_shell_exit_code_arrives() {
    let mut session = plain_shell();
    session.type_line("exit 3").await;

    let code = tokio::time::timeout(Duration::from_secs(20), session.terminal.finished())
        .await
        .unwrap();

    assert_eq!(code, Some(3));
}

#[tokio::test]
async fn a_resize_reaches_the_shell() {
    #[cfg(windows)]
    let mut session = Session::start(powershell(), vec!["-NoLogo".into(), "-NoProfile".into()]);
    #[cfg(not(windows))]
    let mut session = plain_shell();

    session.terminal.resize(97, 33).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    if cfg!(windows) {
        session
            .type_line("'cols=' + [Console]::WindowWidth + ' rows=' + [Console]::WindowHeight")
            .await;
        session.expect("cols=97 rows=33", 20).await;
    } else {
        session.type_line("stty size").await;
        session.expect("33 97", 20).await;
    }
    session.terminal.end(Duration::from_secs(2)).await;
}

#[cfg(windows)]
#[tokio::test]
async fn ending_the_terminal_ends_a_process_in_a_console_of_its_own() {
    let folder = tempfile::tempdir().unwrap();
    let pid_file = folder.path().join("grandchild.pid");
    let mut session = Session::start(powershell(), vec!["-NoLogo".into(), "-NoProfile".into()]);
    // Start-Process gives the sleeper a hidden console of its own, outside
    // the pseudo console, so only the job reaches it.
    session
        .type_line(&format!(
            "Start-Process -WindowStyle Hidden -FilePath '{}' -ArgumentList '-NoProfile -NonInteractive -Command \"$PID | Out-File -Encoding ascii ''{}''; Start-Sleep -Seconds 60\"'",
            powershell().display(),
            pid_file.display()
        ))
        .await;
    let pid = wait_for_pid(&pid_file).await;
    assert!(is_alive(pid));

    session.terminal.end(Duration::from_secs(1)).await;

    assert_ended(pid).await;
    let _ = &mut session.output;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn ending_the_terminal_ends_a_process_in_a_session_of_its_own() {
    let folder = tempfile::tempdir().unwrap();
    let pid_file = folder.path().join("grandchild.pid");
    let session = plain_shell();
    // A new session leaves the terminal's process group, so only the
    // subreaper keeps it in the shell's tree.
    session
        .type_line(&format!(
            "setsid sh -c 'echo $$ > \"{}\"; exec sleep 60' &",
            pid_file.display()
        ))
        .await;
    let pid = wait_for_pid(&pid_file).await;
    assert!(is_alive(pid));

    session.terminal.end(Duration::from_secs(1)).await;

    assert_ended(pid).await;
}

#[test]
fn this_machine_lists_its_shells_with_one_default() {
    let shells = mannie_tools::local_shells(&|_program| true);

    assert_eq!(
        shells.iter().filter(|shell| shell.default).count(),
        1,
        "{shells:?}"
    );
    assert!(
        shells.iter().all(|shell| shell.program.is_absolute()),
        "{shells:?}"
    );
    if cfg!(windows) {
        let names: Vec<&str> = shells.iter().map(|shell| shell.name.as_str()).collect();
        assert!(names.contains(&"Windows PowerShell"), "{names:?}");
        assert!(names.contains(&"Command Prompt"), "{names:?}");
    }
}
