//! Designer projects and their canvas documents (Designer ADR-0001 and ADR-0002).

use std::sync::Arc;

use mannie_plugin_sdk::services::store::{
    Constraint, Durability, Expect, Migration, ModuleStoreApi, Row, Statement, StoreError,
    StoreModule, WriteBatch,
};

use crate::document::{DocumentElement, DocumentFailure, DocumentSnapshot};

/// The prefix of the plugin's tables, which names its module of the store.
pub(crate) const PREFIX: &str = "designer";

/// The code of a name that another project has.
pub(crate) const NAME_TAKEN: &str = "name_taken";

const PROJECT_MIGRATION: Migration = Migration::new(
    1,
    "create the table of the projects",
    "CREATE TABLE designer_projects (
        id TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL UNIQUE COLLATE NOCASE,
        created_at INTEGER NOT NULL
    ) STRICT",
);

const MIGRATIONS: &[Migration] = &[
    PROJECT_MIGRATION,
    Migration::new(
        2,
        "keep the elements and revision of a project's canvas",
        "ALTER TABLE designer_projects ADD COLUMN document_revision INTEGER NOT NULL DEFAULT 0
            CHECK (document_revision BETWEEN 0 AND 4294967295);
         CREATE TABLE designer_elements (
            id TEXT PRIMARY KEY NOT NULL,
            project_id TEXT NOT NULL REFERENCES designer_projects (id) ON DELETE CASCADE,
            x REAL NOT NULL CHECK (x BETWEEN -32768 AND 32768),
            y REAL NOT NULL CHECK (y BETWEEN -32768 AND 32768),
            width REAL NOT NULL CHECK (width >= 1 AND x + width <= 32768),
            height REAL NOT NULL CHECK (height >= 1 AND y + height <= 32768),
            hue INTEGER NOT NULL CHECK (hue BETWEEN 0 AND 360),
            stacking_position INTEGER NOT NULL CHECK (stacking_position >= 0),
            created_at INTEGER NOT NULL
         ) STRICT;
         CREATE INDEX designer_elements_by_project_order
            ON designer_elements (project_id, stacking_position)",
    ),
];

const SELECT_PROJECTS: &str =
    "SELECT id, name, created_at FROM designer_projects ORDER BY created_at DESC, id DESC";

const INSERT_PROJECT: &str =
    "INSERT INTO designer_projects (id, name, created_at) VALUES (?1, ?2, ?3)";

// The left join distinguishes a missing project, an unopened document, and an
// initialized document whose element is gone in a single consistent read.
const SELECT_DOCUMENT: &str = "SELECT p.document_revision, e.id, e.x, e.y, e.width, e.height, e.hue
    FROM designer_projects p LEFT JOIN designer_elements e ON e.project_id = p.id
    WHERE p.id = ?1 ORDER BY e.stacking_position, e.id LIMIT 2";
const ADVANCE_REVISION: &str =
    "UPDATE designer_projects SET document_revision = document_revision + 1
    WHERE id = ?1 AND document_revision = ?2";
const INSERT_SEED: &str = "INSERT INTO designer_elements
    (id, project_id, x, y, width, height, hue, stacking_position, created_at)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)";
const SET_HUE: &str = "UPDATE designer_elements SET hue = ?3 WHERE project_id = ?1 AND id = ?2";

pub(crate) const MODULE: StoreModule = StoreModule::new(
    PREFIX,
    MIGRATIONS,
    &[
        SELECT_PROJECTS,
        INSERT_PROJECT,
        SELECT_DOCUMENT,
        ADVANCE_REVISION,
        INSERT_SEED,
        SET_HUE,
    ],
);

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

fn revision_batch(project_id: &str, revision: u32) -> WriteBatch {
    let advance = Statement::new(ADVANCE_REVISION)
        .bind(project_id)
        .bind(revision);
    WriteBatch::new().push(advance, Expect::Changes(1))
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

    /// Only an uninitialized project returns None. A missing or malformed
    /// saved element must never lead to a new seed.
    async fn read_document(
        &self,
        project_id: &str,
    ) -> Result<Option<DocumentSnapshot>, DocumentFailure> {
        let query = Statement::new(SELECT_DOCUMENT).bind(project_id);
        let rows = self.api.query(query).await?;
        let row = match rows.as_slice() {
            [] => return Err(DocumentFailure::ProjectMissing),
            [row] => row,
            _ => return Err(DocumentFailure::StorageFailed),
        };
        let revision = row.get::<u32>(0)?;
        let id = row.get::<Option<String>>(1)?;

        if revision == 0 && id.is_none() {
            return Ok(None);
        }

        let element = DocumentElement {
            id: id.ok_or(DocumentFailure::StorageFailed)?,
            x: row.get(2)?,
            y: row.get(3)?,
            width: row.get(4)?,
            height: row.get(5)?,
            hue: u16::try_from(row.get::<u32>(6)?)
                .map_err(|_out_of_range| StoreError::InvalidValue { column: 6 })?,
        };

        if revision == 0 || !element.valid() {
            return Err(DocumentFailure::StorageFailed);
        }

        Ok(Some(DocumentSnapshot { revision, element }))
    }

    pub(crate) async fn document(
        &self,
        project_id: &str,
        created_at: i64,
    ) -> Result<DocumentSnapshot, DocumentFailure> {
        if let Some(document) = self.read_document(project_id).await? {
            return Ok(document);
        }

        let element = DocumentElement::seed();
        let insert = Statement::new(INSERT_SEED)
            .bind(element.id.as_str())
            .bind(project_id)
            .bind(element.x)
            .bind(element.y)
            .bind(element.width)
            .bind(element.height)
            .bind(u32::from(element.hue))
            .bind(created_at);
        let batch = revision_batch(project_id, 0).push(insert, Expect::Changes(1));

        match self.api.write(batch, Durability::Full).await {
            Ok(_) => Ok(DocumentSnapshot {
                revision: 1,
                element,
            }),
            // A concurrent first open has already seeded the document.
            Err(StoreError::Expectation { index: 0 }) => self
                .read_document(project_id)
                .await?
                .ok_or(DocumentFailure::StorageFailed),
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) async fn set_document_hue(
        &self,
        project_id: &str,
        confirmed: &DocumentSnapshot,
        hue: u16,
    ) -> Result<DocumentSnapshot, DocumentFailure> {
        let revision = confirmed
            .revision
            .checked_add(1)
            .ok_or(DocumentFailure::StorageFailed)?;

        let update = Statement::new(SET_HUE)
            .bind(project_id)
            .bind(confirmed.element.id.as_str())
            .bind(u32::from(hue));
        let batch = revision_batch(project_id, confirmed.revision).push(update, Expect::Changes(1));

        match self.api.write(batch, Durability::Full).await {
            Ok(_) => {
                let mut element = confirmed.element.clone();
                element.hue = hue;
                Ok(DocumentSnapshot { revision, element })
            }
            Err(StoreError::Expectation { .. }) => Err(DocumentFailure::StaleRevision),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests;
