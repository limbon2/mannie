//! The system tools repository: the grants of conversations, the grants for
//! a project or for every conversation, the command rules, the rules for
//! commands on remote hosts and the rules for runs of installed programs
//! (docs/features/system-tools/operations-and-approvals.md; ADR-0036;
//! ADR-0038). The journal of operations lives in the tool call parts of the
//! runtime's tables, so the runtime stores each change of an operation
//! together with its event.

use mannie_types::clock::UtcMillis;
use mannie_types::ids::{ConversationId, GrantId, ProjectId, RuleId};
use mannie_types::tools::{
    CommandRule, ConversationGrant, GrantScope, ProgramEffectKey, ProgramRule, ProgramTarget,
    RemoteCommandRule, RemoteTarget, SharedGrant,
};
use rusqlite::{OptionalExtension, params};

use crate::error::StoreError;
use crate::writer::TransactError;
use crate::{Durability, Store};

/// The version of the JSON documents this module writes.
const DOCUMENT_VERSION: u32 = 1;

const SELECT_GRANTS: &str = "SELECT id, version, scope, created_at FROM tools_grants \
     WHERE conversation_id = ?1 ORDER BY created_at, id";
const SELECT_CONVERSATION_EXISTS: &str = "SELECT 1 FROM runtime_conversations WHERE id = ?1";
const INSERT_GRANT: &str = "INSERT INTO tools_grants (id, conversation_id, version, scope, created_at) \
     VALUES (?1, ?2, ?3, ?4, ?5)";
const DELETE_GRANT: &str = "DELETE FROM tools_grants WHERE id = ?1 AND conversation_id = ?2";
const SELECT_RULES: &str =
    "SELECT id, version, words, folder, created_at FROM tools_rules ORDER BY created_at, id";
const SELECT_RULE: &str = "SELECT id, version, words, folder, created_at FROM tools_rules \
     WHERE words = ?1 AND COALESCE(folder, '') = COALESCE(?2, '')";
const INSERT_RULE: &str = "INSERT INTO tools_rules (id, version, words, folder, created_at) \
     VALUES (?1, ?2, ?3, ?4, ?5)";
const DELETE_RULE: &str = "DELETE FROM tools_rules WHERE id = ?1";
const SELECT_SHARED_GRANTS: &str = "SELECT id, project_id, version, scope, created_at \
     FROM tools_shared_grants ORDER BY created_at, id";
const SELECT_PROJECT_EXISTS: &str = "SELECT 1 FROM runtime_projects WHERE id = ?1";
// The unique index keeps one copy of a grant for the same project, or for
// everyone, should two approvals save it at once.
const INSERT_SHARED_GRANT: &str = "INSERT INTO tools_shared_grants \
     (id, project_id, version, scope, created_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING";
const DELETE_SHARED_GRANT: &str = "DELETE FROM tools_shared_grants WHERE id = ?1";
const SELECT_REMOTE_RULES: &str = "SELECT id, plugin, target, endpoint, label, version, words, \
     created_at FROM tools_remote_rules ORDER BY created_at, id";
const SELECT_REMOTE_RULE: &str = "SELECT id, plugin, target, endpoint, label, version, words, \
     created_at FROM tools_remote_rules \
     WHERE plugin = ?1 AND target = ?2 AND endpoint = ?3 AND words = ?4";
const INSERT_REMOTE_RULE: &str = "INSERT INTO tools_remote_rules \
     (id, plugin, target, endpoint, label, version, words, created_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)";
const DELETE_REMOTE_RULE: &str = "DELETE FROM tools_remote_rules WHERE id = ?1";
const SELECT_PROGRAM_RULES: &str = "SELECT id, plugin, name, program, version, words, effects, \
     folder, created_at FROM tools_program_rules ORDER BY created_at, id";
const SELECT_PROGRAM_RULE: &str = "SELECT id, plugin, name, program, version, words, effects, \
     folder, created_at FROM tools_program_rules \
     WHERE plugin = ?1 AND name = ?2 AND program = ?3 AND words = ?4 AND effects = ?5 \
     AND COALESCE(folder, '') = COALESCE(?6, '')";
const INSERT_PROGRAM_RULE: &str = "INSERT INTO tools_program_rules \
     (id, plugin, name, program, version, words, effects, folder, created_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";
