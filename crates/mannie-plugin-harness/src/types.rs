//! The contract types that plugin tests build and read and that the SDK's
//! contracts leave out, because no plugin's code uses them: the inputs of
//! conversations, the clocks of tests, the IDs of runs, turns, parts and
//! rules, images as the transcript keeps them, the plugins' views, provider
//! profiles, runs, the documents of tasks and tool calls, the owner's
//! approvals with what they grant, the rules and full access of runs of
//! installed programs, the archives of tool calls, the transcript's parts,
//! and the suggestions and references of mentions.

pub use mannie_types::chat::{
    SendMessage, StartConversation, StartedConversation, StopOutcome, SubscribeConversation,
};
pub use mannie_types::clock::{ManualClock, SystemClock};
pub use mannie_types::ids::{InvalidId, PartId, RuleId, RunId, TurnId};
pub use mannie_types::images::{ImageRef, ImageSize, ReadImage};
pub use mannie_types::mentions::{MentionPlace, MentionRef, MentionSuggestions, SuggestMentions};
pub use mannie_types::plugins::{PluginState, PluginView};
pub use mannie_types::provider::{
    ApiInterface, CreateProviderProfile, ProfileDraft, ProviderProfile, ReasoningEffort,
};
pub use mannie_types::run::{FinishReason, RunState};
pub use mannie_types::tasks::{TaskDocument, TaskPartDocument, TaskRefusal, TaskView};
pub use mannie_types::tools::{
    Access, ApprovalChoice, ApprovalReason, ApprovalRequest, BackgroundCommandView,
    BackgroundEnding, BackgroundSource, CommandRule, DecideApproval, Decision, GrantScope,
    Interruption, ProgramEffectKey, ProgramRule, ProgramTarget, ReadToolOutput, RemoteCommandRule,
    RemoteTarget, SetFullAccess, StopBackgroundCommand, ToolCallDocument, ToolCallStatus,
    ToolCallView, ToolOutcome, UpdateToolsSettings,
};
pub use mannie_types::transcript::{PartContent, PartKind};
