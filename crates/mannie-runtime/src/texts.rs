//! The text the model sees (docs/features/chat/conversations.md, the text the
//! model sees). It is a contract with the model and is reviewed like code:
//! changing it changes how every model behaves in Mannie.

/// The first instruction section of every agent.
pub const BASE_INSTRUCTIONS: &str = "You are the assistant in Mannie, a desktop app, talking with its owner. \
Use Markdown when it helps; Mannie renders it, but does not render raw HTML or load images.";

/// Introduces the owner's global instructions.
pub const OWNER_INSTRUCTIONS_HEADING: &str =
    "Instructions from the owner, which apply to every conversation:";

/// Introduces the checkpoint that stands in for older turns.
pub const CHECKPOINT_HEADING: &str =
    "Summary of the earlier part of this conversation, written when the context ran low:";

/// Asks the model to continue after a step without visible text.
pub const CONTINUATION_NOTE: &str =
    "Your last response was empty. Continue with your answer to the owner's latest message.";

/// Marks a reply that the owner stopped.
pub const STOPPED_MARKER: &str = "[The owner stopped this reply.]";

/// Marks a reply that ended with the process.
pub const INTERRUPTED_MARKER: &str = "[This reply was interrupted before it finished.]";

/// Stands in for a part of a kind that this version cannot show the model.
pub fn unknown_part(kind: &str) -> String {
    format!("[Content of type {kind} that this version of Mannie cannot show.]")
}

/// The instructions of the call that writes a checkpoint.
pub const COMPACTION_INSTRUCTIONS: &str = "You are writing a checkpoint of a conversation between the owner and an assistant, \
so that the assistant can continue it with less context. Keep the owner's goals, requests and preferences, \
decisions and their reasons, facts, numbers, names, file paths and code that are still relevant, open questions, \
and what the assistant said it would do next. Leave out pleasantries and repetition. Write in the language of the \
conversation, as plain text. Do not add anything the conversation does not say.";

/// Introduces the earlier checkpoint in a compaction request.
pub const COMPACTION_PREVIOUS_SUMMARY: &str = "The summary of what came before:";

/// Introduces the turns to summarize in a compaction request.
pub const COMPACTION_TRANSCRIPT: &str = "The conversation to summarize:";

/// The owner's lines in the transcript of a compaction request.
pub const COMPACTION_OWNER: &str = "Owner:";

/// The assistant's lines in the transcript of a compaction request.
pub const COMPACTION_ASSISTANT: &str = "Assistant:";

/// The request that ends a compaction call.
pub const COMPACTION_REQUEST: &str = "Write the checkpoint now.";

/// Stands in for the middle of a turn that is too long for one compaction
/// call.
pub const COMPACTION_OMITTED: &str =
    "[Part of this turn was left out here because it was too long to summarize.]";
