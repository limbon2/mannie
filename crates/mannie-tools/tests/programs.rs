//! Installed programs (ADR-0038, decisions 7 and 8): a program receives its
//! arguments exactly, without a shell; its input and output travel as bytes;
//! its output limit and its timeout end it; and when its run ends, on its
//! own, at its timeout or because its caller dropped it, every process that
//! it started ends with it, as a command's do (V4).
//!
//! The tests run the installed git, which the plugin Git needs, and the
//! system's shell as a program.

#![cfg(test)]
#![cfg(any(windows, target_os = "linux", target_os = "freebsd"))]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mannie_tools::{CommandEnvironment, ProgramEnding, ProgramOutput, ProgramRun, run_program};
use tempfile::TempDir;

/// A folder for a test, in canonical form.
struct Folder {
    _temp: TempDir,
    path: PathBuf,
}

impl Folder {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = PathBuf::from(mannie_tools::canonical_folder(temp.path()).unwrap());
        Self { _temp: temp, path }
    }
}

/// The installed git, which these tests need on the `PATH`.
fn git() -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    mannie_tools::find_program("git", &path, &[])
        .unwrap_or_else(|error| panic!("the tests of programs need git on the PATH: {error}"))
}

/// The environment of commands, with git's configuration kept away from the
/// owner's, so the owner's settings change no result.
fn environment(folder: &Path) -> Vec<(OsString, OsString)> {
    let global = folder.join("gitconfig");
    fs::write(&global, b"").unwrap();
    CommandEnvironment::from_process(&[])
        .for_program(
            &[],
            &[
                (
                    "GIT_CONFIG_GLOBAL".to_owned(),
                    global.to_str().unwrap().to_owned(),
                ),
                ("GIT_CONFIG_NOSYSTEM".to_owned(), "1".to_owned()),
            ],
            std::env::vars_os(),
        )
        .unwrap()
}

fn git_run(folder: &Path, arguments: &[&str]) -> ProgramRun {
    ProgramRun {
        program: git(),
        arguments: arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect(),
        working_directory: folder.to_path_buf(),
        environment: environment(folder),
        input: Vec::new(),
        timeout: Duration::from_secs(60),
        output_limit: 1024 * 1024,
    }
}

/// The system's shell as an installed program, running `script`.
fn shell_run(folder: &Path, script: &str, timeout: Duration) -> ProgramRun {
    #[cfg(windows)]
    let (program, arguments) = {
        let system_root = std::env::var_os("SystemRoot").unwrap();
        (
            PathBuf::from(system_root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe"),
            vec![
                "-NoLogo".to_owned(),
                "-NoProfile".to_owned(),
                "-NonInteractive".to_owned(),
                "-Command".to_owned(),
                script.to_owned(),
            ],
        )
    };
    #[cfg(unix)]
    let (program, arguments) = (
        PathBuf::from("/bin/sh"),
        vec!["-c".to_owned(), script.to_owned()],
    );
    ProgramRun {
        program,
        arguments,
        working_directory: folder.to_path_buf(),
        environment: CommandEnvironment::from_process(&[])
            .for_program(&[], &[], std::env::vars_os())
            .unwrap(),
        input: Vec::new(),
        timeout,
        output_limit: 1024 * 1024,
    }
}

/// How git quotes an argument for a shell (`sq_quote_buf` in git's
/// `quote.c`): in single quotes, with `'` and `!` each closed out and escaped.
fn sq_quote(argument: &str) -> String {
    let mut quoted = String::from("'");
    for character in argument.chars() {
        match character {
            '\'' | '!' => {
                quoted.push_str("'\\");
                quoted.push(character);
                quoted.push('\'');
            }
            other => quoted.push(other),
        }
    }
    quoted.push('\'');
    quoted
}

fn text(output: &ProgramOutput) -> String {
    String::from_utf8(output.output.clone()).unwrap()
}

#[tokio::test]
async fn arguments_reach_the_program_exactly() {
    let folder = Folder::new();
    let corpus = [
        "plain",
        "with space",
        "double\"quote",
        "\"",
        "single'quote",
        "back\\slash",
        "trailing\\",
        "trailing two\\\\",
        "\\\"",
        "% ^ $ & | < > ( ) ; ` *",
        "!bang",
        "%PATH%",
        "$HOME",
        "",
        "été 😀",
        "-c",
        "--output=x",
        "tab\there",
        "line\nbreak",
    ];
    let mut arguments = vec!["rev-parse", "--sq-quote"];
    arguments.extend(corpus);

    let output = run_program(git_run(&folder.path, &arguments))
        .await
        .unwrap();

    assert_eq!(output.ending, ProgramEnding::Exited);
    assert_eq!(
        output.exit_code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.errors)
    );
    let mut expected = String::new();
    for argument in corpus {
        expected.push(' ');
        expected.push_str(&sq_quote(argument));
    }
    assert_eq!(text(&output).trim_end_matches('\n'), expected);
}

