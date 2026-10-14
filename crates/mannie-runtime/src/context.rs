//! Assembling the model's context from the transcript (docs/architecture/
//! agent-runtime.md, context management). Compaction is a projection: a
//! checkpoint replaces the turns it covers here, and the stored transcript
//! never changes (ADR-0012, decision 8).

use mannie_provider::{Content, Message, ModelRequest, Role};
use mannie_store::ModelTurn;
use mannie_types::provider::{ApiInterface, ReasoningEffort};
use mannie_types::run::RunState;
use mannie_types::transcript::PartKind;

use crate::texts;

/// What a request is built from, besides the turns.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RequestShape<'a> {
    /// The assembled system prompt.
    pub(crate) instructions: &'a str,
    /// The model.
    pub(crate) model: &'a str,
    /// The API the call goes to: only the Responses API takes reasoning items.
    pub(crate) interface: ApiInterface,
    /// The binding of the call, which reasoning items must match.
    pub(crate) binding: &'a str,
    /// The output limit.
    pub(crate) max_output_tokens: u32,
    /// The reasoning effort.
    pub(crate) reasoning_effort: ReasoningEffort,
}

/// Builds the request for `turns`, the turns after the checkpoint, oldest
/// first.
pub(crate) fn build_request(shape: RequestShape<'_>, turns: &[ModelTurn]) -> ModelRequest {
    let mut messages = Vec::new();
    for turn in turns {
        append_turn(&mut messages, turn, shape);
    }
    ModelRequest {
        model: shape.model.to_owned(),
        instructions: shape.instructions.to_owned(),
        messages,
        max_output_tokens: shape.max_output_tokens,
        reasoning_effort: shape.reasoning_effort,
    }
}

fn append_turn(messages: &mut Vec<Message>, turn: &ModelTurn, shape: RequestShape<'_>) {
    let mut reply: Vec<Content> = Vec::new();
    for part in &turn.parts {
        match part.kind {
            Some(PartKind::UserText) => messages.push(Message::text(Role::User, part.text.clone())),
            Some(PartKind::Text) => {
                if !part.text.is_empty() {
                    reply.push(Content::Text(part.text.clone()));
                }
            }
            Some(PartKind::Reasoning) => {
                // Encrypted reasoning goes back only to the binding that
                // produced it (ADR-0011, decision 6), and only the Responses
                // API takes it. The reasoning text is for the owner.
                let item = part
                    .provider_item
                    .as_ref()
                    .filter(|item| item.binding == shape.binding);
                if shape.interface == ApiInterface::Responses
                    && let Some(item) = item
                {
                    reply.push(Content::ProviderItem(item.item.clone()));
                }
            }
            Some(PartKind::ContinuationNote) => {
                flush_reply(messages, &mut reply);
                messages.push(Message::text(Role::User, part.text.clone()));
            }
            // Errors are for the owner and never reach the model's turns.
            Some(PartKind::Error) => {}
            None => reply.push(Content::Text(texts::unknown_part(&part.kind_name))),
        }
    }
    let marker = match turn.run_state {
        Some(RunState::Stopped) => Some(texts::STOPPED_MARKER),
        Some(RunState::Interrupted | RunState::Failed) => {
            let wrote_something = reply
                .iter()
                .any(|content| matches!(content, Content::Text(_)));
            wrote_something.then_some(texts::INTERRUPTED_MARKER)
        }
        _ => None,
    };
    if let Some(marker) = marker {
        reply.push(Content::Text(marker.to_owned()));
    }
    flush_reply(messages, &mut reply);
}

/// Adds the reply so far as an assistant message. A reasoning item without
/// text after it would have nothing to lead to, so it is left out.
fn flush_reply(messages: &mut Vec<Message>, reply: &mut Vec<Content>) {
    let has_text = reply
        .iter()
        .any(|content| matches!(content, Content::Text(_)));
    if has_text {
        messages.push(Message {
            role: Role::Assistant,
            content: std::mem::take(reply),
        });
    } else {
        reply.clear();
    }
}

#[cfg(test)]
mod tests;
