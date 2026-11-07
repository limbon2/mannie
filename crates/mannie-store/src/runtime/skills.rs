//! The switches of skills (ADR-0061, decision 5; docs/features/skills.md,
//! data): the owner's choice of whether agents see a skill of this computer or
//! of a project. A skill without a row keeps its place's default, which the
//! runtime knows, so the store only keeps choices.

use mannie_types::ids::ProjectId;
use rusqlite::{Connection, Row, params};

use super::RuntimeStoreError;
use super::rows::parse_id;
use crate::writer::TransactError;
use crate::{Durability, Store};

const SELECT_SWITCHES: &str = "SELECT project_id, name, is_on, digest FROM runtime_skill_switches \
     ORDER BY COALESCE(project_id, ''), name";
const DELETE_SWITCH: &str =
    "DELETE FROM runtime_skill_switches WHERE COALESCE(project_id, '') = ?1 AND name = ?2";
const INSERT_SWITCH: &str = "INSERT INTO runtime_skill_switches \
     (project_id, name, is_on, digest, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[SELECT_SWITCHES, DELETE_SWITCH, INSERT_SWITCH];

/// The owner's choice for one skill, as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSkillSwitch {
    /// The skill's project, or `None` for a skill of this computer.
    pub project_id: Option<ProjectId>,
    /// The skill's name.
    pub name: String,
    /// Whether agents see it.
    pub is_on: bool,
    /// For a project's skill that is on: the digest of what agents' list
    /// showed of it when the owner turned it on.
    pub digest: Option<String>,
}

struct RawSwitch {
    project_id: Option<String>,
    name: String,
    is_on: bool,
    digest: Option<String>,
}

fn switch_from_row(row: &Row<'_>) -> rusqlite::Result<RawSwitch> {
    Ok(RawSwitch {
        project_id: row.get(0)?,
        name: row.get(1)?,
        is_on: row.get(2)?,
        digest: row.get(3)?,
    })
}

fn switches_in(connection: &Connection) -> Result<Vec<StoredSkillSwitch>, RuntimeStoreError> {
    let raw: Vec<RawSwitch> = {
        let mut select = connection.prepare_cached(SELECT_SWITCHES)?;
        let rows = select.query_map([], switch_from_row)?;
        rows.collect::<Result<_, _>>()?
    };
    raw.into_iter()
        .map(|row| {
            let project_id = row
                .project_id
                .as_deref()
                .map(|id| parse_id(id, ProjectId::parse))
                .transpose()?;
            Ok(StoredSkillSwitch {
                project_id,
                name: row.name,
                is_on: row.is_on,
                digest: row.digest,
            })
        })
        .collect()
}

fn rejected(error: TransactError<RuntimeStoreError>) -> RuntimeStoreError {
    match error {
        TransactError::Rejected(error) => error,
        TransactError::Store(error) => RuntimeStoreError::Store(error),
    }
}

impl Store {
    /// Every switch that the owner set, by place and name.
    pub async fn skill_switches(&self) -> Result<Vec<StoredSkillSwitch>, RuntimeStoreError> {
        self.read(|connection| Ok(switches_in(connection))).await?
    }

    /// Sets the switch of one skill, replacing an earlier one of the same
    /// place and name. A project that does not exist fails the write.
    pub async fn set_skill_switch(
        &self,
        switch: &StoredSkillSwitch,
    ) -> Result<(), RuntimeStoreError> {
        let project = switch
            .project_id
            .as_ref()
            .map(|project| project.as_str().to_owned());
        let place = project.clone().unwrap_or_default();
        let name = switch.name.clone();
        let is_on = switch.is_on;
        let digest = switch.digest.clone();
        let now = self.clock().now().as_millis();
        self.transact(Durability::Full, move |transaction| {
            transaction
                .prepare_cached(DELETE_SWITCH)?
                .execute(params![place, name])?;
            transaction
                .prepare_cached(INSERT_SWITCH)?
                .execute(params![project, name, is_on, digest, now])?;
            Ok(())
        })
        .await
        .map_err(rejected)
    }
}