#[tokio::test]
async fn input_reaches_the_program_whole_and_output_returns_as_bytes() {
    let folder = Folder::new();
    // Larger than any pipe's buffer, with bytes that are not UTF-8.
    let content: Vec<u8> = (0..3 * 1024 * 1024)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect();
    let file = folder.path.join("content.bin");
    fs::write(&file, &content).unwrap();
    let mut from_input = git_run(&folder.path, &["hash-object", "--stdin"]);
    from_input.input = content;

    let from_input = run_program(from_input).await.unwrap();
    let from_file = run_program(git_run(
        &folder.path,
        &["hash-object", "--", file.to_str().unwrap()],
    ))
    .await
    .unwrap();

    assert_eq!(from_input.exit_code, Some(0));
    assert_eq!(text(&from_input), text(&from_file));
    assert!(!from_input.output_cut());
}

#[tokio::test]
async fn the_program_runs_in_its_working_directory_with_its_environment() {
    let folder = Folder::new();
    let mut run = git_run(&folder.path, &["var", "GIT_EDITOR"]);
    run.environment
        .push(("GIT_EDITOR".into(), "mannie-test-editor".into()));

    let editor = run_program(run).await.unwrap();
    let initialized = run_program(git_run(&folder.path, &["init", "--quiet"]))
        .await
        .unwrap();
    let top = run_program(git_run(&folder.path, &["rev-parse", "--show-toplevel"]))
        .await
        .unwrap();

    assert_eq!(text(&editor).trim(), "mannie-test-editor");
    assert_eq!(initialized.exit_code, Some(0));
    assert_eq!(
        PathBuf::from(text(&top).trim()),
        // git writes forward slashes on Windows.
        PathBuf::from(folder.path.to_str().unwrap().replace('\\', "/")),
    );
}

#[tokio::test]
async fn error_output_stays_apart_with_the_exit_code() {
    let folder = Folder::new();

    let output = run_program(git_run(&folder.path, &["--no-such-option"]))
        .await
        .unwrap();

    assert_eq!(output.ending, ProgramEnding::Exited);
    assert_eq!(output.exit_code, Some(129));
    assert_eq!(output.output, b"");
    assert_ne!(output.errors, b"");
}

#[tokio::test]
async fn output_past_its_limit_ends_the_run() {
    let folder = Folder::new();
    #[cfg(windows)]
    let script =
        "[Console]::Out.Write('x' * 1000000); [Console]::Out.Flush(); Start-Sleep -Seconds 60";
    #[cfg(unix)]
    let script = "head -c 1000000 /dev/zero; sleep 60";
    let mut run = shell_run(&folder.path, script, Duration::from_secs(60));
    run.output_limit = 1000;
    let started = Instant::now();

    let output = run_program(run).await.unwrap();

    assert_eq!(output.ending, ProgramEnding::OutputFull);
    assert_eq!(output.output.len(), 1000);
    assert!(output.output_total > 1000);
    assert!(output.output_cut());
    assert!(started.elapsed() < Duration::from_secs(30));
}

#[tokio::test]
async fn the_timeout_ends_the_run() {
    let folder = Folder::new();
    #[cfg(windows)]
    let script = "Start-Sleep -Seconds 60";
    #[cfg(unix)]
    let script = "sleep 60";
    let started = Instant::now();

    let output = run_program(shell_run(&folder.path, script, Duration::from_secs(5)))
        .await
        .unwrap();

    assert_eq!(output.ending, ProgramEnding::TimedOut);
    assert_eq!(output.exit_code, None);
    assert!(started.elapsed() < Duration::from_secs(30));
}

