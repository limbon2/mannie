//! The Workers plugin's tables across versions (ADR-0030, decisions 5 and 9;
//! ADR-0031, decisions 8 and 9): the module carries over the migrations of
//! the store's single list before M5, a database of 0.2.0 keeps its paired
//! workers, and the deletions that waited for a worker move into the core's
//! outbox.

#![cfg(test)]

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use mannie_store::{Durability, OutboxEntry, Store, StoreOptions};
use mannie_types::clock::{Clock, ManualClock, UtcMillis};
use mannie_types::ids::ConversationId;
use mannie_workers::store::{WORKERS_MODULE, WorkersStore};

struct Fixture {
    directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            directory: tempfile::tempdir().unwrap(),
        }
    }

    fn database(&self) -> PathBuf {
        self.directory.path().join("mannie.sqlite")
    }

    fn options(&self) -> StoreOptions {
        let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(UtcMillis::from_millis(5_000)));
        StoreOptions::new(self.directory.path().join("backups"), clock)
            .with_modules([WORKERS_MODULE])
    }

    /// Creates the database as a binary up to 0.2.0 left it at `version`,
    /// with the plugin's tables.
    fn open_before_m5(&self, version: u32) -> Store {
        Store::open_as_before_m5(&self.database(), &self.options(), version).unwrap()
    }

    /// Opens the database as the desktop app does, with the plugin's module.
    fn open(&self) -> (Store, WorkersStore) {
        let store = Store::open(&self.database(), &self.options()).unwrap();
        let workers = WorkersStore::new(store.module(&WORKERS_MODULE).unwrap());
        (store, workers)
    }
}

#[test]
fn the_module_carries_the_workers_part_of_the_list_before_m5() {
    let legacy: Vec<u32> = WORKERS_MODULE
        .migrations
        .iter()
        .filter_map(|migration| migration.legacy_version)
        .collect();
    let versions: Vec<u32> = WORKERS_MODULE
        .migrations
        .iter()
        .map(|migration| migration.version)
        .collect();

    assert_eq!(legacy, [19, 20, 22]);
    assert_eq!(versions, [1, 2, 3, 4]);
    assert_eq!(
        WORKERS_MODULE.migrations[3].legacy_version, None,
        "M5's migration has no number of the old list"
    );
    let names: BTreeSet<&str> = [WORKERS_MODULE.name].into();
    assert_eq!(names, ["workers"].into());
}

/// The schema before the owner could name workers (version 21) keeps a
/// paired worker with the name in its configuration. Migration 22 adds the
/// owner's name, which such a worker does not have yet, so it keeps showing
/// its configured name (docs/features/workers/pairing-and-link.md, data).
#[tokio::test]
async fn a_database_with_paired_workers_is_upgraded_for_the_owners_names() {
    let fixture = Fixture::new();
    let older = fixture.open_before_m5(21);
    older
        .module(&WORKERS_MODULE)
        .unwrap()
        .write(Durability::Full, |transaction| {
            transaction.execute(
                "INSERT INTO workers_paired (id, name, address, key_pin, version, details, \
                 paired_at, connected_at, updated_at) \
                 VALUES ('0199e0f2-0000-7000-b000-000000000001', 'build-01', 'build-01.lan:7445', \
                 ?1, 1, '{}', 1, NULL, 1)",
                ["d".repeat(64)],
            )
        })
        .await
        .unwrap();
    drop(older);

    let (_store, workers) = fixture.open();

    let paired = workers.paired_workers().await.unwrap();
    assert_eq!(paired.len(), 1);
    assert_eq!(paired[0].name, "build-01");
    assert_eq!(paired[0].chosen_name, None);
    assert!(workers.has_paired_workers().await.unwrap());
}

/// A deletion that waited for an offline worker in 0.2.0 moves into the
/// core's outbox, where the plugin takes it when the worker connects, and
/// the old table stays empty.
#[tokio::test]
async fn the_deletions_that_waited_for_a_worker_move_into_the_cores_outbox() {
    let fixture = Fixture::new();
    let older = fixture.open_before_m5(22);
    older
        .module(&WORKERS_MODULE)
        .unwrap()
        .write(Durability::Full, |transaction| {
            transaction.execute_batch(
                "INSERT INTO workers_paired (id, name, address, key_pin, version, details, \
                 paired_at, connected_at, updated_at, chosen_name) \
                 VALUES ('0199e0f2-0000-7000-b000-000000000001', 'build-01', 'build-01.lan:7445', \
                 'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee', 1, '{}', 1, NULL, 1, NULL); \
                 INSERT INTO workers_deletions (worker_id, conversation_id, created_at) \
                 VALUES ('0199e0f2-0000-7000-b000-000000000001', '0199e0f2-7a1b-7c3d-8e4f-000000000011', 3), \
                 ('0199e0f2-0000-7000-b000-000000000001', '0199e0f2-7a1b-7c3d-8e4f-000000000012', 4);",
            )
        })
        .await
        .unwrap();
    drop(older);

    let (store, _workers) = fixture.open();
    let outbox = store.executor_outbox("workers").await.unwrap();
    let left: i64 = store
        .module(&WORKERS_MODULE)
        .unwrap()
        .read(|connection| {
            connection.query_row("SELECT count(*) FROM workers_deletions", [], |row| {
                row.get(0)
            })
        })
        .await
        .unwrap();

    let conversation =
        |last: u32| ConversationId::parse(&format!("0199e0f2-7a1b-7c3d-8e4f-{last:012}")).unwrap();
    assert_eq!(
        outbox,
        [
            OutboxEntry {
                target: "0199e0f2-0000-7000-b000-000000000001".to_owned(),
                conversation_id: conversation(11),
            },
            OutboxEntry {
                target: "0199e0f2-0000-7000-b000-000000000001".to_owned(),
                conversation_id: conversation(12),
            },
        ]
    );
    assert_eq!(left, 0);
}
