use std::sync::Arc;

use mannie_types::clock::{Clock, ManualClock, UtcMillis};
use mannie_types::ids::WorkerId;
use mannie_types::workers::{CeilingMode, CeilingView, HostFacts, WorkerDetails};
use tempfile::TempDir;

use super::{NewPairedWorker, WorkersStoreError};
use crate::{Store, StoreOptions};

fn open() -> (TempDir, Store) {
    let folder = tempfile::tempdir().unwrap();
    let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(UtcMillis::from_millis(1_000)));
    let options = StoreOptions::new(folder.path().join("backups"), clock);
    let store = Store::open(&folder.path().join("mannie.sqlite"), &options).unwrap();
    (folder, store)
}

fn worker_id(last: u32) -> WorkerId {
    WorkerId::parse(&format!("0199e0f2-0000-7000-b000-{last:012}")).unwrap()
}

fn details() -> WorkerDetails {
    WorkerDetails {
        purpose: "Builds the backend".to_owned(),
        facts: Some(HostFacts {
            system: "FreeBSD 14.4-RELEASE, x86_64".to_owned(),
            hostname: "build-01".to_owned(),
            shells: vec!["sh".to_owned()],
            tools: vec!["git".to_owned(), "pkg".to_owned()],
        }),
        ceiling: Some(CeilingView {
            mode: CeilingMode::Ask,
            allowed_folders: vec!["/srv/app".to_owned()],
            rules: Vec::new(),
            accepts_master_full_access: true,
        }),
        version: Some("0.4.0".to_owned()),
        max_parallel_tasks: 4,
    }
}

fn new_worker(last: u32, pin: &str) -> NewPairedWorker {
    NewPairedWorker {
        id: worker_id(last),
        name: format!("worker-{last}"),
        chosen_name: None,
        address: format!("host-{last}.lan:7445"),
        key_pin: pin.to_owned(),
        details: details(),
    }
}

#[tokio::test]
async fn a_paired_worker_is_stored_with_its_details() {
    let (_folder, store) = open();

    store
        .add_paired_worker(new_worker(1, &"a".repeat(64)))
        .await
        .unwrap();

    let workers = store.paired_workers().await.unwrap();
    assert_eq!(workers.len(), 1);
    let worker = &workers[0];
    assert_eq!(worker.id, worker_id(1));
    assert_eq!(worker.name, "worker-1");
    assert_eq!(worker.chosen_name, None);
    assert_eq!(worker.address, "host-1.lan:7445");
    assert_eq!(worker.details, details());
    assert_eq!(worker.paired_at, UtcMillis::from_millis(1_000));
    assert_eq!(worker.connected_at, None);
}

#[tokio::test]
async fn a_key_pairs_once() {
    let (_folder, store) = open();
    let pin = "b".repeat(64);
    store.add_paired_worker(new_worker(1, &pin)).await.unwrap();

    let again = store.add_paired_worker(new_worker(2, &pin)).await;

    assert!(matches!(again, Err(WorkersStoreError::KeyTaken)));
    assert_eq!(store.paired_workers().await.unwrap().len(), 1);
}

#[tokio::test]
async fn hello_updates_a_worker() {
    let (_folder, store) = open();
    store
        .add_paired_worker(new_worker(1, &"c".repeat(64)))
        .await
        .unwrap();
    let mut newer = details();
    newer.version = Some("0.4.1".to_owned());

    store
        .update_worker_hello(&worker_id(1), "renamed", &newer)
        .await
        .unwrap();

    let worker = &store.paired_workers().await.unwrap()[0];
    assert_eq!(worker.name, "renamed");
    assert_eq!(worker.details.version.as_deref(), Some("0.4.1"));
    assert_eq!(worker.connected_at, Some(UtcMillis::from_millis(1_000)));
    assert!(matches!(
        store
            .update_worker_hello(&worker_id(9), "gone", &newer)
            .await,
        Err(WorkersStoreError::WorkerNotFound)
    ));
}

#[tokio::test]
async fn a_worker_paired_with_the_owners_name_keeps_it_through_its_hello() {
    let (_folder, store) = open();
    let mut named = new_worker(1, &"e".repeat(64));
    named.chosen_name = Some("freebsd-01".to_owned());
    store.add_paired_worker(named).await.unwrap();

    store
        .update_worker_hello(&worker_id(1), "freebsd.local", &details())
        .await
        .unwrap();

    let worker = &store.paired_workers().await.unwrap()[0];
    assert_eq!(worker.name, "freebsd.local");
    assert_eq!(worker.chosen_name.as_deref(), Some("freebsd-01"));
}

#[tokio::test]
async fn the_owners_name_for_a_worker_is_set_and_cleared() {
    let (_folder, store) = open();
    store
        .add_paired_worker(new_worker(1, &"f".repeat(64)))
        .await
        .unwrap();

    store
        .set_worker_chosen_name(&worker_id(1), Some("build box"))
        .await
        .unwrap();
    let named = store.paired_workers().await.unwrap()[0].chosen_name.clone();
    store
        .set_worker_chosen_name(&worker_id(1), None)
        .await
        .unwrap();
    let cleared = store.paired_workers().await.unwrap()[0].chosen_name.clone();

    assert_eq!(named.as_deref(), Some("build box"));
    assert_eq!(cleared, None);
    assert!(matches!(
        store
            .set_worker_chosen_name(&worker_id(9), Some("gone"))
            .await,
        Err(WorkersStoreError::WorkerNotFound)
    ));
}
