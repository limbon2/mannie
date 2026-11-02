//! The databases that the owner registered on this host for the master's
//! requests (ADR-0057): the worker's side of the link's package
//! `databases`, on the engine, the registry and the journal of the service
//! `sqlite-files` (decision 1).
//!
//! The host's ceiling decides every request first (decision 5): a file
//! inside the worker's own folders is refused in every mode, one outside the
//! allowed folders unless the host is in full access, and a write runs only
//! under an authority that the ceiling accepts. The host classifies a
//! write's statements against its own file and computes the write's digest
//! again from its own registry, so an Allow once of anything else goes back
//! to the owner (decision 7). Each write is journaled under its operation ID
//! before it runs and runs to its end, so a request that arrives twice runs
//! once, and a record that a crash cut off becomes uncertain (decision 8).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mannie_plugin_sdk::services::sqlite_files::{
    Change, ChangeOutcome, DatabaseFile, DatabaseStructure, EvidencedWrite, FileRefusal, FileState,
    MAX_STATEMENTS, Page, PageRequest, ReadLimits, SqliteFilesApi as _, SqliteFilesError,
    Statement, StatementOutcome, StatementRows, WriteEvidence,
};
use mannie_store::dbfiles::{self, NewOperation, Registration};
use mannie_tools::{
    Ceiling, DatabaseRequest, HostDecision, HostWrite, WrittenStatement, decide_database_request,
    write_digest,
};
use mannie_types::ids::DatabaseKey;
use mannie_types::tools::{HostDatabaseRefusal, StatementClass, WriteOrigin};

use super::{
    BoundFiles, Planned, WRITE_LIMITS, begin_operation, change_parameters, checked_name,
    database_file, engine_error, engine_sql, file_state, parameters_digest, record_outcome,
    registry_error, run_planned, statement_rows,
};
use crate::Runtime;

/// How the host takes the master's requests on its databases.
#[derive(Debug, Clone)]
pub struct HostDatabaseSettings {
    /// Whether the host's configuration allows databases (ADR-0057,
    /// decision 2). Without it, every request is refused with
    /// `no_databases_on_host`.
    pub enabled: bool,
    /// The host's ceiling.
    pub ceiling: Ceiling,
    /// Whether the host accepts the master's full access (ADR-0029,
    /// decision 5).
    pub accepts_master_full_access: bool,
    /// The plugin whose master's requests these are, which keys the
    /// registrations and the journal's records.
    pub plugin: String,
}

/// Why the host did not do what the master asked of one of its databases.
#[derive(Debug, Clone, PartialEq)]
pub enum HostDatabaseError {
    /// The host's ceiling refuses it, whatever the master sends.
    Refused(HostDatabaseRefusal),
    /// The owner decides first, through the master: the write runs with the
    /// owner's Allow once of this digest, which the host computed.
    NeedsOwner {
        /// The digest of the write.
        digest: String,
    },
    /// The engine, the registry or the journal refused or failed.
    Files(SqliteFilesError),
}

impl From<SqliteFilesError> for HostDatabaseError {
    fn from(error: SqliteFilesError) -> Self {
        Self::Files(error)
    }
}

/// A database on this host as the master lists it: its registration, and
/// the file's identity, which the master's accesses and evidence name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFile {
    /// The registration, with what the host finds at its path.
    pub file: DatabaseFile,
    /// The file's identity.
    pub identity: String,
}

/// What a write did on this host.
#[derive(Debug, Clone, PartialEq)]
pub enum HostWritten {
    /// The statements of a call of the master's tool, each with its outcome.
    Statements(Vec<StatementOutcome>),
    /// The owner's change.
    Change(ChangeOutcome),
}

/// The databases of this host for the master's requests. Cloning is cheap;
/// clones share everything.
#[derive(Debug, Clone)]
pub struct HostDatabases {
    files: BoundFiles,
    settings: Arc<HostDatabaseSettings>,
}

