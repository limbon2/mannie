//! The commands and the channel of the feature Databases (plugins/workers/
//! docs/databases.md, the view; ADR-0055, decision 6): registering and
//! naming databases, reading their structures, pages and statements, the
//! owner's changes, and following the list. Each is a spec of the plugin SDK,
//! so the host checks its input before the handler runs and answers
//! `feature_off` while the feature is off. The service `sqlite-files` checks
//! names, statements and limits itself and answers with codes that the view
//! words (ADR-0056).

use mannie_plugin_sdk::contracts::{DatabaseKey, StatementClass};
use mannie_plugin_sdk::ipc::{
    ChannelSpec, CommandSpec, InputRefusal, Interface, PluginError, Validate,
};
use mannie_plugin_sdk::services::sqlite_files::{
    DatabaseStructure, FileState, MAX_NAME_CHARS, OwnerChange, Page, PageRequest, SqliteFilesError,
    Statement, StatementRows,
};
use serde::{Deserialize, Serialize};
use specta::Type;

/// The plugin's optional feature Databases.
pub const DATABASES: &str = "databases";

/// The most characters of the ID that the view gives a read it may stop.
pub const MAX_RUN_ID_CHARS: usize = 64;

/// Where a registered database lives. Workers' databases come with
/// ADR-0057.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DatabaseMachine {
    /// This computer.
    ThisComputer,
}

/// A registered database, as the view lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseView {
    /// The plugin's key for it.
    pub key: DatabaseKey,
    /// The owner's name for it, unique among the registered databases.
    pub name: String,
    /// The file's name, without its folder.
    pub file_name: String,
    /// Whether the owner marked it read-only.
    pub read_only: bool,
    /// What the core found at its path when the feature last listed it.
    pub state: FileState,
    /// Where it lives.
    pub machine: DatabaseMachine,
}

/// The databases as they are, then after each change, while the feature is
/// on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DatabaseListUpdate {
    /// Every registered database, by name.
    Databases {
        /// The databases.
        databases: Vec<DatabaseView>,
    },
}

/// The argument of `add_database`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddDatabase {
    /// The owner's name for the database, or `None` for the file's name.
    #[serde(default)]
    pub name: Option<String>,
    /// Whether the owner marks it read-only.
    pub read_only: bool,
    /// The dialog's name for SQLite files, from the catalog.
    pub databases_filter: String,
    /// The dialog's name for every file, from the catalog.
    pub all_files_filter: String,
}

impl Validate for AddDatabase {
    /// The service checks the name and the filters' names, and answers
    /// `invalid_name` or `invalid_filter`.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// The argument of the commands that name one database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatabaseRef {
    /// The database.
    pub key: DatabaseKey,
}

impl Validate for DatabaseRef {
    /// A key decodes only in its form; an unknown one is `not_registered`.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// The argument of `rename_database`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenameDatabase {
    /// The database.
    pub key: DatabaseKey,
    /// Its new name.
    pub name: String,
}

impl Validate for RenameDatabase {
    /// The service checks the name, and answers `invalid_name`.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// The argument of `mark_database`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarkDatabase {
    /// The database.
    pub key: DatabaseKey,
    /// Whether it is read-only from now on.
    pub read_only: bool,
}

impl Validate for MarkDatabase {
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// The argument of `count_database_rows`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountRows {
    /// The database.
    pub key: DatabaseKey,
    /// The table or view.
    pub table: String,
}

impl Validate for CountRows {
    /// The engine quotes the table's name and answers `invalid_statement`
    /// for a table that the database does not have.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// The argument of `database_page`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageDatabase {
    /// The database.
    pub key: DatabaseKey,
    /// The page.
    pub request: PageRequest,
}

impl Validate for PageDatabase {
    /// The service checks the page's size, its filters and its cursor.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// The argument of `classify_database_statement`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClassifyStatement {
    /// The database.
    pub key: DatabaseKey,
    /// The statement's text.
    pub text: String,
}

impl Validate for ClassifyStatement {
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// The argument of `read_database`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadDatabase {
    /// The database.
    pub key: DatabaseKey,
    /// The statement, which must change nothing.
    pub statement: Statement,
    /// The view's ID of the read, with which `stop_database_read` stops it.
    #[serde(default)]
    pub run: Option<String>,
}

impl Validate for ReadDatabase {
    /// The read's ID has 1 to 64 characters, none of them a control
    /// character; the service checks the statement.
    fn validate(&self) -> Result<(), InputRefusal> {
        match &self.run {
            Some(run) if !is_run_id(run) => Err(InputRefusal::new("run")),
            _valid => Ok(()),
        }
    }
}

/// The argument of `stop_database_read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StopRead {
    /// The view's ID of the read.
    pub run: String,
}

impl Validate for StopRead {
    /// The read's ID has 1 to 64 characters, none of them a control
    /// character.
    fn validate(&self) -> Result<(), InputRefusal> {
        if is_run_id(&self.run) {
            Ok(())
        } else {
            Err(InputRefusal::new("run"))
        }
    }
}

