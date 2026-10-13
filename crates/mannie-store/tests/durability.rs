//! V3: killing the process loses no committed data (ADR-0009).
//!
//! The test runs a child process (this test binary, running the ignored
//! `child_writer` test) that appends events one by one with full durability
//! and prints the sequence number of each commit once its write resolved.
//! The parent kills the child at a different point in each round, reopens the
//! database and checks that every acknowledged event is there, that the
//! stored events have no gaps, and that SQLite's integrity check passes.

// Integration tests are always built with cfg(test); saying so lets Clippy
// treat the helpers below as test code.
#![cfg(test)]

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use mannie_store::validation::{ValidationEvent, ValidationEvents};
use mannie_store::{Durability, Store, StoreOptions};
use mannie_types::clock::SystemClock;

const CHILD_DATABASE: &str = "MANNIE_DURABILITY_CHILD_DATABASE";
const CONVERSATION: &str = "01928c4f-0000-7000-8000-000000000001";
const ROUNDS: u32 = 5;

fn open(database: &Path) -> Store {
    let backups = database.with_file_name("backups");
    Store::open(database, &StoreOptions::new(backups, Arc::new(SystemClock)))
        .expect("the store opens")
}

#[test]
#[ignore = "runs only as the child process of killing_the_writer_loses_no_acknowledged_event"]
fn child_writer() {
    let Ok(database) = std::env::var(CHILD_DATABASE) else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let store = open(Path::new(&database));
        let events = ValidationEvents::create(&store).await.unwrap();
        for sequence in 0..=u32::MAX {
            let event = ValidationEvent {
                conversation: CONVERSATION.to_owned(),
                sequence,
                payload: format!("delta {sequence}"),
            };
            events.append(Durability::Full, vec![event]).await.unwrap();
            println!("committed {sequence}");
        }
    });
}

#[tokio::test]
async fn killing_the_writer_loses_no_acknowledged_event() {
    let test_binary = std::env::current_exe().unwrap();
    for round in 0..ROUNDS {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("mannie.sqlite");
        let kill_after = 20 + round * 23;

        let mut child = Command::new(&test_binary)
            .args([
                "child_writer",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_DATABASE, &database)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut acknowledged = None;
        for line in stdout.lines() {
            let line = line.unwrap();
            if let Some(sequence) = line.strip_prefix("committed ") {
                let sequence: u32 = sequence.trim().parse().unwrap();
                acknowledged = Some(sequence);
                if sequence >= kill_after {
                    child.kill().unwrap();
                    break;
                }
            }
        }
        child.wait().unwrap();
        let acknowledged =
            acknowledged.expect("the child acknowledged writes before it was killed");

        let store = open(&database);
        let events = ValidationEvents::create(&store).await.unwrap();
        let stored = events.sequences(CONVERSATION).await.unwrap();
        let integrity = events.integrity_check().await.unwrap();

        assert_eq!(integrity, ["ok"], "round {round}");
        assert!(
            stored.len() > acknowledged as usize,
            "round {round}: lost acknowledged events: {stored:?}"
        );
        let expected: Vec<u32> = (0..u32::try_from(stored.len()).unwrap()).collect();
        assert_eq!(
            stored, expected,
            "round {round}: the stored events have gaps"
        );
    }
}
