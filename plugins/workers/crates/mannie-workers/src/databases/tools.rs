//! The tools of the feature Databases (plugins/workers/docs/databases.md,
//! agents; ADR-0056, decisions 5 to 7). `describe_database` reads a
//! database's structure and `query_database` runs one read; both run
//! without asking. `change_database` runs 1 to 16 statements in one
//! transaction, as an operation that the core's policy decides on and its
//! journal records.
//!
//! Prepare reads only the feature's list of databases: it checks the
//! arguments and returns each statement with its values as a database
//! access, from which the core reads what the statement does. Execute reads
//! the arguments again, and the service runs a change only with the call's
//! ticket, which holds exactly the statements and values that were allowed.

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::points::tools::{
    DatabaseAccess, Tool, ToolAccess, ToolCall, ToolExecution, ToolRefusal, ToolResult, ToolSpec,
};
use mannie_plugin_sdk::services::sqlite_files::{ReadLimits, SqlValue, Statement};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::commands::DatabaseView;
use super::output::{Room, fitted, query_result, structure_text};
use super::registry::{Databases, is_available};
use super::texts;
use crate::schema::parameters;

/// The tool that reads a database's structure.
pub const DESCRIBE_DATABASE: &str = "describe_database";

/// The tool that runs one read.
pub const QUERY_DATABASE: &str = "query_database";

/// The tool that changes a database.
pub const CHANGE_DATABASE: &str = "change_database";

/// The agents that read databases: the owner's conversations, the general
/// child agent and the explore agent.
pub(crate) const READING_AGENTS: [&str; 3] = ["conversation", "general", "explore"];

/// The agents that change databases: the explore agent only reads.
pub(crate) const CHANGING_AGENTS: [&str; 2] = ["conversation", "general"];

/// The code of a call that names no registered database, which the call's
/// card words.
pub(crate) const UNKNOWN_DATABASE: &str = "unknown_database";

/// The code of a call on a database that cannot run statements now.
pub(crate) const DATABASE_UNUSABLE: &str = "database_unusable";

/// The code of a call whose arguments did not fit when it was about to run.
pub(crate) const INVALID_ARGUMENTS: &str = "invalid_arguments";

/// The most characters of a statement.
const MAX_STATEMENT_CHARS: usize = 100_000;

/// The most statements of a change.
const MAX_STATEMENTS: usize = 16;

/// The limits of a tool's read: up to 10,000 rows and 4 MiB within 30
/// seconds, of which the model reads the first rows and the archive keeps
/// up to 2 MiB.
const TOOL_READ_LIMITS: ReadLimits = ReadLimits {
    rows: 10_000,
    bytes: 4 * 1024 * 1024,
    milliseconds: 30_000,
};

/// The arguments of `describe_database`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DescribeArguments {
    /// The name of a registered database, as the list of SQLite databases
    /// gives it.
    database: String,
}

/// The arguments of `query_database`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct QueryArguments {
    /// The name of a registered database, as the list of SQLite databases
    /// gives it.
    database: String,
    /// One SQL statement that changes nothing, with ?1, ?2 and so on for
    /// its parameters: at most 100,000 characters.
    statement: String,
    /// The parameters' values in order: text, numbers, true or false, or
    /// null.
    parameters: Vec<ParameterValue>,
}

/// The arguments of `change_database`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ChangeArguments {
    /// The name of a registered database, as the list of SQLite databases
    /// gives it.
    database: String,
    /// The statements, 1 to 16, which run in order in one transaction.
    statements: Vec<StatementArgument>,
}

/// One statement of `change_database`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StatementArgument {
    /// One SQL statement, with ?1, ?2 and so on for its parameters: at most
    /// 100,000 characters.
    statement: String,
    /// The parameters' values in order: text, numbers, true or false, or
    /// null.
    parameters: Vec<ParameterValue>,
}

/// A parameter's value, as JSON writes it.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
enum ParameterValue {
    /// A whole number.
    Integer(i64),
    /// A number with a fraction.
    Real(f64),
    /// True or false, which SQLite stores as 1 or 0.
    Boolean(bool),
    /// Text.
    Text(String),
    /// Null.
    Null,
}

impl ParameterValue {
    /// The value as SQLite stores it.
    fn into_sql(self) -> SqlValue {
        match self {
            Self::Integer(value) => SqlValue::integer(value),
            Self::Real(value) => SqlValue::real(value),
            Self::Boolean(value) => SqlValue::integer(i64::from(value)),
            Self::Text(value) => SqlValue::text(value),
            Self::Null => SqlValue::Null,
        }
    }
}

/// The arguments of a call, read as `T`, or what the model reads of
/// arguments that do not fit.
fn arguments<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, ToolRefusal> {
    serde_json::from_str(text).map_err(|error| {
        ToolRefusal::new(format!(
            "The arguments do not fit the tool's schema: {error}"
        ))
    })
}