const DELETE_PROGRAM_RULE: &str = "DELETE FROM tools_program_rules WHERE id = ?1";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    SELECT_GRANTS,
    SELECT_CONVERSATION_EXISTS,
    INSERT_GRANT,
    DELETE_GRANT,
    SELECT_RULES,
    SELECT_RULE,
    INSERT_RULE,
    DELETE_RULE,
    SELECT_SHARED_GRANTS,
    SELECT_PROJECT_EXISTS,
    INSERT_SHARED_GRANT,
    DELETE_SHARED_GRANT,
    SELECT_REMOTE_RULES,
    SELECT_REMOTE_RULE,
    INSERT_REMOTE_RULE,
    DELETE_REMOTE_RULE,
    SELECT_PROGRAM_RULES,
    SELECT_PROGRAM_RULE,
    INSERT_PROGRAM_RULE,
    DELETE_PROGRAM_RULE,
];

/// Why grants or rules could not be read or saved.
#[derive(Debug, thiserror::Error)]
pub enum ToolsStoreError {
    /// The conversation does not exist.
    #[error("the conversation does not exist")]
    ConversationNotFound,
    /// The project does not exist.
    #[error("the project does not exist")]
    ProjectNotFound,
    /// Stored data does not have the shape this version expects.
    #[error("stored data is not valid: {what}")]
    InvalidData {
        /// What is wrong.
        what: String,
    },
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl ToolsStoreError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::ConversationNotFound => "tools_store.conversation_not_found",
            Self::ProjectNotFound => "tools_store.project_not_found",
            Self::InvalidData { .. } => "tools_store.invalid_data",
            Self::Store(error) => error.code(),
        }
    }

    fn invalid(what: impl Into<String>) -> Self {
        Self::InvalidData { what: what.into() }
    }
}

impl From<rusqlite::Error> for ToolsStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(StoreError::query(error))
    }
}

