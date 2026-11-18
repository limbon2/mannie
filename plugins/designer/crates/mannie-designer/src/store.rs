//! The plugin's table of projects (plugins/designer/docs/README.md) holds the
//! project's ID, its name and when the owner created it. SQLite's `NOCASE`
//! collation ignores ASCII case differences when it checks name uniqueness.

use std::sync::Arc;

use mannie_plugin_sdk::services::store::{
    Constraint, Durability, Expect, Migration, ModuleStoreApi, Row, Statement, StoreError,
    StoreModule, WriteBatch,
};

/// The prefix of the plugin's tables, which names its module of the store.
pub(crate) const PREFIX: &str = "designer";

/// The code of a name that another project has.
pub(crate) const NAME_TAKEN: &str = "name_taken";

const MIGRATIONS: &[Migration] = &[Migration::new(
    1,
    "create the table of the projects",
    "CREATE TABLE designer_projects (
        id TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL UNIQUE COLLATE NOCASE,
        created_at INTEGER NOT NULL
    ) STRICT",
)];

const SELECT_PROJECTS: &str =
    "SELECT id, name, created_at FROM designer_projects ORDER BY created_at DESC, id DESC";

const INSERT_PROJECT: &str =
    "INSERT INTO designer_projects (id, name, created_at) VALUES (?1, ?2, ?3)";

/// The plugin's module of the store: one table, `designer_projects`, with the
/// statements that read and change it.
pub(crate) const MODULE: StoreModule =
    StoreModule::new(PREFIX, MIGRATIONS, &[SELECT_PROJECTS, INSERT_PROJECT]);

/// A project as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) created_at: i64,
}

fn project_of(row: &Row) -> Result<ProjectRecord, StoreError> {
    Ok(ProjectRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        created_at: row.get(2)?,
    })
}

/// Why the store refused a write that the plugin maps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WriteFailure {
    /// A constraint that the batch mapped failed, with the plugin's code.
    Constraint(&'static str),
    /// The store failed.
    Store(StoreError),
}

impl From<StoreError> for WriteFailure {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Constraint { code } => Self::Constraint(code),
            other => Self::Store(other),
        }
    }
}

/// The plugin's table, through its module of the store.
#[derive(Debug, Clone)]
pub(crate) struct DesignerStore {
    api: Arc<dyn ModuleStoreApi>,
}

impl DesignerStore {
    pub(crate) fn new(api: Arc<dyn ModuleStoreApi>) -> Self {
        Self { api }
    }

    /// Every project, the newest first.
    pub(crate) async fn projects(&self) -> Result<Vec<ProjectRecord>, WriteFailure> {
        let rows = self.api.query(Statement::new(SELECT_PROJECTS)).await?;
        rows.iter()
            .map(project_of)
            .collect::<Result<_, _>>()
            .map_err(WriteFailure::from)
    }

    pub(crate) async fn insert_project(&self, project: &ProjectRecord) -> Result<(), WriteFailure> {
        let insert = Statement::new(INSERT_PROJECT)
            .bind(project.id.as_str())
            .bind(project.name.as_str())
            .bind(project.created_at);
        let batch = WriteBatch::new()
            .push(insert, Expect::Changes(1))
            .on_constraint(
                Constraint::Key {
                    table: "designer_projects",
                    columns: &["name"],
                },
                NAME_TAKEN,
            );
        self.api
            .write(batch, Durability::Full)
            .await
            .map(drop)
            .map_err(WriteFailure::from)
    }
}