/// The registered database with this name that can run statements now.
fn usable(databases: &Databases, name: &str) -> Result<DatabaseView, ToolRefusal> {
    let database = databases.by_name(name).ok_or_else(|| {
        ToolRefusal::new(texts::unknown_database(name)).with_code(UNKNOWN_DATABASE)
    })?;
    if is_available(&database) {
        Ok(database)
    } else {
        Err(ToolRefusal::new(texts::unusable_database(&database.name)).with_code(DATABASE_UNUSABLE))
    }
}

/// A statement of a call, with its values.
fn statement(text: String, values: Vec<ParameterValue>) -> Result<Statement, ToolRefusal> {
    if text.chars().count() > MAX_STATEMENT_CHARS {
        return Err(ToolRefusal::new(texts::STATEMENT_TOO_LONG));
    }
    let parameters = values.into_iter().map(ParameterValue::into_sql).collect();
    Ok(Statement::new(text, parameters))
}

/// The room of a call's result.
fn room(execution: &ToolExecution) -> Room<'_> {
    Room {
        limit: usize::try_from(execution.result_limit).unwrap_or(usize::MAX),
        archive_ids: &execution.archive_ids,
    }
}

/// `describe_database`.
#[derive(Debug)]
pub(crate) struct DescribeDatabase {
    databases: Databases,
}

impl DescribeDatabase {
    pub(crate) fn new(databases: Databases) -> Self {
        Self { databases }
    }
}

impl Tool for DescribeDatabase {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            DESCRIBE_DATABASE,
            texts::DESCRIBE_DATABASE,
            parameters::<DescribeArguments>(),
        )
    }

    /// A read of the plugin's own list: the call has no access.
    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let checked = arguments::<DescribeArguments>(&call.arguments)
            .and_then(|arguments| usable(&self.databases, &arguments.database))
            .map(|_database| Vec::new());
        Box::pin(std::future::ready(checked))
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let databases = self.databases.clone();
        Box::pin(async move {
            let database = match arguments::<DescribeArguments>(&execution.arguments)
                .and_then(|arguments| usable(&databases, &arguments.database))
            {
                Ok(database) => database,
                Err(refusal) => return refused(refusal),
            };
            match databases.files().structure(database.key.clone()).await {
                Ok(structure) => {
                    let text = structure_text(&structure);
                    let tables = structure.tables.len();
                    let shown = fitted(&text, &text, usize::MAX, room(&execution));
                    let result = ToolResult::new(shown.text).with_data(json!({
                        "database": database.name,
                        "tables": tables,
                    }));
                    match shown.archive {
                        Some(archive) => result.with_archives(vec![archive]),
                        None => result,
                    }
                }
                Err(error) => failed(&databases, error).await,
            }
        })
    }
}

/// `query_database`.
#[derive(Debug)]
pub(crate) struct QueryDatabase {
    databases: Databases,
}

impl QueryDatabase {
    pub(crate) fn new(databases: Databases) -> Self {
        Self { databases }
    }
}

/// A checked call of `query_database`.
fn checked_query(
    databases: &Databases,
    text: &str,
) -> Result<(DatabaseView, Statement), ToolRefusal> {
    let arguments = arguments::<QueryArguments>(text)?;
    let database = usable(databases, &arguments.database)?;
    let statement = statement(arguments.statement, arguments.parameters)?;
    Ok((database, statement))
}

impl Tool for QueryDatabase {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            QUERY_DATABASE,
            texts::QUERY_DATABASE,
            parameters::<QueryArguments>(),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let checked =
            checked_query(&self.databases, &call.arguments).map(|(database, statement)| {
                vec![ToolAccess::Database(DatabaseAccess::new(
                    database.key,
                    statement,
                ))]
            });
        Box::pin(std::future::ready(checked))
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let databases = self.databases.clone();
        Box::pin(async move {
            let (database, statement) = match checked_query(&databases, &execution.arguments) {
                Ok(checked) => checked,
                Err(refusal) => return refused(refusal),
            };
            let read = databases
                .files()
                .read(database.key.clone(), statement, TOOL_READ_LIMITS)
                .await;
            match read {
                Ok(rows) => {
                    let data = json!({
                        "database": database.name,
                        "rows": rows.rows.len(),
                        "columns": rows.columns.len(),
                        "cut": rows.cut.is_some(),
                    });
                    query_result(&rows, room(&execution)).with_data(data)
                }
                Err(error) => failed(&databases, error).await,
            }
        })
    }
}

/// `change_database`.
#[derive(Debug)]
pub(crate) struct ChangeDatabase {
    databases: Databases,
}

impl ChangeDatabase {
    pub(crate) fn new(databases: Databases) -> Self {
        Self { databases }
    }
}

