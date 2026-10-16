//! What comes with a task that another machine assigns (ADR-0028, decision
//! 1): the task, its instruction and the provider profile of the reply that
//! assigned it, without the credential. A link carries these documents as
//! JSON, so they change only by addition.

use serde::{Deserialize, Serialize};

use crate::ids::{ConversationId, ProfileId, TaskId};
use crate::provider::{ApiInterface, ReasoningEffort, ServerKind};

/// The provider profile of the reply that delegated a task, without its
/// credential (docs/features/workers/credential-leases.md, the profile of a
/// task). The credential follows as a lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSnapshot {
    /// The master's profile.
    pub id: ProfileId,
    /// Its configuration revision, which binds provider-private data.
    pub config_revision: u32,
    /// The API it speaks.
    pub interface: ApiInterface,
    /// The server's base URL.
    pub base_url: String,
    /// The model.
    pub model: String,
    /// The context window, in tokens.
    pub context_window: u32,
    /// The output limit, in tokens.
    pub output_limit: u32,
    /// The reasoning effort.
    pub reasoning_effort: ReasoningEffort,
    /// The server the provider test recognized, for its quirks.
    #[serde(default)]
    pub server: Option<ServerKind>,
    /// Whether the model gets tools.
    pub use_tools: bool,
    /// Whether the profile has a credential, which the master leases.
    pub has_credential: bool,
}

impl ProfileSnapshot {
    /// The binding of provider-private data: the master's profile and its
    /// revision, as on the master.
    pub fn binding(&self) -> String {
        format!("{}:{}", self.id, self.config_revision)
    }
}

/// A task that the master assigns to a worker. The master chooses the IDs,
/// so assigning the same task again assigns it once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAssignment {
    /// The task.
    pub task_id: TaskId,
    /// The task's conversation, which the worker keeps.
    pub conversation_id: ConversationId,
    /// Whether the task continues an earlier task of the same conversation.
    pub follow_up: bool,
    /// The task's title, for a new conversation.
    pub title: String,
    /// The instruction, which becomes the first message of the worker's
    /// turn.
    pub instruction: String,
    /// Whether the task may only read and search, which narrows the worker's
    /// ceiling for it.
    pub read_only: bool,
    /// The profile that the task's runs use.
    pub profile: ProfileSnapshot,
}

#[cfg(test)]
mod tests {
    use super::{ProfileSnapshot, TaskAssignment};
    use crate::ids::{ConversationId, ProfileId, TaskId};
    use crate::provider::{ApiInterface, ReasoningEffort};

    #[test]
    fn an_assignment_reads_as_the_link_carries_it() {
        let assignment = TaskAssignment {
            task_id: TaskId::parse("0199e0f2-0000-7000-a000-000000000001").unwrap(),
            conversation_id: ConversationId::parse("0199e0f2-0000-7000-a000-000000000002").unwrap(),
            follow_up: false,
            title: "Build the backend".to_owned(),
            instruction: "Build it and run the tests.".to_owned(),
            read_only: true,
            profile: ProfileSnapshot {
                id: ProfileId::parse("0199e0f2-0000-7000-a000-000000000003").unwrap(),
                config_revision: 2,
                interface: ApiInterface::ChatCompletions,
                base_url: "https://api.example.com/v1".to_owned(),
                model: "model-1".to_owned(),
                context_window: 128_000,
                output_limit: 16_384,
                reasoning_effort: ReasoningEffort::ServerDefault,
                server: None,
                use_tools: true,
                has_credential: true,
            },
        };

        let json = serde_json::to_value(&assignment).unwrap();

        assert_eq!(json["taskId"], "0199e0f2-0000-7000-a000-000000000001");
        assert_eq!(json["readOnly"], true);
        assert_eq!(json["profile"]["configRevision"], 2);
        assert_eq!(json["profile"]["hasCredential"], true);
        assert_eq!(
            serde_json::from_value::<TaskAssignment>(json).unwrap(),
            assignment
        );
        assert_eq!(
            assignment.profile.binding(),
            "0199e0f2-0000-7000-a000-000000000003:2"
        );
    }
}