/// Whether a text is a read's ID that the view may give.
fn is_run_id(run: &str) -> bool {
    let is_sized = (1..=MAX_RUN_ID_CHARS).contains(&run.chars().count());
    is_sized && !run.chars().any(char::is_control)
}

/// The argument of `run_database_change`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeDatabase {
    /// The database.
    pub key: DatabaseKey,
    /// The owner's change, with the ID that the view made for it.
    pub change: OwnerChange,
}

impl Validate for ChangeDatabase {
    /// The service checks the change and refuses an ID that its journal has.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// What an owner's change did. Integers travel as decimal text, since
/// they may be larger than JavaScript's numbers hold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChangeDone {
    /// The rows that it changed.
    pub changes: String,
    /// The rowid of an inserted row, for a table with rowids.
    pub rowid: Option<String>,
    /// The rows that its last statement returned, such as through
    /// `RETURNING`.
    pub rows: Option<StatementRows>,
}

/// Why a command of Databases failed. The host's refusals, such as
/// `feature_off`, come beside these (ADR-0034, decision 9).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DatabasesError {
    /// Another registered database has this name, ignoring case.
    #[error("another database has this name")]
    NameTaken,
    /// The owner stopped the read.
    #[error("the read was stopped")]
    Stopped,
    /// The service `sqlite-files` refused or failed.
    #[error("{error}")]
    Files {
        /// The service's error.
        error: SqliteFilesError,
    },
}

impl From<SqliteFilesError> for DatabasesError {
    fn from(error: SqliteFilesError) -> Self {
        Self::Files { error }
    }
}

impl PluginError for DatabasesError {
    /// The stable code of the error, for logs.
    fn code(&self) -> &'static str {
        match self {
            Self::NameTaken => "workers.databases.name_taken",
            Self::Stopped => "workers.databases.stopped",
            Self::Files { error } => error.code(),
        }
    }
}

/// `follow_databases`: the registered databases, as they are and then after
/// each change.
#[derive(Debug)]
pub enum FollowDatabasesChannel {}

impl ChannelSpec for FollowDatabasesChannel {
    const NAME: &'static str = "follow_databases";
    const DOCS: &'static str = "The registered databases, as they are and then after each change.";
    type Input = ();
    type Item = DatabaseListUpdate;
    type Error = DatabasesError;
}

/// `refresh_databases`: looks at each registered file again, such as when
/// the view opens.
#[derive(Debug)]
pub enum RefreshDatabasesCommand {}

impl CommandSpec for RefreshDatabasesCommand {
    const NAME: &'static str = "refresh_databases";
    const DOCS: &'static str = "Looks at each registered file again and updates their states.";
    type Input = ();
    type Output = ();
    type Error = DatabasesError;
}

/// `add_database`: registers the file that the owner chooses in the
/// system's file dialog, or nothing when the owner closes it.
#[derive(Debug)]
pub enum AddDatabaseCommand {}

impl CommandSpec for AddDatabaseCommand {
    const NAME: &'static str = "add_database";
    const DOCS: &'static str =
        "Registers the SQLite file that the owner chooses in the system's file dialog.";
    type Input = AddDatabase;
    type Output = Option<DatabaseView>;
    type Error = DatabasesError;
}

/// `rename_database`: names a database.
#[derive(Debug)]
pub enum RenameDatabaseCommand {}

impl CommandSpec for RenameDatabaseCommand {
    const NAME: &'static str = "rename_database";
    const DOCS: &'static str = "Gives a registered database another name.";
    type Input = RenameDatabase;
    type Output = ();
    type Error = DatabasesError;
}

/// `mark_database`: marks a database read-only, or lifts the mark.
#[derive(Debug)]
pub enum MarkDatabaseCommand {}

impl CommandSpec for MarkDatabaseCommand {
    const NAME: &'static str = "mark_database";
    const DOCS: &'static str = "Marks a registered database read-only, or lifts the mark.";
    type Input = MarkDatabase;
    type Output = ();
    type Error = DatabasesError;
}

/// `accept_database`: accepts the file that is at a registration's path now.
#[derive(Debug)]
pub enum AcceptDatabaseCommand {}

impl CommandSpec for AcceptDatabaseCommand {
    const NAME: &'static str = "accept_database";
    const DOCS: &'static str = "Accepts the file that is at a registered database's path now, such as after a program replaced it.";
    type Input = DatabaseRef;
    type Output = ();
    type Error = DatabasesError;
}

/// `remove_database`: forgets a registration and leaves the file as it is.
#[derive(Debug)]
pub enum RemoveDatabaseCommand {}

impl CommandSpec for RemoveDatabaseCommand {
    const NAME: &'static str = "remove_database";
    const DOCS: &'static str = "Forgets a registered database. The file stays as it is.";
    type Input = DatabaseRef;
    type Output = ();
    type Error = DatabasesError;
}

/// `database_structure`: a database's tables, views, indexes and triggers.
#[derive(Debug)]
pub enum DatabaseStructureCommand {}

