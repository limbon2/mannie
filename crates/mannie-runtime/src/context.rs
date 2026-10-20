//! Assembling the model's context from the transcript (docs/architecture/
//! agent-runtime.md, context management). Compaction is a projection: a
//! checkpoint replaces the turns it covers here, and the stored transcript
//! never changes (ADR-0012, decision 8).

use std::collections::HashSet;

use mannie_provider::{
    Content, ImageInput, Message, ModelRequest, Role, ToolChoice, ToolDefinition,
};
use mannie_store::{ModelPart, ModelTurn};
use mannie_types::ids::{PartId, RunId};
use mannie_types::provider::{ApiInterface, ReasoningEffort};
use mannie_types::run::RunState;
use mannie_types::tools::{Interruption, NotRunReason, Refusal, ToolCallDocument, ToolCallStatus};
use mannie_types::transcript::PartKind;

use crate::agents::Requester;
use crate::{tasks, texts};

/// The most characters of a call's arguments that a model without tools
/// reads in the text that stands in for the call.
const MAX_TEXT_CALL_ARGUMENTS: usize = 2_000;

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
    /// The tools the model may call. Without tools, earlier tool calls reach
    /// the model as text.
    pub(crate) tools: &'a [ToolDefinition],
    /// Whether the model may call them in this call.
    pub(crate) tool_choice: ToolChoice,
    /// A note after the turns, such as the one that the reply used up its
    /// tool calls.
    pub(crate) note: Option<&'a str>,
    /// Tool calls whose results are left out to save space.
    pub(crate) left_out: &'a HashSet<PartId>,
    /// Whom the agent answers to, which decides how results address it.
    pub(crate) requester: Requester,
}

/// Builds the request for `turns`, the turns after the checkpoint, oldest
/// first.
pub(crate) fn build_request(shape: RequestShape<'_>, turns: &[ModelTurn]) -> ModelRequest {
    let mut messages = Vec::new();
    for turn in turns {
        append_turn(&mut messages, turn, shape);
    }
    if let Some(note) = shape.note {
        messages.push(Message::text(Role::User, note));
    }
    ModelRequest {
        model: shape.model.to_owned(),
        instructions: shape.instructions.to_owned(),
        messages,
        max_output_tokens: shape.max_output_tokens,
        reasoning_effort: shape.reasoning_effort,
        tools: shape.tools.to_vec(),
        tool_choice: shape.tool_choice,
    }
}

/// The reply that a turn's parts build: the assistant's content, and the
/// results of its tool calls, which follow the assistant message that holds
/// the calls.
#[derive(Default)]
struct Reply {
    content: Vec<Content>,
    results: Vec<Message>,
}

impl Reply {
    /// Adds the reply so far to the messages: the assistant message, then the
    /// results of its tool calls.
    fn flush(&mut self, messages: &mut Vec<Message>) {
        flush_reply(messages, &mut self.content);
        messages.append(&mut self.results);
    }

    /// Whether the reply wrote anything that the model reads.
    fn wrote_something(&self) -> bool {
        !self.results.is_empty()
            || self
                .content
                .iter()
                .any(|content| matches!(content, Content::Text(_) | Content::ToolCall { .. }))
    }
}