impl From<TransactError<Self>> for ToolsStoreError {
    fn from(error: TransactError<Self>) -> Self {
        match error {
            TransactError::Rejected(error) => error,
            TransactError::Store(error) => Self::Store(error),
        }
    }
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, ToolsStoreError> {
    serde_json::to_string(value)
        .map_err(|error| ToolsStoreError::invalid(format!("could not encode a document: {error}")))
}

fn check_version(version: i64, what: &str) -> Result<(), ToolsStoreError> {
    if version == i64::from(DOCUMENT_VERSION) {
        Ok(())
    } else {
        Err(ToolsStoreError::invalid(format!(
            "{what} has document version {version}"
        )))
    }
}

struct RawGrant {
    id: String,
    version: i64,
    scope: String,
    created_at: i64,
}

impl RawGrant {
    fn into_grant(self) -> Result<ConversationGrant, ToolsStoreError> {
        check_version(self.version, "a grant")?;
        let scope: GrantScope = serde_json::from_str(&self.scope)
            .map_err(|error| ToolsStoreError::invalid(format!("grant {}: {error}", self.id)))?;
        Ok(ConversationGrant {
            id: GrantId::parse(&self.id)
                .map_err(|error| ToolsStoreError::invalid(error.to_string()))?,
            scope,
            created_at: UtcMillis::from_millis(self.created_at),
        })
    }
}

struct RawSharedGrant {
    id: String,
    project_id: Option<String>,
    version: i64,
    scope: String,
    created_at: i64,
}

impl RawSharedGrant {
    fn into_grant(self) -> Result<SharedGrant, ToolsStoreError> {
        check_version(self.version, "a shared grant")?;
        let scope: GrantScope = serde_json::from_str(&self.scope)
            .map_err(|error| ToolsStoreError::invalid(format!("grant {}: {error}", self.id)))?;
        let project_id = self
            .project_id
            .as_deref()
            .map(ProjectId::parse)
            .transpose()
            .map_err(|error| ToolsStoreError::invalid(error.to_string()))?;
        Ok(SharedGrant {
            id: GrantId::parse(&self.id)
                .map_err(|error| ToolsStoreError::invalid(error.to_string()))?,
            project_id,
            scope,
            created_at: UtcMillis::from_millis(self.created_at),
        })
    }
}

struct RawRule {
    id: String,
    version: i64,
    words: String,
    folder: Option<String>,
    created_at: i64,
}

fn rule_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawRule> {
    Ok(RawRule {
        id: row.get(0)?,
        version: row.get(1)?,
        words: row.get(2)?,
        folder: row.get(3)?,
        created_at: row.get(4)?,
    })
}

impl RawRule {
    fn into_rule(self) -> Result<CommandRule, ToolsStoreError> {
        check_version(self.version, "a rule")?;
        let words: Vec<String> = serde_json::from_str(&self.words)
            .map_err(|error| ToolsStoreError::invalid(format!("rule {}: {error}", self.id)))?;
        Ok(CommandRule {
            id: RuleId::parse(&self.id)
                .map_err(|error| ToolsStoreError::invalid(error.to_string()))?,
            words,
            folder: self.folder,
            created_at: UtcMillis::from_millis(self.created_at),
        })
    }
}

struct RawRemoteRule {
    id: String,
    plugin: String,
    target: String,
    endpoint: String,
    label: String,
    version: i64,
    words: String,
    created_at: i64,
}

fn remote_rule_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawRemoteRule> {
    Ok(RawRemoteRule {
        id: row.get(0)?,
        plugin: row.get(1)?,
        target: row.get(2)?,
        endpoint: row.get(3)?,
        label: row.get(4)?,
        version: row.get(5)?,
        words: row.get(6)?,
        created_at: row.get(7)?,
    })
}

impl RawRemoteRule {
    fn into_rule(self) -> Result<RemoteCommandRule, ToolsStoreError> {
        check_version(self.version, "a remote rule")?;
        let words: Vec<String> = serde_json::from_str(&self.words).map_err(|error| {
            ToolsStoreError::invalid(format!("remote rule {}: {error}", self.id))
        })?;
        Ok(RemoteCommandRule {
            id: RuleId::parse(&self.id)
                .map_err(|error| ToolsStoreError::invalid(error.to_string()))?,
            target: RemoteTarget {
                plugin: self.plugin,
                target: self.target,
                label: self.label,
                endpoint: self.endpoint,
            },
            words,
            created_at: UtcMillis::from_millis(self.created_at),
        })
    }
}

struct RawProgramRule {
    id: String,
    plugin: String,
    name: String,
    program: String,
    version: i64,
    words: String,
    effects: String,
    folder: Option<String>,
    created_at: i64,
}

fn program_rule_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawProgramRule> {
    Ok(RawProgramRule {
        id: row.get(0)?,
        plugin: row.get(1)?,
        name: row.get(2)?,
        program: row.get(3)?,
        version: row.get(4)?,
        words: row.get(5)?,
        effects: row.get(6)?,
        folder: row.get(7)?,
        created_at: row.get(8)?,
    })
}

impl RawProgramRule {
    fn into_rule(self) -> Result<ProgramRule, ToolsStoreError> {
        check_version(self.version, "a program rule")?;
        let invalid = |error: serde_json::Error| {
            ToolsStoreError::invalid(format!("program rule {}: {error}", self.id))
        };
        let words: Vec<String> = serde_json::from_str(&self.words).map_err(invalid)?;
        let effects: Vec<ProgramEffectKey> =
            serde_json::from_str(&self.effects).map_err(invalid)?;
        Ok(ProgramRule {
            id: RuleId::parse(&self.id)
                .map_err(|error| ToolsStoreError::invalid(error.to_string()))?,
            target: ProgramTarget {
                plugin: self.plugin,
                name: self.name,
                program: self.program,
                effects,
            },
            words,
            folder: self.folder,
            created_at: UtcMillis::from_millis(self.created_at),
        })
    }
}

impl Store {
    /// The grants of a conversation, the oldest first.
    pub async fn grants(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<ConversationGrant>, ToolsStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        let raw: Vec<RawGrant> = self
            .read(move |connection| {
                let mut select = connection.prepare_cached(SELECT_GRANTS)?;
                let rows = select.query_map([conversation], |row| {
                    Ok(RawGrant {
                        id: row.get(0)?,
                        version: row.get(1)?,
                        scope: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                })?;
                rows.collect()
            })
            .await?;
        raw.into_iter().map(RawGrant::into_grant).collect()
    }

    /// Adds grants to a conversation, with full durability, and returns the
    /// grants that were new. A scope that the conversation already has is
    /// not added again.
    pub async fn add_grants(
        &self,
        conversation_id: &ConversationId,
        grants: Vec<(GrantId, GrantScope)>,
    ) -> Result<Vec<ConversationGrant>, ToolsStoreError> {
        let existing = self.grants(conversation_id).await?;
        let conversation = conversation_id.as_str().to_owned();
        let now = self.clock().now();
        let mut new = Vec::new();
        for (id, scope) in grants {
            let known = existing.iter().any(|grant| grant.scope == scope)
                || new
                    .iter()
                    .any(|grant: &ConversationGrant| grant.scope == scope);
            if !known {
                new.push(ConversationGrant {
                    id,
                    scope,
                    created_at: now,
                });
            }
        }
        let documents = new
            .iter()
            .map(|grant| Ok((grant.id.as_str().to_owned(), to_json(&grant.scope)?)))
            .collect::<Result<Vec<_>, ToolsStoreError>>()?;
        self.transact(Durability::Full, move |transaction| {
            let exists = transaction
                .prepare_cached(SELECT_CONVERSATION_EXISTS)?
                .query_row([&conversation], |_row| Ok(()))
                .optional()?
                .is_some();
            if !exists {
                return Err(TransactError::Rejected(
                    ToolsStoreError::ConversationNotFound,
                ));
            }
            for (id, scope) in &documents {
                transaction.prepare_cached(INSERT_GRANT)?.execute(params![
                    id,
                    conversation,
                    DOCUMENT_VERSION,
                    scope,
                    now.as_millis()
                ])?;
            }
            Ok(())
        })
        .await
        .map_err(ToolsStoreError::from)?;
        Ok(new)
    }

    /// Revokes a grant of a conversation. Returns whether it existed.
    pub async fn revoke_grant(
        &self,
        conversation_id: &ConversationId,
        grant_id: &GrantId,
    ) -> Result<bool, ToolsStoreError> {
        let conversation = conversation_id.as_str().to_owned();
        let grant = grant_id.as_str().to_owned();
        let deleted = self
            .write(Durability::Full, move |transaction| {
                transaction
                    .prepare_cached(DELETE_GRANT)?
                    .execute([grant, conversation])
            })
            .await?;
        Ok(deleted > 0)
    }

    /// The grants for a project or for every conversation, the oldest first.
    pub async fn shared_grants(&self) -> Result<Vec<SharedGrant>, ToolsStoreError> {
        let raw: Vec<RawSharedGrant> = self
            .read(|connection| {
                let mut select = connection.prepare_cached(SELECT_SHARED_GRANTS)?;
                let rows = select.query_map([], |row| {
                    Ok(RawSharedGrant {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        version: row.get(2)?,
                        scope: row.get(3)?,
                        created_at: row.get(4)?,
                    })
                })?;
                rows.collect()
            })
            .await?;
        raw.into_iter().map(RawSharedGrant::into_grant).collect()
    }

    /// Adds grants for every conversation of a project, or for every
    /// conversation when `project` is `None`, with full durability, and
    /// returns the grants that were new. A scope that the same project, or
    /// everyone, already has is not added again.
    pub async fn add_shared_grants(
        &self,
        project: Option<&ProjectId>,
        grants: Vec<(GrantId, GrantScope)>,
    ) -> Result<Vec<SharedGrant>, ToolsStoreError> {
        let existing: Vec<SharedGrant> = self
            .shared_grants()
            .await?
            .into_iter()
            .filter(|grant| grant.project_id.as_ref() == project)
            .collect();
        let now = self.clock().now();
        let mut new: Vec<SharedGrant> = Vec::new();
        for (id, scope) in grants {
            let known = existing
                .iter()
                .chain(&new)
                .any(|grant| grant.scope == scope);
            if !known {
                new.push(SharedGrant {
                    id,
                    project_id: project.cloned(),
                    scope,
                    created_at: now,
                });
            }
        }
        let documents = new
            .iter()
            .map(|grant| Ok((grant.id.as_str().to_owned(), to_json(&grant.scope)?)))
            .collect::<Result<Vec<_>, ToolsStoreError>>()?;
        let project = project.map(|id| id.as_str().to_owned());
        self.transact(Durability::Full, move |transaction| {
            if let Some(project) = &project {
                let exists = transaction
                    .prepare_cached(SELECT_PROJECT_EXISTS)?
                    .query_row([project], |_row| Ok(()))
                    .optional()?
                    .is_some();
                if !exists {
                    return Err(TransactError::Rejected(ToolsStoreError::ProjectNotFound));
                }
            }
            for (id, scope) in &documents {
                transaction
                    .prepare_cached(INSERT_SHARED_GRANT)?
                    .execute(params![
                        id,
                        project,
                        DOCUMENT_VERSION,
                        scope,
                        now.as_millis()
                    ])?;
            }
            Ok(())
        })
        .await
        .map_err(ToolsStoreError::from)?;
        Ok(new)
    }

    /// Revokes a grant for a project or for every conversation. Returns
    /// whether it existed.
    pub async fn revoke_shared_grant(&self, grant_id: &GrantId) -> Result<bool, ToolsStoreError> {
        let grant = grant_id.as_str().to_owned();
        let deleted = self
            .write(Durability::Full, move |transaction| {
                transaction
                    .prepare_cached(DELETE_SHARED_GRANT)?
                    .execute([grant])
            })
            .await?;
        Ok(deleted > 0)
    }

    /// Every command rule, the oldest first.
    pub async fn rules(&self) -> Result<Vec<CommandRule>, ToolsStoreError> {
        let raw: Vec<RawRule> = self
            .read(|connection| {
                let mut select = connection.prepare_cached(SELECT_RULES)?;
                let rows = select.query_map([], rule_from_row)?;
                rows.collect()
            })
            .await?;
        raw.into_iter().map(RawRule::into_rule).collect()
    }

    /// Saves a command rule with full durability. When the same rule exists,
    /// returns it instead.
    pub async fn add_rule(
        &self,
        id: &RuleId,
        words: Vec<String>,
        folder: Option<String>,
    ) -> Result<CommandRule, ToolsStoreError> {
        let id = id.as_str().to_owned();
        let words = to_json(&words)?;
        let now = self.clock().now().as_millis();
        let raw = self
            .transact(Durability::Full, move |transaction| {
                let existing = transaction
                    .prepare_cached(SELECT_RULE)?
                    .query_row(params![words, folder], rule_from_row)
                    .optional()?;
                if let Some(existing) = existing {
                    return Ok(existing);
                }
                transaction.prepare_cached(INSERT_RULE)?.execute(params![
                    id,
                    DOCUMENT_VERSION,
                    words,
                    folder,
                    now
                ])?;
                Ok(RawRule {
                    id,
                    version: i64::from(DOCUMENT_VERSION),
                    words,
                    folder,
                    created_at: now,
                })
            })
            .await
            .map_err(|error: TransactError<ToolsStoreError>| ToolsStoreError::from(error))?;
        raw.into_rule()
    }

    /// Deletes a command rule. Returns whether it existed.
    pub async fn delete_rule(&self, rule_id: &RuleId) -> Result<bool, ToolsStoreError> {
        let rule = rule_id.as_str().to_owned();
        let deleted = self
            .write(Durability::Full, move |transaction| {
                transaction.prepare_cached(DELETE_RULE)?.execute([rule])
            })
            .await?;
        Ok(deleted > 0)
    }

    /// Every rule for commands on remote hosts, the oldest first.
    pub async fn remote_rules(&self) -> Result<Vec<RemoteCommandRule>, ToolsStoreError> {
        let raw: Vec<RawRemoteRule> = self
            .read(|connection| {
                let mut select = connection.prepare_cached(SELECT_REMOTE_RULES)?;
                let rows = select.query_map([], remote_rule_from_row)?;
                rows.collect()
            })
            .await?;
        raw.into_iter().map(RawRemoteRule::into_rule).collect()
    }

    /// Saves a rule for commands on one remote host with full durability.
    /// When the same rule exists for the same host, returns it instead.
    pub async fn add_remote_rule(
        &self,
        id: &RuleId,
        target: RemoteTarget,
        words: Vec<String>,
    ) -> Result<RemoteCommandRule, ToolsStoreError> {
        let id = id.as_str().to_owned();
        let words = to_json(&words)?;
        let now = self.clock().now().as_millis();
        let raw = self
            .transact(Durability::Full, move |transaction| {
                let existing = transaction
                    .prepare_cached(SELECT_REMOTE_RULE)?
                    .query_row(
                        params![target.plugin, target.target, target.endpoint, words],
                        remote_rule_from_row,
                    )
                    .optional()?;
                if let Some(existing) = existing {
                    return Ok(existing);
                }
                transaction
                    .prepare_cached(INSERT_REMOTE_RULE)?
                    .execute(params![
                        id,
                        target.plugin,
                        target.target,
                        target.endpoint,
                        target.label,
                        DOCUMENT_VERSION,
                        words,
                        now
                    ])?;
                Ok(RawRemoteRule {
                    id,
                    plugin: target.plugin,
                    target: target.target,
                    endpoint: target.endpoint,
                    label: target.label,
                    version: i64::from(DOCUMENT_VERSION),
                    words,
                    created_at: now,
                })
            })
            .await
            .map_err(|error: TransactError<ToolsStoreError>| ToolsStoreError::from(error))?;
        raw.into_rule()
    }

    /// Deletes a rule for commands on a remote host. Returns whether it
    /// existed.
    pub async fn delete_remote_rule(&self, rule_id: &RuleId) -> Result<bool, ToolsStoreError> {
        let rule = rule_id.as_str().to_owned();
        let deleted = self
            .write(Durability::Full, move |transaction| {
                transaction
                    .prepare_cached(DELETE_REMOTE_RULE)?
                    .execute([rule])
            })
            .await?;
        Ok(deleted > 0)
    }

    /// Every rule for runs of installed programs, the oldest first.
    pub async fn program_rules(&self) -> Result<Vec<ProgramRule>, ToolsStoreError> {
        let raw: Vec<RawProgramRule> = self
            .read(|connection| {
                let mut select = connection.prepare_cached(SELECT_PROGRAM_RULES)?;
                let rows = select.query_map([], program_rule_from_row)?;
                rows.collect()
            })
            .await?;
        raw.into_iter().map(RawProgramRule::into_rule).collect()
    }

    /// Saves a rule for runs of one plugin's installed program with full
    /// durability. When the same rule exists, returns it instead. The keys of
    /// the effects are stored sorted, so the same effects make the same rule.
    pub async fn add_program_rule(
        &self,
        id: &RuleId,
        target: ProgramTarget,
        words: Vec<String>,
        folder: Option<String>,
    ) -> Result<ProgramRule, ToolsStoreError> {
        let id = id.as_str().to_owned();
        let words = to_json(&words)?;
        let mut keys = target.effects;
        keys.sort();
        keys.dedup();
        let effects = to_json(&keys)?;
        let ProgramTarget {
            plugin,
            name,
            program,
            ..
        } = target;
        let now = self.clock().now().as_millis();
        let raw = self
            .transact(Durability::Full, move |transaction| {
                let existing = transaction
                    .prepare_cached(SELECT_PROGRAM_RULE)?
                    .query_row(
                        params![plugin, name, program, words, effects, folder],
                        program_rule_from_row,
                    )
                    .optional()?;
                if let Some(existing) = existing {
                    return Ok(existing);
                }
                transaction
                    .prepare_cached(INSERT_PROGRAM_RULE)?
                    .execute(params![
                        id,
                        plugin,
                        name,
                        program,
                        DOCUMENT_VERSION,
                        words,
                        effects,
                        folder,
                        now
                    ])?;
                Ok(RawProgramRule {
                    id,
                    plugin,
                    name,
                    program,
                    version: i64::from(DOCUMENT_VERSION),
                    words,
                    effects,
                    folder,
                    created_at: now,
                })
            })
            .await
            .map_err(|error: TransactError<ToolsStoreError>| ToolsStoreError::from(error))?;
        raw.into_rule()
    }

    /// Deletes a rule for runs of an installed program. Returns whether it
    /// existed.
    pub async fn delete_program_rule(&self, rule_id: &RuleId) -> Result<bool, ToolsStoreError> {
        let rule = rule_id.as_str().to_owned();
        let deleted = self
            .write(Durability::Full, move |transaction| {
                transaction
                    .prepare_cached(DELETE_PROGRAM_RULE)?
                    .execute([rule])
            })
            .await?;
        Ok(deleted > 0)
    }
}

#[cfg(test)]
mod tests;