/// A checked call of `change_database`.
fn checked_change(
    databases: &Databases,
    text: &str,
) -> Result<(DatabaseView, Vec<Statement>), ToolRefusal> {
    let arguments = arguments::<ChangeArguments>(text)?;
    let database = usable(databases, &arguments.database)?;
    let count = arguments.statements.len();
    if count == 0 || count > MAX_STATEMENTS {
        return Err(ToolRefusal::new(texts::STATEMENT_COUNT));
    }
    let statements = arguments
        .statements
        .into_iter()
        .map(|argument| statement(argument.statement, argument.parameters))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((database, statements))
}

impl Tool for ChangeDatabase {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            CHANGE_DATABASE,
            texts::CHANGE_DATABASE,
            parameters::<ChangeArguments>(),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let checked =
            checked_change(&self.databases, &call.arguments).map(|(database, statements)| {
                statements
                    .into_iter()
                    .map(|statement| {
                        ToolAccess::Database(DatabaseAccess::new(database.key.clone(), statement))
                    })
                    .collect()
            });
        Box::pin(std::future::ready(checked))
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let databases = self.databases.clone();
        Box::pin(async move {
            let (database, statements) = match checked_change(&databases, &execution.arguments) {
                Ok(checked) => checked,
                Err(refusal) => return refused(refusal),
            };
            let Some(ticket) = execution.database.clone() else {
                // The core gives every call with writes a ticket; a call
                // without one was not allowed to write.
                return ToolResult::new(texts::failure(
                    &mannie_plugin_sdk::services::sqlite_files::SqliteFilesError::NotAllowed,
                ))
                .with_failure("not_allowed");
            };
            let written = databases
                .files()
                .write(ticket, database.key.clone(), statements)
                .await;
            match written {
                Ok(outcomes) => {
                    let mut text = String::new();
                    let mut changes = Vec::with_capacity(outcomes.len());
                    for (index, outcome) in outcomes.iter().enumerate() {
                        changes.push(outcome.changes);
                        text.push_str(&changed_line(index + 1, outcome.changes));
                        if let Some(rows) = &outcome.rows {
                            let returned = query_result(rows, room(&execution));
                            text.push_str(&returned.output);
                        }
                    }
                    ToolResult::new(text).with_data(json!({
                        "database": database.name,
                        "changes": changes,
                    }))
                }
                Err(error) => failed(&databases, error).await,
            }
        })
    }
}

/// What one statement of a change did, as a line.
fn changed_line(number: usize, changes: u64) -> String {
    match changes {
        1 => format!("Statement {number}: changed 1 row.\n"),
        _other => format!("Statement {number}: changed {changes} rows.\n"),
    }
}

/// The result of a call that the arguments or the list refused when it was
/// about to run.
fn refused(refusal: ToolRefusal) -> ToolResult {
    let code = refusal.code.unwrap_or_else(|| INVALID_ARGUMENTS.to_owned());
    ToolResult::new(refusal.detail).with_failure(code)
}

/// The result of a call that the service refused or failed, after the list
/// is read again when the error is about the file.
async fn failed(
    databases: &Databases,
    error: mannie_plugin_sdk::services::sqlite_files::SqliteFilesError,
) -> ToolResult {
    let error = databases.noticed(error).await;
    let code = error.code().trim_start_matches("sqlite_files.").to_owned();
    ToolResult::new(texts::failure(&error)).with_failure(code)
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::services::sqlite_files::SqlValue;
    use serde_json::json;

    use super::{ChangeArguments, ParameterValue, QueryArguments, changed_line};
    use crate::schema::parameters;

    #[test]
    fn parameters_are_json_values_that_become_sqlite_values() {
        let arguments: QueryArguments = serde_json::from_value(json!({
            "database": "Inventory",
            "statement": "SELECT ?1, ?2, ?3, ?4, ?5",
            "parameters": [12, 1.5, true, "bolt", null],
        }))
        .unwrap();

        let values: Vec<SqlValue> = arguments
            .parameters
            .into_iter()
            .map(ParameterValue::into_sql)
            .collect();

        assert_eq!(
            values,
            vec![
                SqlValue::integer(12),
                SqlValue::real(1.5),
                SqlValue::integer(1),
                SqlValue::text("bolt"),
                SqlValue::Null,
            ]
        );
    }

    #[test]
    fn the_schemas_require_every_argument_and_offer_each_kind_of_value() {
        let query = parameters::<QueryArguments>();
        let change = parameters::<ChangeArguments>();

        assert_eq!(
            query["required"],
            json!(["database", "parameters", "statement"])
        );
        let value = &query["properties"]["parameters"]["items"];
        let kinds: Vec<&str> = value["anyOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|branch| branch["type"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["integer", "number", "boolean", "string", "null"]);
        let statement = &change["properties"]["statements"]["items"];
        assert_eq!(statement["required"], json!(["parameters", "statement"]));
        assert_eq!(statement["additionalProperties"], json!(false));
    }

    #[test]
    fn a_change_says_how_many_rows_each_statement_changed() {
        assert_eq!(changed_line(1, 1), "Statement 1: changed 1 row.\n");
        assert_eq!(changed_line(2, 0), "Statement 2: changed 0 rows.\n");
    }
}