fn append_turn(messages: &mut Vec<Message>, turn: &ModelTurn, shape: RequestShape<'_>) {
    let mut reply = Reply::default();
    // The parts of the runs that the turn keeps come first, run by run; each
    // ends with the marker of how it ended.
    let mut current_run: Option<&RunId> = None;
    // The model call that made the latest tool call, so that calls of the
    // next step get an assistant message of their own after the results.
    let mut calling_attempt: Option<u32> = None;
    for part in &turn.parts {
        if let Some(run) = &part.run_id
            && current_run != Some(run)
        {
            if let Some(previous) = current_run
                && let Some(kept) = turn.kept_runs.iter().find(|kept| kept.run_id == *previous)
            {
                end_run(messages, &mut reply, Some(kept.state));
            }
            current_run = Some(run);
            calling_attempt = None;
        }
        let is_call = part.kind.is_some_and(PartKind::is_call);
        let next_step = calling_attempt.is_some_and(|attempt| attempt != part.attempt);
        let follows_results = !reply.results.is_empty();
        if follows_results && (!is_call || next_step) {
            // The next model call of the reply starts after the results.
            reply.flush(messages);
        }
        if is_call {
            calling_attempt = Some(part.attempt);
        }
        match part.kind {
            // A message with images may have no text (ADR-0033, decision 1),
            // and no API takes an empty text part.
            Some(PartKind::UserText) => {
                let content = if part.text.trim().is_empty() {
                    Vec::new()
                } else {
                    vec![Content::Text(part.text.clone())]
                };
                messages.push(Message {
                    role: Role::User,
                    content,
                });
            }
            // What the owner's message mentions follows it, in the same
            // message (ADR-0032, decision 7).
            Some(PartKind::Mention) => match messages.last_mut() {
                Some(owner) if owner.role == Role::User => {
                    owner.content.push(Content::Text(part.text.clone()));
                }
                _ => messages.push(Message::text(Role::User, part.text.clone())),
            },
            // The images follow the text and the mentions, in the same
            // message, and the model never reads their names (ADR-0033,
            // decision 6).
            Some(PartKind::Image) => {
                let content = match &part.image {
                    Some(image) => Content::Image(ImageInput {
                        format: image.format,
                        width: image.width,
                        height: image.height,
                        data: image.data.clone(),
                    }),
                    None => Content::Text(texts::IMAGE_MISSING.to_owned()),
                };
                match messages.last_mut() {
                    Some(owner) if owner.role == Role::User => owner.content.push(content),
                    _ => messages.push(Message {
                        role: Role::User,
                        content: vec![content],
                    }),
                }
            }
            Some(PartKind::Text) => {
                if !part.text.is_empty() {
                    reply.content.push(Content::Text(part.text.clone()));
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
                    reply.content.push(Content::ProviderItem(item.item.clone()));
                }
            }
            Some(PartKind::ToolCall | PartKind::Task) => append_call(&mut reply, part, shape),
            Some(PartKind::ContinuationNote) => {
                reply.flush(messages);
                messages.push(Message::text(Role::User, part.text.clone()));
            }
            // Errors are for the owner and never reach the model's turns.
            Some(PartKind::Error) => {}
            None => reply
                .content
                .push(Content::Text(texts::unknown_part(&part.kind_name))),
        }
    }
    // A kept run may end the turn, before the current run wrote anything.
    if let Some(previous) = current_run
        && let Some(kept) = turn.kept_runs.iter().find(|kept| kept.run_id == *previous)
    {
        end_run(messages, &mut reply, Some(kept.state));
    }
    end_run(messages, &mut reply, turn.run_state);
}

/// Ends the parts of one run: its reply, and a marker when it was stopped or
/// interrupted. A marker without tool results stays in the reply's own
/// message, as before tools existed.
fn end_run(messages: &mut Vec<Message>, reply: &mut Reply, state: Option<RunState>) {
    let marker = match state {
        Some(RunState::Stopped) => Some(texts::STOPPED_MARKER),
        Some(RunState::Interrupted | RunState::Failed) => {
            reply.wrote_something().then_some(texts::INTERRUPTED_MARKER)
        }
        _ => None,
    };
    if reply.results.is_empty() {
        if let Some(marker) = marker {
            reply.content.push(Content::Text(marker.to_owned()));
        }
        reply.flush(messages);
    } else {
        reply.flush(messages);
        if let Some(marker) = marker {
            messages.push(Message::text(Role::Assistant, marker));
        }
    }
}

/// Adds a call of a tool or of a task tool: as a call with its result for a
/// model with tools, as text for a model without them.
fn append_call(reply: &mut Reply, part: &ModelPart, shape: RequestShape<'_>) {
    let Some(call) = &part.tool_call else {
        return;
    };
    let result = part_result(part, call, shape.left_out, shape.requester);
    if shape.tools.is_empty() {
        let arguments: String = part.text.chars().take(MAX_TEXT_CALL_ARGUMENTS).collect();
        reply.content.push(Content::Text(texts::tool_call_as_text(
            &call.tool, &arguments, &result,
        )));
        return;
    }
    reply.content.push(Content::ToolCall {
        call_id: call.call_id.clone(),
        name: call.tool.clone(),
        arguments: json_object_or_empty(&part.text),
    });
    reply.results.push(Message {
        role: Role::Tool,
        content: vec![Content::ToolResult {
            call_id: call.call_id.clone(),
            output: result,
        }],
    });
}

