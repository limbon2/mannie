#![cfg(test)]
//! The worker binary in a terminal: it starts, logs and opens its store, stops
//! cleanly when asked, and refuses a database from a newer version of Mannie.

use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const DATABASE_FILE: &str = "mannie-worker.sqlite";

/// How long the worker may take to start.
const START_TIMEOUT: Duration = Duration::from_secs(30);

fn worker(data_directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mannie-worker"));
    command
        .arg("run")
        .arg("--data-dir")
        .arg(data_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Everything in the worker's log files so far.
fn log_text(data_directory: &Path) -> String {
    let Ok(entries) = fs::read_dir(data_directory.join("logs")) else {
        return String::new();
    };
    let mut text = String::new();
    for entry in entries {
        text.push_str(
            &fs::read_to_string(entry.expect("a log folder entry").path())
                .expect("a readable log file"),
        );
    }
    text
}

fn wait_for_log_line(data_directory: &Path, child: &mut Child, message: &str) {
    let deadline = Instant::now() + START_TIMEOUT;
    while !log_text(data_directory).contains(message) {
        if let Some(status) = child.try_wait().expect("the worker's status") {
            panic!("the worker exited with {status} before it logged {message:?}");
        }
        assert!(
            Instant::now() < deadline,
            "the worker did not log {message:?} in time"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// Writes an empty SQLite database whose user version, the schema version
/// that Mannie keeps (ADR-0009), is 999: the 100-byte header of
/// <https://www.sqlite.org/fileformat2.html> and one page with the empty
/// schema table.
fn write_database_from_a_newer_version(path: &Path) {
    const PAGE_SIZE: u16 = 4096;
    let mut page = vec![0_u8; usize::from(PAGE_SIZE)];
    page[..16].copy_from_slice(b"SQLite format 3\0");
    page[16..18].copy_from_slice(&PAGE_SIZE.to_be_bytes());
    // File format versions 1 (rollback journal), no reserved bytes, and the
    // fixed payload fractions.
    page[18..24].copy_from_slice(&[1, 1, 0, 64, 32, 32]);
    // The change counter, the size in pages, and the change counter that
    // makes the size valid.
    page[24..28].copy_from_slice(&1_u32.to_be_bytes());
    page[28..32].copy_from_slice(&1_u32.to_be_bytes());
    page[92..96].copy_from_slice(&1_u32.to_be_bytes());
    // Schema format 4, UTF-8 text, and the user version.
    page[44..48].copy_from_slice(&4_u32.to_be_bytes());
    page[56..60].copy_from_slice(&1_u32.to_be_bytes());
    page[60..64].copy_from_slice(&999_u32.to_be_bytes());
    // The SQLite version that wrote the file.
    page[96..100].copy_from_slice(&3_046_000_u32.to_be_bytes());
    // The schema table: an empty leaf table page whose cell content area
    // starts at the end of the page.
    page[100] = 0x0D;
    page[105..107].copy_from_slice(&PAGE_SIZE.to_be_bytes());
    fs::write(path, page).expect("the planted database");
}

#[cfg(unix)]
#[test]
fn starts_and_stops_on_a_termination_signal() {
    use std::os::unix::fs::PermissionsExt;

    let folder = tempfile::tempdir().unwrap();
    let data_directory = folder.path().join("worker");
    let mut child = worker(&data_directory).spawn().unwrap();
    wait_for_log_line(&data_directory, &mut child, "the worker started");

    let signalled = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status()
        .unwrap();
    assert!(signalled.success(), "kill could not signal the worker");
    let output = child.wait_with_output().unwrap();

    assert!(
        output.status.success(),
        "the worker exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        log_text(&data_directory).contains("the worker stopped"),
        "the worker logged that it stopped"
    );
    let folder_mode = fs::metadata(&data_directory).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        folder_mode, 0o700,
        "only the worker's account can open the data folder"
    );
    let database_mode = fs::metadata(data_directory.join(DATABASE_FILE))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        database_mode, 0o600,
        "only the worker's account can read the database"
    );
}

// Windows has no termination signal that a test can send to a console
// process; the unit test `worker::tests::starts_and_stops_in_order` covers
// the clean stop in process.
#[cfg(windows)]
#[test]
fn starts_logs_and_opens_its_store() {
    let folder = tempfile::tempdir().unwrap();
    let data_directory = folder.path().join("worker");
    let mut child = worker(&data_directory).spawn().unwrap();

    wait_for_log_line(&data_directory, &mut child, "the worker started");
    child.kill().unwrap();
    child.wait().unwrap();

    assert!(
        data_directory.join(DATABASE_FILE).is_file(),
        "the worker created its database"
    );
}

#[test]
fn refuses_a_database_from_a_newer_version_and_leaves_it_untouched() {
    let folder = tempfile::tempdir().unwrap();
    let data_directory = folder.path().join("worker");
    fs::create_dir_all(&data_directory).unwrap();
    let database = data_directory.join(DATABASE_FILE);
    write_database_from_a_newer_version(&database);
    let planted = fs::read(&database).unwrap();

    let output = worker(&data_directory).output().unwrap();

    assert!(!output.status.success(), "the worker refused to start");
    let error_output = String::from_utf8_lossy(&output.stderr);
    assert!(
        error_output.contains("schema version 999"),
        "the error names the database's version: {error_output}"
    );
    assert!(
        log_text(&data_directory).contains("store.schema_too_new"),
        "the log has the error's code"
    );
    assert_eq!(
        fs::read(&database).unwrap(),
        planted,
        "the database is untouched"
    );
}
