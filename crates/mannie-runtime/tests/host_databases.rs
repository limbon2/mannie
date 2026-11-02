//! The databases that the owner registered on a worker's host, as the
//! worker takes the master's requests on them (ADR-0057, decisions 2, 3, 5,
//! 7 and 8): nothing runs while the host's configuration does not allow
//! databases; a typed path registers only inside the allowed folders unless
//! the host is in full access, and only a file that passes the service's
//! checks; a file whose folder the ceiling no longer allows is refused; a
//! write runs under an authority that the ceiling accepts, with an Allow
//! once of exactly the write that the host classified and computed, and
//! goes back to the owner otherwise; a database that the master did not see
//! or that the owner marked read-only refuses it; and an operation runs
//! once. Unpairing forgets the registrations and keeps the files.

#![cfg(test)]

use std::path::{Path, PathBuf};

use mannie_plugin_sdk::contracts::{
    ConversationId, DatabaseKey, DatabaseOperationId, HostDatabaseRefusal, WriteAuthority,
    WriteOrigin,
};
use mannie_plugin_sdk::services::sqlite_files::{
    Change, ColumnValue, EvidencedWrite, FileRefusal, FileState, ReadLimits, RowMatch, SqlValue,
    SqliteFilesError, Statement, WriteEvidence,
};
use mannie_runtime::testing::{Harness, HarnessOptions};
use mannie_runtime::{
    HostDatabaseError, HostDatabaseSettings, HostDatabases, HostFile, HostWritten,
};
use mannie_store::dbfiles;
use mannie_tools::{Ceiling, canonical_folder};
use mannie_types::ceiling::CeilingMode;
use mannie_types::ids::PartId;
use mannie_types::tools::GrantScope;
use tempfile::TempDir;

/// The database's tables and rows when a test starts.
const SCHEMA: &str = "CREATE TABLE parts (name TEXT PRIMARY KEY, stock INTEGER NOT NULL);
                      INSERT INTO parts (name, stock) VALUES ('bolt', 12), ('nut', 40);";

const UPDATE: &str = "UPDATE parts SET stock = ?1 WHERE name = ?2";
const CREATE_INDEX: &str = "CREATE INDEX parts_stock ON parts (stock)";

/// A worker's runtime with a folder that its ceiling allows, a folder that
/// it does not, and a database in each.
struct Setup {
    harness: Harness,
    _folder: TempDir,
    allowed: PathBuf,
    inside: PathBuf,
    outside: PathBuf,
}

impl Setup {
    async fn new() -> Self {
        let harness = Harness::with_options(HarnessOptions::default()).await;
        let folder = tempfile::tempdir().unwrap();
        let allowed = folder.path().join("allowed");
        let elsewhere = folder.path().join("elsewhere");
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let inside = allowed.join("parts.db");
        let outside = elsewhere.join("parts.db");
        dbfiles::create_for_tests(&inside, SCHEMA).unwrap();
        dbfiles::create_for_tests(&outside, SCHEMA).unwrap();
        Self {
            harness,
            _folder: folder,
            allowed,
            inside,
            outside,
        }
    }

    /// The host's databases under a ceiling in `mode` that allows the
    /// folder `allowed`.
    fn host(&self, mode: CeilingMode, accepts_master_full_access: bool) -> HostDatabases {
        self.host_with(true, mode, accepts_master_full_access, &self.allowed)
    }

    fn host_with(
        &self,
        enabled: bool,
        mode: CeilingMode,
        accepts_master_full_access: bool,
        allowed: &Path,
    ) -> HostDatabases {
        self.harness.runtime.host_databases(HostDatabaseSettings {
            enabled,
            ceiling: Ceiling {
                mode,
                allowed_folders: vec![PathBuf::from(canonical_folder(allowed).unwrap())],
                rules: Vec::new(),
            },
            accepts_master_full_access,
            plugin: "workers".to_owned(),
        })
    }

    /// Registers the database in the allowed folder.
    async fn register(&self, host: &HostDatabases) -> HostFile {
        host.register(self.inside.clone(), Some("Parts".to_owned()), false)
            .await
            .unwrap()
    }
}

const OPERATION: &str = "0199e0f2-0000-7000-8000-000000000201";