/// The arguments as they go back to the provider: a JSON object, or `{}` for
/// arguments that are not one, which providers would refuse.
fn json_object_or_empty(arguments: &str) -> String {
    match serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(arguments) {
        Ok(_) => arguments.to_owned(),
        Err(_) => "{}".to_owned(),
    }
}

/// What the model reads as the result of a call part: a tool call's result,
/// or for a task part its result, its refusal or its task's end
/// (docs/features/child-agents/tasks.md, the task part).
pub(crate) fn part_result(
    part: &ModelPart,
    call: &ToolCallDocument,
    left_out: &HashSet<PartId>,
    requester: Requester,
) -> String {
    let section = part
        .task
        .as_ref()
        .filter(|_section| part.kind == Some(PartKind::Task));
    let Some(section) = section else {
        return call_result(&part.id, call, left_out, requester);
    };
    if left_out.contains(&part.id) && tasks::returned_reports(section) {
        return texts::report_left_out(&tasks::reported_numbers(section));
    }
    tasks::task_result(section).unwrap_or_else(|| call_result(&part.id, call, left_out, requester))
}

/// What the model reads as a tool call's result.
pub(crate) fn call_result(
    part: &PartId,
    call: &ToolCallDocument,
    left_out: &HashSet<PartId>,
    requester: Requester,
) -> String {
    match call.status {
        ToolCallStatus::Completed => match &call.result {
            Some(result) if left_out.contains(part) => {
                let archives: Vec<String> = result
                    .archives
                    .iter()
                    .map(|archive| archive.id.to_string())
                    .collect();
                texts::result_left_out(&archives)
            }
            Some(result) => result.output.clone(),
            None => texts::CANCELED.to_owned(),
        },
        ToolCallStatus::Denied => texts::denied(requester).to_owned(),
        ToolCallStatus::Uncertain => match &call.interruption {
            None => texts::UNCERTAIN.to_owned(),
            Some(Interruption::Stopped) => texts::UNCERTAIN_STOPPED.to_owned(),
            Some(Interruption::PluginOff { plugin }) => texts::uncertain_plugin_off(plugin),
        },
        ToolCallStatus::NotRun | ToolCallStatus::Canceled => match &call.not_run {
            Some(reason) => not_run(reason, &call.tool),
            None => texts::CANCELED.to_owned(),
        },
        ToolCallStatus::Receiving
        | ToolCallStatus::Pending
        | ToolCallStatus::AwaitingApproval
        | ToolCallStatus::Executing
        | ToolCallStatus::Running => texts::CANCELED.to_owned(),
    }
}

/// Why a call never started, for the model.
fn not_run(reason: &NotRunReason, tool: &str) -> String {
    match reason {
        NotRunReason::UnknownTool => texts::unknown_tool(tool),
        NotRunReason::InvalidArguments { detail } => texts::invalid_arguments(detail),
        NotRunReason::CutOff => texts::CUT_OFF.to_owned(),
        NotRunReason::EarlierCallDenied => texts::SKIPPED_AFTER_DENIAL.to_owned(),
        NotRunReason::OverBudget => texts::OVER_BUDGET.to_owned(),
        NotRunReason::ToolsOff => texts::TOOLS_OFF.to_owned(),
        NotRunReason::Refused { refusal } => match refusal {
            Refusal::ReadOnlyHost => texts::READ_ONLY_HOST.to_owned(),
            Refusal::NoProjectsOnHost => texts::NO_PROJECTS_ON_HOST.to_owned(),
            Refusal::NoRemoteCommands => texts::NO_REMOTE_COMMANDS.to_owned(),
            Refusal::NoProgramsOnHost => texts::NO_PROGRAMS_ON_HOST.to_owned(),
        },
        NotRunReason::PluginOff { plugin } => texts::tool_plugin_off(plugin),
    }
}

/// Adds the reply so far as an assistant message. A reply needs text or a
/// tool call: a reasoning item with nothing after it would lead nowhere, so
/// it is left out.
fn flush_reply(messages: &mut Vec<Message>, reply: &mut Vec<Content>) {
    let leads_somewhere = reply
        .iter()
        .any(|content| matches!(content, Content::Text(_) | Content::ToolCall { .. }));
    if leads_somewhere {
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
