//! The journal of installs of updates (ADR-0063, decision 9). The desktop app
//! commits a record before it quits to install a version, and the next start
//! settles the record from the version that runs then.
//!
//! The installer runs after the core shut down, and on Windows it ends
//! Mannie's process, so no code of the old version can record how the install
//! ended. The version that starts afterwards is the evidence.

use mannie_types::ids::UpdateId;
use mannie_types::updates::{UpdateOutcome, UpdateResult};
use rusqlite::{OptionalExtension, params};

use crate::{Durability, Store, StoreError};

const INSERT: &str = "INSERT INTO updates_operations (id, from_version, to_version, state, created_at, updated_at) VALUES (?1, ?2, ?3, 'executing', ?4, ?4)";
const NOTE_FAILURE: &str = "UPDATE updates_operations SET failure = ?2, updated_at = ?3 WHERE id = ?1 AND state = 'executing'";
const NEWEST_EXECUTING: &str = "SELECT from_version, to_version, failure FROM updates_operations WHERE state = 'executing' ORDER BY created_at DESC, id DESC LIMIT 1";
const SETTLE: &str = "UPDATE updates_operations SET state = CASE WHEN to_version = ?1 THEN 'completed' ELSE 'failed' END, updated_at = ?2 WHERE state = 'executing'";

#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[INSERT, NOTE_FAILURE, NEWEST_EXECUTING, SETTLE];

/// An install that an earlier run of Mannie began, as the start after it
/// settled it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettledUpdate {
    /// The versions of the install and how it ended.
    pub outcome: UpdateOutcome,
    /// What the old version noted when the install failed and it started
    /// again. The text is an error of the installer for the log, never for
    /// the owner.
    pub failure: Option<String>,
}

impl Store {
    /// Commits the record of an install before Mannie quits to run it.
    pub async fn begin_update(
        &self,
        id: &UpdateId,
        from_version: &str,
        to_version: &str,
    ) -> Result<(), StoreError> {
        let (id, from_version, to_version) = (
            id.to_string(),
            from_version.to_owned(),
            to_version.to_owned(),
        );
        let now = self.clock().now().as_millis();
        self.write(Durability::Full, move |transaction| {
            transaction.execute(INSERT, params![id, from_version, to_version, now])?;
            Ok(())
        })
        .await
    }

    /// Notes why an install failed, while its record still waits for the next
    /// start. The record's state does not change: the next start settles it.
    pub async fn note_update_failure(
        &self,
        id: &UpdateId,
        failure: &str,
    ) -> Result<(), StoreError> {
        let (id, failure) = (id.to_string(), failure.to_owned());
        let now = self.clock().now().as_millis();
        self.write(Durability::Full, move |transaction| {
            transaction.execute(NOTE_FAILURE, params![id, failure, now])?;
            Ok(())
        })
        .await
    }

