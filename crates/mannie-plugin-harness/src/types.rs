//! The contract types that plugin tests build and read and that the SDK's
//! contracts leave out, because no plugin's code uses them: the inputs of
//! conversations, the clocks of tests, the IDs of runs, turns, parts and
//! rules, images as the transcript keeps them, the plugins' views, provider
//! profiles, runs, the documents of tasks and tool calls, and the transcript's
//! parts.

pub use mannie_types::chat::{
    SendMessage, StartConversation, StartedConversation, StopOutcome, SubscribeConversation,
};
pub use mannie_types::clock::{ManualClock, SystemClock};
pub use mannie_types::ids::{InvalidId, PartId, RuleId, RunId, TurnId};
pub use mannie_types::images::{ImageRef, ImageSize, ReadImage};
pub use mannie_types::plugins::{PluginState, PluginView};
pub use mannie_types::provider::{
    ApiInterface, CreateProviderProfile, ProfileDraft, ProviderProfile, ReasoningEffort,
};
pub use mannie_types::run::{FinishReason, RunState};
pub use mannie_types::tasks::{TaskDocument, TaskPartDocument, TaskRefusal, TaskView};
pub use mannie_types::tools::{
    CommandRule, Decision, ToolCallDocument, ToolCallStatus, ToolOutcome, UpdateToolsSettings,
};
pub use mannie_types::transcript::{PartContent, PartKind};
