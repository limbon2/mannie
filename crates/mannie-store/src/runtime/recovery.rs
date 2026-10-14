//! Recovery at startup: runs that were active when the process ended become
//! interrupted, and their streaming parts keep the text stored so far
//! (docs/architecture/agent-runtime.md, stop, retry and recovery).

use mannie_types::ids::{ConversationId, RunId};
use mannie_types::run::{RunOutcome, RunState};

use super::rows::parse_id;
use super::{Change, RuntimeStoreError};
use crate::{Durability, Store};

const SELECT_ACTIVE_RUNS: &str = "SELECT id, conversation_id FROM runtime_runs \
     WHERE state IN ('queued', 'running') ORDER BY created_at";

/// Every statement of this module.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[SELECT_ACTIVE_RUNS];

/// A run that recovery marked as interrupted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovered {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The run.
    pub run_id: RunId,
}

impl Store {
    /// Marks every active run as interrupted, with an event for each change.
    /// The runtime calls it once at startup, before it accepts commands, so no
    /// actor runs at the same time.
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
            let change = Change::EndRun {
                run_id: run_id.clone(),
                state: RunState::Interrupted,
                outcome: RunOutcome::default(),
            };
            self.apply(&conversation_id, vec![change], Durability::Full)
                .await?;
            tracing::info!(run = %run_id, conversation = %conversation_id, "marked a run as interrupted after a restart");
            recovered.push(Recovered {
                conversation_id,
                run_id,
            });
        }
        Ok(recovered)
    }
}