fn operation(last: u8) -> DatabaseOperationId {
    DatabaseOperationId::parse(&format!("0199e0f2-0000-7000-8000-0000000002{last:02x}")).unwrap()
}

fn call_origin() -> WriteOrigin {
    WriteOrigin::Call {
        conversation: ConversationId::parse("0199e0f2-0000-7000-8000-000000000301").unwrap(),
        part: PartId::parse("0199e0f2-0000-7000-8000-000000000302").unwrap(),
    }
}

/// The evidence of a call's write of the bolts' stock.
fn statements(
    file: &HostFile,
    id: DatabaseOperationId,
    authority: WriteAuthority,
) -> WriteEvidence {
    WriteEvidence::new(
        id,
        file.file.key.clone(),
        file.identity.clone(),
        EvidencedWrite::Statements {
            statements: vec![Statement::new(
                UPDATE,
                vec![SqlValue::integer(11), SqlValue::text("bolt")],
            )],
        },
        call_origin(),
        authority,
    )
}

/// The evidence of the owner's edit of the bolts' stock.
fn edit(file: &HostFile, id: DatabaseOperationId) -> WriteEvidence {
    WriteEvidence::new(
        id,
        file.file.key.clone(),
        file.identity.clone(),
        EvidencedWrite::Change {
            change: Change::UpdateRow {
                table: "parts".to_owned(),
                row: RowMatch {
                    key: vec![SqlValue::integer(1)],
                    expected: vec![ColumnValue {
                        column: "stock".to_owned(),
                        value: SqlValue::integer(12),
                    }],
                },
                set: vec![ColumnValue {
                    column: "stock".to_owned(),
                    value: SqlValue::integer(11),
                }],
            },
        },
        WriteOrigin::OwnerOperation,
        WriteAuthority::OwnerAction,
    )
}

fn grant(key: &DatabaseKey, text: &str) -> GrantScope {
    GrantScope::Database {
        plugin: "workers".to_owned(),
        key: key.clone(),
        name: "Parts".to_owned(),
        statement: text.to_owned(),
    }
}

/// The bolts' stock, as the host reads it.
async fn bolts(host: &HostDatabases, key: &DatabaseKey) -> String {
    let rows = host
        .read(
            key.clone(),
            Statement::new(
                "SELECT stock FROM parts WHERE name = ?1",
                vec![SqlValue::text("bolt")],
            ),
            ReadLimits::VIEW,
        )
        .await
        .unwrap();
    rows.rows[0][0].display(20)
}

#[tokio::test]
async fn a_host_without_databases_in_its_configuration_refuses_every_request() {
    let setup = Setup::new().await;
    let host = setup.host_with(false, CeilingMode::Autonomous, true, &setup.allowed);
    let refused = Err(HostDatabaseError::Refused(
        HostDatabaseRefusal::NoDatabasesOnHost,
    ));

    assert_eq!(
        host.register(setup.inside.clone(), None, false)
            .await
            .map(|_file| ()),
        refused
    );
    assert_eq!(host.files().await.map(|_files| ()), refused);
    let registered = setup
        .register(&setup.host(CeilingMode::Autonomous, true))
        .await;
    assert_eq!(
        host.structure(registered.file.key.clone())
            .await
            .map(|_structure| ()),
        refused
    );
    assert_eq!(
        host.write(edit(&registered, operation(1)))
            .await
            .map(|_written| ()),
        refused
    );
}

#[tokio::test]
async fn a_typed_path_registers_only_inside_the_allowed_folders_unless_in_full_access() {
    let setup = Setup::new().await;
    let host = setup.host(CeilingMode::Autonomous, true);

    assert_eq!(
        host.register(setup.outside.clone(), None, false)
            .await
            .map(|_file| ()),
        Err(HostDatabaseError::Refused(
            HostDatabaseRefusal::OutsideAllowedFolders
        ))
    );
    assert_eq!(
        host.register(PathBuf::from("parts.db"), None, false)
            .await
            .map(|_file| ()),
        Err(HostDatabaseError::Files(SqliteFilesError::FileRefused {
            reason: FileRefusal::NotAFile
        }))
    );
    let text = setup.allowed.join("notes.txt");
    std::fs::write(&text, "not a database").unwrap();
    assert_eq!(
        host.register(text, None, false).await.map(|_file| ()),
        Err(HostDatabaseError::Files(SqliteFilesError::FileRefused {
            reason: FileRefusal::NotADatabase
        }))
    );

    let file = setup.register(&host).await;
    assert_eq!(file.file.name, "Parts");
    assert_eq!(file.file.state, FileState::Available);
    assert_eq!(
        file.identity,
        dbfiles::check_new(&setup.inside, &[]).unwrap().identity
    );
    assert_eq!(host.files().await.unwrap(), vec![file]);

    let full_access = setup.host(CeilingMode::FullAccess, false);
    let anywhere = full_access
        .register(setup.outside.clone(), None, true)
        .await
        .unwrap();
    assert_eq!(anywhere.file.name, "parts.db");
    assert!(anywhere.file.read_only);
}