    /// Settles every install that still waits: completed when it was to
    /// install the version that runs now, failed otherwise. Returns the
    /// newest of them, or nothing when none waited, so a second call returns
    /// nothing.
    pub async fn resolve_updates(
        &self,
        current_version: &str,
    ) -> Result<Option<SettledUpdate>, StoreError> {
        let current_version = current_version.to_owned();
        let now = self.clock().now().as_millis();
        self.write(Durability::Full, move |transaction| {
            let newest = transaction
                .query_row(NEWEST_EXECUTING, [], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .optional()?;
            let Some((from_version, to_version, failure)) = newest else {
                return Ok(None);
            };
            transaction.execute(SETTLE, params![current_version, now])?;
            let result = if to_version == current_version {
                UpdateResult::Completed
            } else {
                UpdateResult::Failed
            };
            Ok(Some(SettledUpdate {
                outcome: UpdateOutcome {
                    from_version,
                    to_version,
                    result,
                },
                failure,
            }))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use mannie_types::clock::{ManualClock, UtcMillis};
    use mannie_types::ids::UpdateId;
    use mannie_types::updates::{UpdateOutcome, UpdateResult};
    use tempfile::TempDir;

    use super::SettledUpdate;
    use crate::{Store, StoreOptions};

    struct Fixture {
        store: Store,
        clock: Arc<ManualClock>,
        _directory: TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let clock = Arc::new(ManualClock::new(UtcMillis::from_millis(1_794_000_000_000)));
            let options = StoreOptions::new(
                directory.path().join("backups"),
                Arc::<ManualClock>::clone(&clock),
            );
            let store = Store::open(&directory.path().join("test.sqlite"), &options).unwrap();
            Self {
                store,
                clock,
                _directory: directory,
            }
        }

        /// Each record as its ID, its state, its noted failure and its two
        /// times, in the order of the IDs.
        async fn records(&self) -> Vec<(String, String, Option<String>, i64, i64)> {
            self.store
                .read(|connection| {
                    connection
                        .prepare(
                            "SELECT id, state, failure, created_at, updated_at \
                             FROM updates_operations ORDER BY id",
                        )?
                        .query_map([], |row| {
                            Ok((
                                row.get(0)?,
                                row.get(1)?,
                                row.get(2)?,
                                row.get(3)?,
                                row.get(4)?,
                            ))
                        })?
                        .collect()
                })
                .await
                .unwrap()
        }
    }

    fn id(last: u8) -> UpdateId {
        UpdateId::parse(&format!("0199e0f2-0000-7000-8000-0000000000{last:02x}")).unwrap()
    }

    fn outcome(from_version: &str, to_version: &str, result: UpdateResult) -> UpdateOutcome {
        UpdateOutcome {
            from_version: from_version.to_owned(),
            to_version: to_version.to_owned(),
            result,
        }
    }

    #[tokio::test]
    async fn an_install_is_completed_when_its_version_runs_at_the_next_start() {
        let fixture = Fixture::new();
        fixture
            .store
            .begin_update(&id(1), "0.6.1", "0.7.0")
            .await
            .unwrap();
        assert_eq!(
            fixture.records().await,
            [(
                id(1).to_string(),
                "executing".to_owned(),
                None,
                1_794_000_000_000,
                1_794_000_000_000
            )]
        );

        fixture.clock.advance(Duration::from_secs(30));
        let settled = fixture.store.resolve_updates("0.7.0").await.unwrap();

        assert_eq!(
            settled,
            Some(SettledUpdate {
                outcome: outcome("0.6.1", "0.7.0", UpdateResult::Completed),
                failure: None,
            })
        );
        assert_eq!(
            fixture.records().await,
            [(
                id(1).to_string(),
                "completed".to_owned(),
                None,
                1_794_000_000_000,
                1_794_000_030_000
            )]
        );
    }

    #[tokio::test]
    async fn an_install_is_failed_when_another_version_runs_at_the_next_start() {
        let fixture = Fixture::new();
        fixture
            .store
            .begin_update(&id(1), "0.6.1", "0.7.0")
            .await
            .unwrap();

        let settled = fixture.store.resolve_updates("0.6.1").await.unwrap();

        assert_eq!(
            settled,
            Some(SettledUpdate {
                outcome: outcome("0.6.1", "0.7.0", UpdateResult::Failed),
                failure: None,
            })
        );
        assert_eq!(fixture.records().await[0].1, "failed");
    }

    #[tokio::test]
    async fn a_settled_install_is_reported_once() {
        let fixture = Fixture::new();
        assert_eq!(fixture.store.resolve_updates("0.6.1").await.unwrap(), None);
        fixture
            .store
            .begin_update(&id(1), "0.6.1", "0.7.0")
            .await
            .unwrap();
        fixture.store.resolve_updates("0.7.0").await.unwrap();
        let after_first = fixture.records().await;

        fixture.clock.advance(Duration::from_secs(30));
        let second = fixture.store.resolve_updates("0.6.1").await.unwrap();

        assert_eq!(second, None);
        assert_eq!(fixture.records().await, after_first);
    }

    #[tokio::test]
    async fn a_noted_failure_stays_with_the_record_until_it_is_settled() {
        let fixture = Fixture::new();
        fixture
            .store
            .begin_update(&id(1), "0.6.1", "0.7.0")
            .await
            .unwrap();

        fixture.clock.advance(Duration::from_secs(5));
        fixture
            .store
            .note_update_failure(&id(1), "the package manager refused the package")
            .await
            .unwrap();
        assert_eq!(
            fixture.records().await,
            [(
                id(1).to_string(),
                "executing".to_owned(),
                Some("the package manager refused the package".to_owned()),
                1_794_000_000_000,
                1_794_000_005_000
            )]
        );
        let settled = fixture.store.resolve_updates("0.6.1").await.unwrap();

        assert_eq!(
            settled,
            Some(SettledUpdate {
                outcome: outcome("0.6.1", "0.7.0", UpdateResult::Failed),
                failure: Some("the package manager refused the package".to_owned()),
            })
        );

        // A note that arrives for a settled record changes nothing.
        let settled_records = fixture.records().await;
        fixture
            .store
            .note_update_failure(&id(1), "a late note")
            .await
            .unwrap();
        assert_eq!(fixture.records().await, settled_records);
    }

    #[tokio::test]
    async fn every_waiting_install_is_settled_and_the_newest_is_reported() {
        let fixture = Fixture::new();
        fixture
            .store
            .begin_update(&id(1), "0.6.0", "0.6.1")
            .await
            .unwrap();
        fixture.clock.advance(Duration::from_secs(60));
        fixture
            .store
            .begin_update(&id(2), "0.6.1", "0.7.0")
            .await
            .unwrap();

        let settled = fixture.store.resolve_updates("0.7.0").await.unwrap();

        assert_eq!(
            settled.map(|settled| settled.outcome),
            Some(outcome("0.6.1", "0.7.0", UpdateResult::Completed))
        );
        let states: Vec<String> = fixture
            .records()
            .await
            .into_iter()
            .map(|record| record.1)
            .collect();
        assert_eq!(states, ["failed", "completed"]);
    }

    #[tokio::test]
    async fn an_install_cannot_be_recorded_twice_under_one_id() {
        let fixture = Fixture::new();
        fixture
            .store
            .begin_update(&id(1), "0.6.1", "0.7.0")
            .await
            .unwrap();

        let second = fixture.store.begin_update(&id(1), "0.6.1", "0.7.1").await;

        assert!(second.is_err());
        assert_eq!(fixture.records().await.len(), 1);
    }
}
