//! Recovery at startup: runs that were active when the process ended become
//! interrupted, and their streaming parts keep the text stored so far
//! (docs/architecture/agent-runtime.md, stop, retry and recovery). Tool calls
//! that were open get the status that says what may have happened before
//! their run ends (docs/features/system-tools/operations-and-approvals.md,
//! recovery).

use mannie_types::ids::{ConversationId, PartId, RunId};
use mannie_types::run::{RunOutcome, RunState};
use mannie_types::tools::{ProcessRecord, ToolCallStatus};
use mannie_types::transcript::PartKind;

use super::rows::{PartDocument, from_json, parse_id};
use super::{Change, RuntimeStoreError};
use crate::{Durability, Store};

const SELECT_ACTIVE_RUNS: &str = "SELECT id, conversation_id FROM runtime_runs \
     WHERE state IN ('queued', 'running') ORDER BY created_at";
const SELECT_OPEN_TOOL_CALLS: &str = "SELECT id, document FROM runtime_parts \
     WHERE run_id = ?1 AND kind = ?2 AND state = 'streaming' ORDER BY position";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[SELECT_ACTIVE_RUNS, SELECT_OPEN_TOOL_CALLS];

/// A tool call whose operation was executing when the process ended, so that
/// it may have taken effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UncertainCall {
    /// The tool call's part.
    pub part_id: PartId,
    /// The tool's name.
    pub tool: String,
    /// The process of a command, which may still run.
    pub process: Option<ProcessRecord>,
}

/// A run that recovery marked as interrupted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovered {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The run.
    pub run_id: RunId,
    /// Its tool calls that became uncertain.
    pub uncertain: Vec<UncertainCall>,
}

impl Store {
    /// Marks every active run as interrupted, with an event for each change.
    /// Its open tool calls become uncertain when they were executing and
    /// canceled otherwise, in the same transaction. The runtime calls this once
    /// at startup, before it accepts commands, so no actor runs at the same
    /// time.
    pub async fn recover_interrupted_runs(&self) -> Result<Vec<Recovered>, RuntimeStoreError> {
        let active: Vec<(String, String)> = self
            .read(|connection| {
                let mut select = connection.prepare(SELECT_ACTIVE_RUNS)?;
                let rows = select.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
                rows.collect()
            })
            .await?;
        let mut recovered = Vec::with_capacity(active.len());
        for (run, conversation) in active {
            let run_id = parse_id(&run, RunId::parse)?;
            let conversation_id = parse_id(&conversation, ConversationId::parse)?;
            let uncertain = self.executing_tool_calls(&run).await?;
            let change = Change::EndRun {
                run_id: run_id.clone(),
                state: RunState::Interrupted,
                outcome: RunOutcome::default(),
            };
            self.apply(&conversation_id, vec![change], Durability::Full)
                .await?;
            tracing::info!(run = %run_id, conversation = %conversation_id, uncertain = uncertain.len(), "marked a run as interrupted after a restart");
            recovered.push(Recovered {
                conversation_id,
                run_id,
                uncertain,
            });
        }
        Ok(recovered)
    }

    /// The tool calls of a run that were executing.
    async fn executing_tool_calls(
        &self,
        run: &str,
    ) -> Result<Vec<UncertainCall>, RuntimeStoreError> {
        let run = run.to_owned();
        let raw: Vec<(String, String)> = self
            .read(move |connection| {
                let mut select = connection.prepare(SELECT_OPEN_TOOL_CALLS)?;
                let rows = select
                    .query_map([run.as_str(), PartKind::ToolCall.as_str()], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })?;
                rows.collect()
            })
            .await?;
        let mut uncertain = Vec::new();
        for (part, document) in raw {
            let document: PartDocument = from_json(&document, "part document")?;
            let Some(call) = document.tool_call else {
                continue;
            };
            if call.status == ToolCallStatus::Executing {
                uncertain.push(UncertainCall {
                    part_id: parse_id(&part, PartId::parse)?,
                    tool: call.tool,
                    process: call.process,
                });
            }
        }
        Ok(uncertain)
    }
}
