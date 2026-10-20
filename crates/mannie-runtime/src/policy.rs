//! What the policy reads besides an operation's accesses (docs/features/
//! system-tools/operations-and-approvals.md, policy): the grants that apply to
//! a conversation, the command rules, the rules of remote hosts, the rules of
//! installed programs and the tools settings. Tool calls and mentions read
//! them the same way, so a mention is authorized as the model's read would be
//! (ADR-0004, decision 2; ADR-0032, decision 8).

use std::fmt::Display;

use mannie_types::ids::{ConversationId, ProjectId};
use mannie_types::tools::{CommandRule, GrantScope, ProgramRule, RemoteCommandRule, ToolsSettings};

use crate::Runtime;

/// The grants, rules and settings that the policy decides with.
#[derive(Debug, Clone, Default)]
pub(crate) struct PolicyGrants {
    /// The tools settings.
    pub(crate) settings: ToolsSettings,
    /// The conversation's own grants.
    pub(crate) conversation: Vec<GrantScope>,
    /// The grants for every conversation of its project.
    pub(crate) project: Vec<GrantScope>,
    /// The grants for every conversation.
    pub(crate) always: Vec<GrantScope>,
    /// The command rules.
    pub(crate) rules: Vec<CommandRule>,
    /// The rules for commands on remote hosts (ADR-0036).
    pub(crate) remote_rules: Vec<RemoteCommandRule>,
    /// The rules for runs of installed programs (ADR-0038).
    pub(crate) program_rules: Vec<ProgramRule>,
}

/// What could not be read, with the error, for the log.
#[derive(Debug)]
pub(crate) struct PolicyReadError {
    /// What could not be read, such as "the rules".
    pub(crate) what: &'static str,
    /// The error.
    pub(crate) error: String,
}

impl Runtime {
    /// Reads the grants, rules and settings for a conversation in `project`.
    /// `conversation` is `None` for a message that starts a conversation,
    /// which has no grants of its own yet.
    pub(crate) async fn policy_grants(
        &self,
        conversation: Option<&ConversationId>,
        project: Option<&ProjectId>,
    ) -> Result<PolicyGrants, PolicyReadError> {
        let store = &self.inner.store;
        let mut grants = PolicyGrants::default();
        let shared = store
            .shared_grants()
            .await
            .map_err(|error| read_error("the shared grants", &error))?;
        for grant in shared {
            match &grant.project_id {
                None => grants.always.push(grant.scope),
                Some(owner) if Some(owner) == project => grants.project.push(grant.scope),
                Some(_other) => {}
            }
        }
        grants.settings = store
            .load_settings::<ToolsSettings>()
            .await
            .map_err(|error| read_error("the tools settings", &error))?;
        if let Some(conversation) = conversation {
            grants.conversation = store
                .grants(conversation)
                .await
                .map_err(|error| read_error("the grants", &error))?
                .into_iter()
                .map(|grant| grant.scope)
                .collect();
        }
        grants.rules = store
            .rules()
            .await
            .map_err(|error| read_error("the rules", &error))?;
        grants.remote_rules = store
            .remote_rules()
            .await
            .map_err(|error| read_error("the rules of remote hosts", &error))?;
        grants.program_rules = store
            .program_rules()
            .await
            .map_err(|error| read_error("the rules of installed programs", &error))?;
        Ok(grants)
    }
}

fn read_error(what: &'static str, error: &dyn Display) -> PolicyReadError {
    PolicyReadError {
        what,
        error: error.to_string(),
    }
}