impl Runtime {
    /// The databases that the owner registered on this host for the
    /// master's requests, under these settings (ADR-0057).
    pub fn host_databases(&self, settings: HostDatabaseSettings) -> HostDatabases {
        HostDatabases {
            files: BoundFiles {
                runtime: self.clone(),
                chooser: None,
                plugin: settings.plugin.clone(),
                contributor: settings.plugin.clone(),
            },
            settings: Arc::new(settings),
        }
    }
}

/// What a write runs: the statements of a call of the master's tool, or the
/// owner's change.
enum HostRun {
    Statements {
        statements: Vec<dbfiles::Sql>,
        class: StatementClass,
    },
    Change(Planned),
}

/// A write as the host plans it: what runs, the statements that the
/// write's digest and the ceiling name, each with the digest of its values
/// and its class, and the text, the digest of the values and the class that
/// the journal records.
struct HostPlan {
    run: HostRun,
    statements: Vec<(String, String, StatementClass)>,
    text: String,
    parameters_digest: String,
    class: StatementClass,
}

impl HostDatabases {
    /// The settings that the host takes requests under.
    pub fn settings(&self) -> &HostDatabaseSettings {
        &self.settings
    }

    /// Refuses every request while the host's configuration does not allow
    /// databases.
    fn check_enabled(&self) -> Result<(), HostDatabaseError> {
        if self.settings.enabled {
            Ok(())
        } else {
            Err(HostDatabaseError::Refused(
                HostDatabaseRefusal::NoDatabasesOnHost,
            ))
        }
    }

    /// The ceiling's answer to a request on the file at `path`, or to a
    /// write on it.
    fn decide(&self, path: &Path, write: Option<HostWrite<'_>>) -> HostDecision {
        let protected = self.files.protected();
        decide_database_request(
            &self.settings.ceiling,
            self.settings.accepts_master_full_access,
            &DatabaseRequest {
                path,
                protected: &protected,
                write,
            },
        )
    }

    /// Lets the master reach the file at `path`: inside the allowed folders
    /// unless the host is in full access, and never inside the worker's own
    /// folders.
    fn admit(&self, path: &Path) -> Result<(), HostDatabaseError> {
        match self.decide(path, None) {
            HostDecision::Runs => Ok(()),
            HostDecision::Refused(refusal) => Err(HostDatabaseError::Refused(refusal)),
            // A read never needs the owner; should the ceiling ever say so,
            // the master learns it.
            HostDecision::NeedsOwner => Err(HostDatabaseError::NeedsOwner {
                digest: String::new(),
            }),
        }
    }

    /// The registration of `key`, once the ceiling lets the master reach its
    /// file.
    async fn reachable(&self, key: &DatabaseKey) -> Result<Registration, HostDatabaseError> {
        self.check_enabled()?;
        let registration = self.files.registration(key).await?;
        self.admit(&registration.path)?;
        Ok(registration)
    }

    /// The registration of `key` as the master lists it.
    async fn host_file(&self, key: &DatabaseKey) -> Result<HostFile, HostDatabaseError> {
        let registration = self.files.registration(key).await?;
        let listed = tokio::task::spawn_blocking(move || HostFile {
            file: database_file(
                &registration,
                file_state(&registration.path, &registration.identity),
            ),
            identity: registration.identity,
        })
        .await
        .map_err(|_join_error| SqliteFilesError::Failed)?;
        Ok(listed)
    }

    /// Registers the file at an absolute path that the owner typed on the
    /// master (ADR-0057, decision 3), once it passes the checks of the
    /// service, which refuse links, network paths, the worker's own folders
    /// and files without SQLite's header, and the host's ceiling, which
    /// refuses a file outside the allowed folders unless the host is in full
    /// access.
    pub async fn register(
        &self,
        path: PathBuf,
        name: Option<String>,
        read_only: bool,
    ) -> Result<HostFile, HostDatabaseError> {
        self.check_enabled()?;
        if !path.is_absolute() {
            return Err(SqliteFilesError::FileRefused {
                reason: FileRefusal::NotAFile,
            }
            .into());
        }
        let name = name.as_deref().map(checked_name).transpose()?;
        let checked = self.files.check_path(path).await?;
        self.admit(&checked.path)?;
        let registration = self.files.add_checked(checked, name, read_only).await?;
        Ok(HostFile {
            file: database_file(&registration, FileState::Available),
            identity: registration.identity,
        })
    }

