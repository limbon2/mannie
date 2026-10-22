//! Commands that run in the background (ADR-0041, docs/features/system-tools/
//! background-commands.md): their output arrives in their log as they write
//! it, their own end reports its exit code, and ending them early ends them.

#![cfg(test)]
#![cfg(any(windows, target_os = "linux", target_os = "freebsd"))]

use std::path::PathBuf;
use std::time::Duration;

use mannie_tools::{BackgroundProcess, BackgroundSpec, CommandEnvironment, LiveLog, SystemFacts};
use tempfile::TempDir;

/// How long a test waits for output that a running command writes. Windows
/// PowerShell 5.1 can take seconds to start when the tests run in parallel.
const PATIENCE: Duration = Duration::from_secs(30);

fn spec(folder: &TempDir, text: &str) -> BackgroundSpec {
    let system = SystemFacts::detect();
    let shell = system.shell.expect("a shell on this platform").path;
    BackgroundSpec {
        shell,
        text: text.to_owned(),
        working_directory: PathBuf::from(mannie_tools::canonical_folder(folder.path()).unwrap()),
        environment: CommandEnvironment::from_process(&[]).variables().to_vec(),
    }
}

/// The texts of the lines that the log holds.
fn texts(log: &LiveLog) -> Vec<String> {
    log.lines_after(0)
        .lines
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

/// Waits until the log holds at least `lines` lines.
async fn wait_for_lines(log: &LiveLog, lines: u64) {
    let mut changes = log.subscribe();
    let waited = tokio::time::timeout(PATIENCE, async {
        while changes.borrow_and_update().lines < lines {
            changes.changed().await.unwrap();
        }
    })
    .await;
    assert!(waited.is_ok(), "the log holds {:?}", texts(log));
}

#[cfg(windows)]
const THREE_LINES: &str =
    "Write-Output 'one'\n[Console]::Error.WriteLine('two')\nWrite-Output 'three'";
#[cfg(unix)]
const THREE_LINES: &str = "echo one\necho two >&2\necho three";

#[cfg(windows)]
const EXIT_THREE: &str = "Write-Output 'bye'\nexit 3";
#[cfg(unix)]
const EXIT_THREE: &str = "echo bye\nexit 3";

#[cfg(windows)]
const TICKS: &str =
    "for ($i = 1; $i -le 600; $i++) { Write-Output \"tick $i\"; Start-Sleep -Milliseconds 100 }";
#[cfg(unix)]
const TICKS: &str = "i=1\nwhile [ $i -le 600 ]; do echo \"tick $i\"; i=$((i + 1)); sleep 0.1; done";

#[tokio::test]
async fn both_streams_become_one_log_of_numbered_lines() {
    let folder = tempfile::tempdir().unwrap();
    let mut process = BackgroundProcess::spawn(&spec(&folder, THREE_LINES))
        .await
        .unwrap();

    let code = process.finished().await;
    let log = process.log().clone();
    process.end(true).await;

    assert_eq!(code, Some(0));
    let mut lines = texts(&log);
    // The two streams are read side by side, so only each stream keeps its
    // order.
    let stdout: Vec<&String> = lines.iter().filter(|line| *line != "two").collect();
    assert_eq!(stdout, ["one", "three"]);
    lines.sort();
    assert_eq!(lines, ["one", "three", "two"]);
    let mark = log.mark();
    assert_eq!(mark.lines, 3);
    assert!(mark.ended);
}

#[tokio::test]
async fn the_command_reports_its_exit_code() {
    let folder = tempfile::tempdir().unwrap();
    let mut process = BackgroundProcess::spawn(&spec(&folder, EXIT_THREE))
        .await
        .unwrap();

    let code = process.finished().await;
    let log = process.log().clone();
    process.end(true).await;

    assert_eq!(code, Some(3));
    assert_eq!(texts(&log), ["bye"]);
}

#[tokio::test]
async fn output_arrives_while_the_command_runs_and_ending_it_stops_it() {
    let folder = tempfile::tempdir().unwrap();
    let process = BackgroundProcess::spawn(&spec(&folder, TICKS))
        .await
        .unwrap();
    let log = process.log().clone();

    wait_for_lines(&log, 3).await;
    let before = log.mark();
    assert!(!before.ended);
    assert_eq!(log.lines_from(1, 2).lines[0].1, "tick 1");
    let started = std::time::Instant::now();
    process.end(false).await;

    assert!(
        started.elapsed() < Duration::from_secs(5),
        "ending took {:?}",
        started.elapsed()
    );
    let after = log.mark();
    assert!(after.ended);
    // Nothing more arrives once the command ended.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(log.mark(), after);
    assert!(after.lines < 600);
}

#[tokio::test]
async fn a_new_read_returns_only_the_lines_after_the_last_one_read() {
    let folder = tempfile::tempdir().unwrap();
    let process = BackgroundProcess::spawn(&spec(&folder, TICKS))
        .await
        .unwrap();
    let log = process.log().clone();

    wait_for_lines(&log, 2).await;
    let first = log.lines_after(0);
    let last_read = first.lines.last().unwrap().0;
    wait_for_lines(&log, last_read + 2).await;
    let second = log.lines_after(last_read);
    process.end(false).await;

    assert_eq!(second.lines[0].0, last_read + 1);
    assert_eq!(second.lines[0].1, format!("tick {}", last_read + 1));
    assert_eq!(second.left_out, None);
}