#[tokio::test]
async fn a_file_whose_folder_the_ceiling_no_longer_allows_is_refused_until_registered_again() {
    let setup = Setup::new().await;
    let file = setup
        .register(&setup.host(CeilingMode::Autonomous, true))
        .await;
    let key = file.file.key.clone();
    let narrowed = setup.host_with(
        true,
        CeilingMode::Autonomous,
        true,
        setup.outside.parent().unwrap(),
    );
    let refused = Err(HostDatabaseError::Refused(
        HostDatabaseRefusal::OutsideAllowedFolders,
    ));

    assert_eq!(narrowed.structure(key.clone()).await.map(|_s| ()), refused);
    assert_eq!(
        narrowed
            .read(
                key.clone(),
                Statement::new("SELECT 1", Vec::new()),
                ReadLimits::VIEW
            )
            .await
            .map(|_rows| ()),
        refused
    );
    assert_eq!(
        narrowed.write(edit(&file, operation(2))).await.map(|_w| ()),
        refused
    );
    assert_eq!(
        bolts(&setup.host(CeilingMode::Autonomous, true), &key).await,
        "12"
    );
}

#[tokio::test]
async fn the_owners_action_and_allow_once_run_on_a_host_that_asks() {
    let setup = Setup::new().await;
    let host = setup.host(CeilingMode::Ask, false);
    let file = setup.register(&host).await;
    let key = file.file.key.clone();

    let written = host.write(edit(&file, operation(3))).await.unwrap();
    assert!(matches!(written, HostWritten::Change(ref outcome) if outcome.changes == 1));
    assert_eq!(bolts(&host, &key).await, "11");

    // An Allow once of another write goes back to the owner with the digest
    // that the host computed, and the owner's Allow once of it runs.
    let other = WriteAuthority::AllowedOnce {
        digest: "the digest of another write".to_owned(),
    };
    let Err(HostDatabaseError::NeedsOwner { digest }) =
        host.write(statements(&file, operation(4), other)).await
    else {
        panic!("an Allow once of another write needs the owner");
    };
    let written = host
        .write(statements(
            &file,
            operation(5),
            WriteAuthority::AllowedOnce { digest },
        ))
        .await
        .unwrap();
    let HostWritten::Statements(outcomes) = written else {
        panic!("a call's write has an outcome for each statement");
    };
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].changes, 1);
}

#[tokio::test]
async fn a_grant_needs_the_owner_on_an_ask_host_and_runs_on_an_autonomous_one() {
    let setup = Setup::new().await;
    let file = setup.register(&setup.host(CeilingMode::Ask, false)).await;
    let key = file.file.key.clone();
    let granted = || WriteAuthority::Grants {
        scopes: vec![grant(&key, UPDATE)],
    };

    assert!(matches!(
        setup
            .host(CeilingMode::Ask, true)
            .write(statements(&file, operation(6), granted()))
            .await,
        Err(HostDatabaseError::NeedsOwner { .. })
    ));
    assert!(matches!(
        setup
            .host(CeilingMode::Autonomous, true)
            .write(statements(
                &file,
                operation(7),
                WriteAuthority::Grants {
                    scopes: vec![grant(&key, "UPDATE parts SET stock = 0")]
                }
            ))
            .await,
        Err(HostDatabaseError::NeedsOwner { .. })
    ));
    setup
        .host(CeilingMode::Autonomous, false)
        .write(statements(&file, operation(8), granted()))
        .await
        .unwrap();
    assert_eq!(
        bolts(&setup.host(CeilingMode::Ask, false), &key).await,
        "11"
    );
}

