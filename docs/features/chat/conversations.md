# Conversations

|              |                                                                                                                                                                                                                                                                                                     |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                            |
| Milestone    | M1                                                                                                                                                                                                                                                                                                  |
| Last updated | 2026-10-23                                                                                                                                                                                                                                                                                          |
| Part of      | [Chat](README.md)                                                                                                                                                                                                                                                                                   |
| Related      | [ADR-0012](../../adr/0012-run-agents-as-durable-actors.md), [ADR-0009](../../adr/0009-persist-state-in-sqlite.md), [Agent runtime](../../architecture/agent-runtime.md), [Domain model](../../architecture/domain-model.md), [Desktop app](../../architecture/desktop-app.md#3-streaming-to-the-ui) |

## Design

### Runs

- `mannie-runtime` runs each reply as an actor: a tokio task with a mailbox for Stop and Shutdown ([ADR-0012](../../adr/0012-run-agents-as-durable-actors.md)). The run continues whether or not a window follows it.
- A conversation has at most one active run, which the store enforces. At most eight model calls run at once across all conversations, and runs over the limit wait in order with the status "waiting for a free slot".
- The conversation agent is the built-in agent definition of M1: its instructions, no tools, at most two continuation notes per turn, and the conversation's provider profile. [System tools](../system-tools/tool-calls.md#the-loop) give it its tools and their budgets.
- Each run is a sequence of steps, one model call each. A step that ends without any visible text gets a continuation note and one more call, up to the limit. A step that ends at the output limit completes the reply and marks it as cut at the output limit.

| State       | Entered when                                                                                                                                                                 |
| ----------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Queued      | The owner sent a message or retried a reply                                                                                                                                  |
| Running     | The run holds a model slot: preparing the context, compacting, streaming or waiting to retry. With system tools, it also runs tools or waits for an approval, without a slot |
| Completed   | The model finished the reply                                                                                                                                                 |
| Failed      | A permanent error, a second context overflow or the last automatic retry failed                                                                                              |
| Stopped     | The owner stopped the reply                                                                                                                                                  |
| Interrupted | Mannie quit or ended while the run was active                                                                                                                                |

### Stop, retry and recovery

- Stop reaches the run through its mailbox, cancels the model call, keeps the streamed text marked as stopped, and ends the run within one second. Stopping a reply that already ended changes nothing.
- A transient failure discards the text of the failed attempt, shows the retry with its cause and the time of the next attempt, and tries again: up to six attempts, after a randomized delay between one second and one minute, or after `Retry-After` when the provider sends one. A `Retry-After` of more than ten minutes ends the reply with the time of the next possible attempt.
- A context overflow forces one compaction and one more attempt. A second overflow fails the reply.
- Retry starts a new run for the latest turn when its reply failed, was stopped or was interrupted. The new attempt replaces the old one on screen and in the model's context; the old parts stay in the database. A reply that made tool calls is kept instead, and the new attempt continues after it ([tool calls](../system-tools/tool-calls.md#retry)).
- At startup, before any command runs, runs that were active become interrupted, and their streamed text is kept up to the last stored batch.
- Quitting stops every active run as interrupted, waits up to three seconds for them, and then closes the store.

### Transcript and events

- The transcript is a list of parts per turn: the owner's text, its mentions and images, the reply's text, reasoning, errors, continuation notes and checkpoint markers. Parts are appended. A part's text grows while it streams, a tool call's document changes while Mannie handles it, and both are final once the part is finished.
- Each part has an audience: the owner and the model, only the model (continuation notes), or only the owner (errors).
- Every change is an event with the conversation's next sequence number, stored before it is published. Streamed text is stored in batches every 50 ms with normal durability; every other change is stored with full durability ([ADR-0009](../../adr/0009-persist-state-in-sqlite.md)).
- When a part finishes, its full text is stored with the part and its streamed deltas are dropped from the event log. The finished event carries the whole text, so replaying the log from any cursor still reproduces the conversation ([ADR-0012](../../adr/0012-run-agents-as-durable-actors.md), decision 9).
- A window subscribes to a conversation with the sequence number of the last event it applied. It receives every stored event after that number and then the live events, without gaps or repeats. Without a number, or when the number is far behind, it receives a snapshot of the latest 50 turns (at most 400 parts) and then the events after the snapshot. Earlier turns load page by page as the owner scrolls up.
- A window that falls more than 1,024 events behind is told to subscribe again from its last number. Reloading the window cancels its subscriptions; the runs continue.
- The conversation list has a subscription of its own: the whole list first, then each conversation that is created, changed or deleted.

### Context budget and compaction

- The context budget of a call is the profile's context window minus its output limit and a margin of 5% of the window.
- Before a conversation's first reply with a profile, Mannie estimates the size of the context: two tokens for every five bytes of text, plus four per message. After a reply, the token counts the provider reported are the base, and only the text added since is estimated, scaled by how far the last estimate was off. The meter in the conversation view marks estimates.
- Before each call, when the projected context passes 80% of the budget, Mannie compacts: it asks the same model, without tools, for a summary of the oldest complete turns that are not yet covered, keeping the newest two turns when it can, then one, then none, until the context fits in 30% of the budget. It skips an automatic compaction that would free less than 10% of the budget. The latest request is never compacted; when it alone does not fit, the reply fails with the cause.
- A summary that ends at the output limit lost its end, where the latest of what it covers and the next steps go, and a model that reasons may have spent the whole limit before it wrote anything. Mannie keeps no checkpoint from it: the compaction fails with `summary_cut`, which asks the owner to raise the profile's output limit.
- The summary is a checkpoint: it records the last turn it covers, a hash of the covered text and the token counts. In the model's context, the checkpoint replaces the turns it covers; the stored transcript does not change. A checkpoint whose covered text no longer matches its hash is ignored, and the previous checkpoint is used.
- The owner can open the latest checkpoint and start a compaction by hand.
- Encrypted reasoning from the Responses API goes back to the model only for the same profile and configuration revision. Reasoning from Chat Completions servers is shown to the owner and never sent back.

### The owner's instructions

- A settings section with the key `chat` holds the owner's instructions: plain text of at most 32,768 characters, applied to every conversation from its next reply. Until 2026-10-16 the limit was 32 KiB of UTF-8, so every text saved under the old limit stays within the new one.

### The text the model sees

The system prompt is assembled from instruction sections in a fixed order: the base instructions, the agent's instructions, the owner's instructions, and then the checkpoint, which changes more often than the sections before it. [System tools](../system-tools/tool-calls.md#the-text-the-model-sees) add the tool guidance after the agent's instructions and the environment after the owner's. What the model reads of a mention follows the owner's text ([mentions](mentions.md)). This text is a contract with the model and is reviewed like code. It lives in `crates/mannie-runtime/src/texts.rs`:

| Text                     | Wording                                                                                                                                                             |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Base instructions        | You are the assistant in Mannie, a desktop app, talking with its owner. Use Markdown when it helps; Mannie renders it, but does not render raw HTML or load images. |
| The owner's instructions | Instructions from the owner, which apply to every conversation: followed by the text                                                                                |
| Checkpoint               | Summary of the earlier part of this conversation, written when the context ran low: followed by the summary                                                         |
| Continuation note        | Your last response was empty. Continue with your answer to the owner's latest message.                                                                              |
| Stopped reply            | [The owner stopped this reply.]                                                                                                                                     |
| Interrupted reply        | [This reply was interrupted before it finished.]                                                                                                                    |
| Unknown part             | [Content of type {kind} that this version of Mannie cannot show.]                                                                                                   |

The compaction request has instructions of its own. They ask for a checkpoint in the conversation's language, under fixed headings that keep what is done and established, with where it comes from, apart from what nobody checked: the goal, with the owner's requests and preferences; what is done, each with what shows it, such as a changed file, a command's result or a task's report; the facts, numbers, names, file paths and code that still matter, each with its source, such as a path and line, a command and its result, or a task's report; decisions and their reasons; what the assistant assumed, or a report claimed, without evidence; open questions; and what the assistant said it would do next, with the files involved. A heading with nothing under it is left out, and the checkpoint adds nothing that the conversation does not say.

### Errors

A failed reply shows the cause from the run failure codes: `invalid_credential`, `permission_denied`, `quota_exceeded`, `model_not_found`, `endpoint_not_found`, `rate_limited` (with the time of the next possible attempt), `server_error`, `context_overflow`, `request_too_large`, `summary_cut` (a summary reached the output limit, so the compaction kept nothing), `bad_request`, `content_filtered`, `connection_failed`, `timed_out`, `broken_stream`, `invalid_response`, `redirect_refused`, `credential_missing`, `credential_store_unavailable`, `profile_missing`, `no_profile`, `tools_unsupported` (the server refused tools; see [system tools](../system-tools/tool-calls.md#use-tools)) and `internal`. The commands report their own codes through `ChatError`: `message_empty`, `message_too_long`, `title_empty`, `title_too_long`, `instructions_too_many_characters` (`instructions_too_long`, which counted bytes, is deprecated and no longer returned), `conversation_not_found`, `checkpoint_not_found`, `run_active`, `nothing_to_retry`, `nothing_to_compact`, `compacting` (a compaction of the conversation is running), `compaction_failed` (with the run failure that ended it), `profile_not_found`, `written_by_newer_version`, `storage_failed` and `shutting_down`.

### Contracts

- Commands (`apps/desktop/core/src/commands/chat.rs`): `subscribe_conversations`, `subscribe_conversation`, `unsubscribe`, `load_earlier_turns`, `start_conversation` (with the first message, and the project of a conversation that starts in one), `send_message`, `stop_reply`, `retry_reply`, `rename_conversation`, `delete_conversation`, `set_conversation_profile`, `compact_conversation`, `get_checkpoint`, `get_chat_settings`, `update_chat_settings` and `read_image` ([images](../../../plugins/images/docs/README.md)).
- `start_conversation` and `send_message` take the message's `mentions` ([mentions](mentions.md)) and `images` ([images](../../../plugins/images/docs/README.md)). A message with images may have no text, and a new conversation whose first message has none takes the name of its first image as its title.
- Channels carry `ChannelBatch<ConversationUpdate>` and `ChannelBatch<ConversationListUpdate>`. The batch's own counter detects lost batches; each conversation event carries the conversation's sequence number, which is the cursor.
- Limits: a message is at most 256 KiB, a title at most 200 characters, and instructions at most 32,768 characters.
- Numbers that cross IPC are 32-bit integers, and times are milliseconds since the Unix epoch as JSON numbers.

### Data

Migrations `0002` to `0005` in `crates/mannie-store/src/runtime/migrations/` create `runtime_conversations`, `runtime_turns`, `runtime_runs`, `runtime_parts`, `runtime_provider_items`, `runtime_events` and `runtime_checkpoints`. The tables carry the `runtime_` prefix because the same runtime will keep the task conversations of child agents and workers. JSON columns hold versioned documents. Deleting a conversation removes its rows with `secure_delete` on, so the freed pages are overwritten, and then truncates the write-ahead log.

### Threat notes

- Model output and reasoning are untrusted content. Nothing in them changes a setting or grants anything; they reach the webview only through the renderer of [conversation view](conversation-view.md#markdown-rendering).
- Logs at `info` level record runs and steps with their identifiers, states, codes and token counts, never their content.
- A deleted conversation leaves no text in the database file or the write-ahead log.

## Acceptance

- Reloading the window during a reply shows the whole reply without gaps or repeats (end-to-end, with numbered tokens from the mock provider).
- Killing the process at each point where a run stores something leaves an intact transcript, marks the run as interrupted at the next start, and Retry works (crash tests in `mannie-runtime`, and an end-to-end test that kills the app).
- Stop takes effect within one second and keeps the partial text (runtime test and end-to-end test).
- Transient failures are retried with backoff and honor `Retry-After`; permanent failures show their cause (runtime tests with the scripted provider, end-to-end tests with the mock provider).
- Every subscriber, whatever order appends, subscriptions and lag happen in, ends with the same state as a fresh snapshot, and its sequence numbers only increase (property test).
- Compaction keeps a conversation usable at the context limit of every known model and of small local models, and the stored transcript is unchanged afterwards (runtime tests, end-to-end test of automatic and manual compaction).
- Budgets use the token counts the provider reported, and estimates are marked as estimates.
- After deleting a conversation, a planted string from it appears in neither the database nor the write-ahead log.
- Opening a conversation of 10,000 parts takes less than one second (benchmark and performance test).