    /// The databases registered on this host, by name, each with what the
    /// host finds at its path.
    pub async fn files(&self) -> Result<Vec<HostFile>, HostDatabaseError> {
        self.check_enabled()?;
        let registrations = self
            .files
            .runtime
            .inner
            .store
            .db_files(&self.settings.plugin)
            .await
            .map_err(registry_error)?;
        let listed = tokio::task::spawn_blocking(move || {
            registrations
                .into_iter()
                .map(|registration| HostFile {
                    file: database_file(
                        &registration,
                        file_state(&registration.path, &registration.identity),
                    ),
                    identity: registration.identity,
                })
                .collect()
        })
        .await
        .map_err(|_join_error| SqliteFilesError::Failed)?;
        Ok(listed)
    }

    /// Changes the owner's name for a database.
    pub async fn rename(
        &self,
        key: DatabaseKey,
        name: String,
    ) -> Result<HostFile, HostDatabaseError> {
        self.check_enabled()?;
        self.files.rename(key.clone(), name).await?;
        self.host_file(&key).await
    }

    /// Marks a database read-only, or lifts the mark.
    pub async fn mark(
        &self,
        key: DatabaseKey,
        read_only: bool,
    ) -> Result<HostFile, HostDatabaseError> {
        self.check_enabled()?;
        self.files.mark(key.clone(), read_only).await?;
        self.host_file(&key).await
    }

    /// Accepts the file that is at a registration's path now, once it passes
    /// the checks and the ceiling.
    pub async fn accept(&self, key: DatabaseKey) -> Result<HostFile, HostDatabaseError> {
        self.reachable(&key).await?;
        let file = self.files.accept(key.clone()).await?;
        let registration = self.files.registration(&key).await?;
        Ok(HostFile {
            file,
            identity: registration.identity,
        })
    }

    /// Forgets a database. The file stays as it is.
    pub async fn forget(&self, key: DatabaseKey) -> Result<(), HostDatabaseError> {
        self.check_enabled()?;
        self.files.forget(key).await?;
        Ok(())
    }

    /// Forgets every database of this host, as unpairing does (ADR-0057,
    /// decision 3). The files and the journal stay, and a host whose
    /// configuration no longer allows databases forgets them too.
    pub async fn forget_all(&self) -> Result<(), HostDatabaseError> {
        let store = &self.files.runtime.inner.store;
        let registrations = store
            .db_files(&self.settings.plugin)
            .await
            .map_err(registry_error)?;
        for registration in registrations {
            store
                .forget_db_file(&self.settings.plugin, &registration.key)
                .await
                .map_err(registry_error)?;
        }
        Ok(())
    }

    /// The structure of a database.
    pub async fn structure(
        &self,
        key: DatabaseKey,
    ) -> Result<DatabaseStructure, HostDatabaseError> {
        self.reachable(&key).await?;
        Ok(self.files.structure(key).await?)
    }

    /// Counts the rows of a table or a view, within the service's time
    /// limit.
    pub async fn count(&self, key: DatabaseKey, table: String) -> Result<u64, HostDatabaseError> {
        self.reachable(&key).await?;
        Ok(self.files.count(key, table).await?)
    }

    /// One page of a table.
    pub async fn page(
        &self,
        key: DatabaseKey,
        request: PageRequest,
    ) -> Result<Page, HostDatabaseError> {
        self.reachable(&key).await?;
        Ok(self.files.page(key, request).await?)
    }