#[tokio::test]
async fn runs_past_the_limits_of_programs_are_refused_before_they_start() {
    let folder = Folder::new();
    let mut long = git_run(&folder.path, &["--version"]);
    long.timeout = Duration::from_secs(601);
    let mut wide = git_run(&folder.path, &["--version"]);
    wide.output_limit = mannie_tools::MAX_PROGRAM_OUTPUT_BYTES + 1;

    for run in [long, wide] {
        let error = run_program(run).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}

/// How the containment tests end their run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// The program ends on its own once the grandchild runs.
    Normal,
    /// The program sleeps past its timeout.
    Timeout,
    /// The caller drops the run once the grandchild runs.
    Dropped,
}

/// A script that starts a grandchild, which detaches, writes its process ID
/// to `pid_file` and sleeps; waits until the file is written; and then ends
/// or sleeps.
fn grandchild_script(pid_file: &Path, sleep_after: bool) -> String {
    #[cfg(windows)]
    {
        let powershell = r"$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe";
        let rest = if sleep_after {
            "Start-Sleep -Seconds 60"
        } else {
            ""
        };
        format!(
            "Start-Process -WindowStyle Hidden -FilePath \"{powershell}\" -ArgumentList '-NoProfile -NonInteractive -Command \"$PID | Out-File -Encoding ascii ''{pid}''; Start-Sleep -Seconds 60\"'\n\
             while (-not ((Test-Path '{pid}') -and (Get-Content '{pid}'))) {{ Start-Sleep -Milliseconds 50 }}\n\
             {rest}",
            pid = pid_file.display()
        )
    }
    #[cfg(unix)]
    {
        let detach = if cfg!(target_os = "freebsd") {
            "daemon -f"
        } else {
            "setsid"
        };
        let rest = if sleep_after { "sleep 60" } else { "" };
        format!(
            "{detach} sh -c 'echo $$ > \"{pid}\"; exec sleep 60' &\n\
             while [ ! -s '{pid}' ]; do sleep 0.05; done\n\
             {rest}",
            pid = pid_file.display()
        )
    }
}

fn read_pid(pid_file: &Path) -> Option<u32> {
    fs::read_to_string(pid_file).ok()?.trim().parse().ok()
}

/// Runs a program whose grandchild detaches, ends it as `end` says, and
/// returns the grandchild's process ID.
async fn run_with_grandchild(end: End) -> u32 {
    let folder = Folder::new();
    let pid_file = folder.path.join("grandchild.pid");
    let script = grandchild_script(&pid_file, end != End::Normal);
    let timeout = if end == End::Timeout {
        Duration::from_secs(10)
    } else {
        Duration::from_secs(60)
    };
    let run = run_program(shell_run(&folder.path, &script, timeout));
    match end {
        End::Normal | End::Timeout => {
            let output = run.await.unwrap();
            let expected = if end == End::Normal {
                ProgramEnding::Exited
            } else {
                ProgramEnding::TimedOut
            };
            assert_eq!(
                output.ending,
                expected,
                "{}",
                String::from_utf8_lossy(&output.errors)
            );
        }
        End::Dropped => {
            let running = tokio::spawn(run);
            let deadline = Instant::now() + Duration::from_secs(30);
            while read_pid(&pid_file).is_none() && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            running.abort();
            assert!(running.await.unwrap_err().is_cancelled());
        }
    }
    read_pid(&pid_file).expect("the grandchild wrote no process ID")
}

/// Asserts that a process ended. The end of a dropped run happens in a task
/// of the runtime, so the wait yields to it, and a process may take a moment
/// to disappear.
async fn assert_ended(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while is_alive(pid) {
        assert!(
            Instant::now() < deadline,
            "process {pid} is still alive after its run ended"
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
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "state="])
        .output()
        .unwrap();
    let state = String::from_utf8_lossy(&output.stdout);
    let state = state.trim();
    !state.is_empty() && !state.starts_with('Z')
}

#[tokio::test]
async fn a_detached_grandchild_ends_when_the_program_ends() {
    assert_ended(run_with_grandchild(End::Normal).await).await;
}

#[tokio::test]
async fn a_detached_grandchild_ends_at_the_timeout() {
    assert_ended(run_with_grandchild(End::Timeout).await).await;
}

#[tokio::test]
async fn a_detached_grandchild_ends_when_the_run_is_dropped() {
    assert_ended(run_with_grandchild(End::Dropped).await).await;
}