#[tokio::test]
async fn the_masters_full_access_runs_only_where_the_host_accepts_it() {
    let setup = Setup::new().await;
    let file = setup.register(&setup.host(CeilingMode::Ask, false)).await;

    assert!(matches!(
        setup
            .host(CeilingMode::Autonomous, false)
            .write(statements(&file, operation(9), WriteAuthority::FullAccess))
            .await,
        Err(HostDatabaseError::NeedsOwner { .. })
    ));
    setup
        .host(CeilingMode::Ask, true)
        .write(statements(&file, operation(10), WriteAuthority::FullAccess))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_read_only_host_refuses_every_write_and_reads() {
    let setup = Setup::new().await;
    let file = setup.register(&setup.host(CeilingMode::Ask, false)).await;
    let read_only = setup.host(CeilingMode::ReadOnly, true);

    for authority in [WriteAuthority::OwnerAction, WriteAuthority::FullAccess] {
        assert_eq!(
            read_only
                .write(statements(&file, operation(11), authority))
                .await
                .map(|_written| ()),
            Err(HostDatabaseError::Refused(
                HostDatabaseRefusal::ReadOnlyHost
            ))
        );
    }
    assert_eq!(bolts(&read_only, &file.file.key).await, "12");
    read_only.structure(file.file.key.clone()).await.unwrap();
}

#[tokio::test]
async fn an_operation_runs_once_and_a_database_that_the_master_did_not_see_refuses_it() {
    let setup = Setup::new().await;
    let host = setup.host(CeilingMode::Autonomous, true);
    let file = setup.register(&host).await;
    let key = file.file.key.clone();

    host.write(edit(&file, operation(12))).await.unwrap();
    let again = host.write(edit(&file, operation(12))).await;
    assert_eq!(
        again.map(|_written| ()),
        Err(HostDatabaseError::Files(SqliteFilesError::AlreadyRun))
    );

    let mut stale = edit(&file, operation(13));
    stale.identity = "7:42".to_owned();
    assert_eq!(
        host.write(stale).await.map(|_written| ()),
        Err(HostDatabaseError::Files(SqliteFilesError::FileChanged))
    );

    host.mark(key.clone(), true).await.unwrap();
    assert_eq!(
        host.write(edit(&file, operation(14)))
            .await
            .map(|_written| ()),
        Err(HostDatabaseError::Files(SqliteFilesError::MarkedReadOnly))
    );
    host.mark(key.clone(), false).await.unwrap();

    let unconfirmed = WriteEvidence::new(
        DatabaseOperationId::parse(OPERATION).unwrap(),
        key.clone(),
        file.identity.clone(),
        EvidencedWrite::Change {
            change: Change::Statements {
                statements: vec![Statement::new(CREATE_INDEX, Vec::new())],
                schema_confirmed: false,
            },
        },
        WriteOrigin::OwnerOperation,
        WriteAuthority::OwnerAction,
    );
    assert_eq!(
        host.write(unconfirmed).await.map(|_written| ()),
        Err(HostDatabaseError::Files(
            SqliteFilesError::SchemaUnconfirmed
        ))
    );
    // The edit ran once, and the refused writes changed nothing.
    assert_eq!(bolts(&host, &key).await, "11");
}

#[tokio::test]
async fn a_call_may_not_read_through_a_write_and_unpairing_forgets_every_database() {
    let setup = Setup::new().await;
    let host = setup.host(CeilingMode::FullAccess, false);
    let file = setup.register(&host).await;
    let read = WriteEvidence::new(
        operation(15),
        file.file.key.clone(),
        file.identity.clone(),
        EvidencedWrite::Statements {
            statements: vec![Statement::new("SELECT stock FROM parts", Vec::new())],
        },
        call_origin(),
        WriteAuthority::FullAccess,
    );

    assert_eq!(
        host.write(read).await.map(|_written| ()),
        Err(HostDatabaseError::Files(SqliteFilesError::NotARead {
            class: mannie_types::tools::StatementClass::Read
        }))
    );

    host.forget_all().await.unwrap();
    assert_eq!(host.files().await.unwrap(), Vec::new());
    assert!(setup.inside.exists(), "the file stays");
}