    /// Runs one statement that changes nothing, within the limits. Dropping
    /// the future, as when the master stops the read, interrupts it.
    pub async fn read(
        &self,
        key: DatabaseKey,
        statement: Statement,
        limits: ReadLimits,
    ) -> Result<StatementRows, HostDatabaseError> {
        self.reachable(&key).await?;
        Ok(self.files.read(key, statement, limits).await?)
    }

    /// Runs a write with its evidence (ADR-0057, decisions 5, 7 and 8): the
    /// database is the one that the master saw and the owner did not mark
    /// read-only, the ceiling accepts the evidence's authority for the write
    /// as this host classified it and computed its digest, and the journal
    /// has no record of its operation. The write then runs to its end, even
    /// when the master stops waiting.
    pub async fn write(&self, evidence: WriteEvidence) -> Result<HostWritten, HostDatabaseError> {
        let registration = self.reachable(&evidence.key).await?;
        if registration.read_only {
            return Err(SqliteFilesError::MarkedReadOnly.into());
        }
        if evidence.identity != registration.identity {
            return Err(SqliteFilesError::FileChanged.into());
        }
        let plan = self.plan(&registration, &evidence.write).await?;
        let written: Vec<WrittenStatement<'_>> = plan
            .statements
            .iter()
            .map(|(text, parameters_digest, class)| WrittenStatement {
                text,
                parameters_digest,
                class: *class,
            })
            .collect();
        let digest = write_digest(&evidence.key, &registration.identity, &written);
        let decision = self.decide(
            &registration.path,
            Some(HostWrite {
                key: &evidence.key,
                statements: &written,
                digest: &digest,
                authority: &evidence.authority,
            }),
        );
        match decision {
            HostDecision::Runs => {}
            HostDecision::NeedsOwner => return Err(HostDatabaseError::NeedsOwner { digest }),
            HostDecision::Refused(refusal) => return Err(HostDatabaseError::Refused(refusal)),
        }
        match &evidence.origin {
            WriteOrigin::Call { conversation, part } => tracing::info!(
                operation = %evidence.operation,
                conversation = %conversation,
                part = %part,
                "the master writes to a database of this host for a call of its tool"
            ),
            WriteOrigin::OwnerOperation => tracing::info!(
                operation = %evidence.operation,
                "the master writes the owner's change to a database of this host"
            ),
        }
        let operation = NewOperation {
            id: evidence.operation.clone(),
            plugin: self.settings.plugin.clone(),
            key: evidence.key.clone(),
            statement: plan.text,
            parameters_digest: plan.parameters_digest,
            class: plan.class,
        };
        // The write begins its record, runs and records how it ended in a
        // task of its own, so a link that drops while it runs neither stops
        // it nor leaves its record executing.
        let finished = tokio::spawn(run_journaled(
            self.files.runtime.clone(),
            operation,
            registration,
            plan.run,
        ));
        let written = finished.await.unwrap_or_else(|_join_error| {
            tracing::error!("the task of a write on a database of this host failed to finish");
            Err(SqliteFilesError::Failed)
        })?;
        Ok(written)
    }