impl CommandSpec for DatabaseStructureCommand {
    const NAME: &'static str = "database_structure";
    const DOCS: &'static str = "The tables, views, indexes and triggers of a registered database.";
    type Input = DatabaseRef;
    type Output = DatabaseStructure;
    type Error = DatabasesError;
}

/// `count_database_rows`: the rows of a table or view, within 10 seconds.
#[derive(Debug)]
pub enum CountRowsCommand {}

impl CommandSpec for CountRowsCommand {
    const NAME: &'static str = "count_database_rows";
    const DOCS: &'static str =
        "Counts the rows of a table or view of a registered database, as decimal text.";
    type Input = CountRows;
    type Output = String;
    type Error = DatabasesError;
}

/// `database_page`: one page of a table or view.
#[derive(Debug)]
pub enum DatabasePageCommand {}

impl CommandSpec for DatabasePageCommand {
    const NAME: &'static str = "database_page";
    const DOCS: &'static str = "One page of a table or view of a registered database.";
    type Input = PageDatabase;
    type Output = Page;
    type Error = DatabasesError;
}

/// `classify_database_statement`: what a statement does, without running
/// it.
#[derive(Debug)]
pub enum ClassifyStatementCommand {}

impl CommandSpec for ClassifyStatementCommand {
    const NAME: &'static str = "classify_database_statement";
    const DOCS: &'static str = "Whether a statement reads, writes rows or changes the schema, read from SQLite without running it.";
    type Input = ClassifyStatement;
    type Output = StatementClass;
    type Error = DatabasesError;
}

/// `read_database`: runs one statement that changes nothing.
#[derive(Debug)]
pub enum ReadDatabaseCommand {}

impl CommandSpec for ReadDatabaseCommand {
    const NAME: &'static str = "read_database";
    const DOCS: &'static str = "Runs one statement that changes nothing on a registered database, within the view's limits.";
    type Input = ReadDatabase;
    type Output = StatementRows;
    type Error = DatabasesError;
}

/// `stop_database_read`: stops a read that runs.
#[derive(Debug)]
pub enum StopReadCommand {}

impl CommandSpec for StopReadCommand {
    const NAME: &'static str = "stop_database_read";
    const DOCS: &'static str = "Stops a read of the view that runs; one that ended stays as it is.";
    type Input = StopRead;
    type Output = ();
    type Error = DatabasesError;
}

/// `run_database_change`: runs an owner's change as an owner operation.
#[derive(Debug)]
pub enum ChangeDatabaseCommand {}

impl CommandSpec for ChangeDatabaseCommand {
    const NAME: &'static str = "run_database_change";
    const DOCS: &'static str =
        "Runs the owner's change of a registered database, which the core journals before it runs.";
    type Input = ChangeDatabase;
    type Output = ChangeDone;
    type Error = DatabasesError;
}

/// The feature's commands, its channel and the limits that the view checks.
pub fn interface(feature: Interface) -> Interface {
    feature
        .command::<RefreshDatabasesCommand>()
        .command::<AddDatabaseCommand>()
        .command::<RenameDatabaseCommand>()
        .command::<MarkDatabaseCommand>()
        .command::<AcceptDatabaseCommand>()
        .command::<RemoveDatabaseCommand>()
        .command::<DatabaseStructureCommand>()
        .command::<CountRowsCommand>()
        .command::<DatabasePageCommand>()
        .command::<ClassifyStatementCommand>()
        .command::<ReadDatabaseCommand>()
        .command::<StopReadCommand>()
        .command::<ChangeDatabaseCommand>()
        .channel::<FollowDatabasesChannel>()
        .constant(
            "MAX_DATABASE_NAME_CHARACTERS",
            "The most characters of a database's name.",
            u32::try_from(MAX_NAME_CHARS).unwrap_or(u32::MAX),
        )
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::ipc::{PluginError, Validate};
    use mannie_plugin_sdk::services::sqlite_files::SqliteFilesError;
    use serde_json::json;

    use super::{DatabasesError, ReadDatabase, StopRead};

    #[test]
    fn an_error_of_the_service_crosses_json_inside_its_own_code_and_logs_the_services_code() {
        let error = DatabasesError::from(SqliteFilesError::Busy);

        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            json!({ "code": "files", "error": { "code": "busy" } })
        );
        assert_eq!(error.code(), "sqlite_files.busy");
        assert_eq!(
            serde_json::to_value(DatabasesError::NameTaken).unwrap(),
            json!({ "code": "name_taken" })
        );
    }

    #[test]
    fn a_reads_id_has_1_to_64_characters_without_control_characters() {
        let read = |run: Option<&str>| -> ReadDatabase {
            serde_json::from_value(json!({
                "key": "0199e0f2-0000-7000-8000-0000000000ee",
                "statement": { "text": "SELECT 1" },
                "run": run,
            }))
            .unwrap()
        };

        read(None).validate().unwrap();
        read(Some("run-1")).validate().unwrap();
        assert!(read(Some("")).validate().is_err());
        assert!(read(Some(&"r".repeat(65))).validate().is_err());
        assert!(
            StopRead {
                run: "a\nb".to_owned()
            }
            .validate()
            .is_err()
        );
    }
}