    /// Plans a write on its registered file: a call's statements, at most
    /// [`MAX_STATEMENTS`], each classified against the file and none a read,
    /// or the owner's change as the service plans it.
    async fn plan(
        &self,
        registration: &Registration,
        write: &EvidencedWrite,
    ) -> Result<HostPlan, SqliteFilesError> {
        match write {
            EvidencedWrite::Statements { statements } => {
                if statements.is_empty() || statements.len() > MAX_STATEMENTS {
                    return Err(SqliteFilesError::TooManyStatements);
                }
                let sqls = statements
                    .iter()
                    .map(engine_sql)
                    .collect::<Result<Vec<_>, _>>()?;
                let texts: Vec<String> = sqls.iter().map(|sql| sql.text.clone()).collect();
                let (path, identity) = (registration.path.clone(), registration.identity.clone());
                let classes = dbfiles::run(move |_cancel| {
                    texts
                        .iter()
                        .map(|text| dbfiles::classify(&path, &identity, text))
                        .collect::<Result<Vec<_>, _>>()
                })
                .await
                .map_err(engine_error)?;
                if let Some(read) = classes.iter().find(|class| **class == StatementClass::Read) {
                    return Err(SqliteFilesError::NotARead { class: *read });
                }
                let class = classes
                    .iter()
                    .copied()
                    .max()
                    .unwrap_or(StatementClass::Write);
                let named = statements
                    .iter()
                    .zip(&classes)
                    .map(|(statement, class)| {
                        (
                            statement.text.clone(),
                            parameters_digest(&statement.parameters),
                            *class,
                        )
                    })
                    .collect();
                let all: Vec<_> = statements
                    .iter()
                    .flat_map(|statement| statement.parameters.iter().cloned())
                    .collect();
                Ok(HostPlan {
                    text: joined(statements),
                    parameters_digest: parameters_digest(&all),
                    statements: named,
                    run: HostRun::Statements {
                        statements: sqls,
                        class,
                    },
                    class,
                })
            }
            EvidencedWrite::Change { change } => {
                let planned = self.files.plan(registration, change).await?;
                let (text, class) = match &planned {
                    Planned::Statements { class, .. } => (change_text(change), *class),
                    Planned::Edit { table, edit } => (
                        dbfiles::edit_text(table, edit).map_err(engine_error)?,
                        StatementClass::Write,
                    ),
                };
                let values_digest = parameters_digest(&change_parameters(change));
                let named = match change {
                    Change::Statements { statements, .. } => statements
                        .iter()
                        .map(|statement| {
                            (
                                statement.text.clone(),
                                parameters_digest(&statement.parameters),
                                class,
                            )
                        })
                        .collect(),
                    Change::UpdateRow { .. }
                    | Change::InsertRow { .. }
                    | Change::DeleteRows { .. } => {
                        vec![(text.clone(), values_digest.clone(), class)]
                    }
                };
                Ok(HostPlan {
                    run: HostRun::Change(planned),
                    statements: named,
                    text,
                    parameters_digest: values_digest,
                    class,
                })
            }
        }
    }
}

/// Journals a write under its operation ID, runs it on its file and
/// records how it ended. An ID that the journal has refuses the write.
async fn run_journaled(
    runtime: Runtime,
    operation: NewOperation,
    registration: Registration,
    run: HostRun,
) -> Result<HostWritten, SqliteFilesError> {
    begin_operation(&runtime, &operation).await?;
    let (path, identity) = (registration.path, registration.identity);
    match run {
        HostRun::Statements { statements, class } => {
            let ran = dbfiles::run(move |cancel| {
                dbfiles::write(&path, &identity, &statements, class, WRITE_LIMITS, cancel)
            })
            .await;
            let written = record_outcome(&runtime, &operation.id, ran, |written| {
                written.iter().map(|done| done.changes).sum()
            })
            .await?;
            Ok(HostWritten::Statements(
                written
                    .iter()
                    .map(|done| StatementOutcome {
                        changes: done.changes,
                        rows: done.rows.as_ref().map(statement_rows),
                    })
                    .collect(),
            ))
        }
        HostRun::Change(planned) => {
            let ran =
                dbfiles::run(move |cancel| run_planned(&path, &identity, planned, cancel)).await;
            record_outcome(&runtime, &operation.id, ran, |outcome| outcome.changes)
                .await
                .map(HostWritten::Change)
        }
    }
}

/// The texts of statements, one after another.
fn joined(statements: &[Statement]) -> String {
    statements
        .iter()
        .map(|statement| statement.text.as_str())
        .collect::<Vec<_>>()
        .join(";\n")
}

/// The text of an owner's change of statements, which the journal records.
fn change_text(change: &Change) -> String {
    match change {
        Change::Statements { statements, .. } => joined(statements),
        Change::UpdateRow { .. } | Change::InsertRow { .. } | Change::DeleteRows { .. } => {
            String::new()
        }
    }
}
