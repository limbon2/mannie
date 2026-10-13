# Chat

|              |                                                                                                                                                                                                                                                                                     |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Accepted                                                                                                                                                                                                                                                                            |
| Milestone    | M1                                                                                                                                                                                                                                                                                  |
| Last updated | 2026-10-13                                                                                                                                                                                                                                                                          |
| Related      | [ADR-0011](../adr/0011-own-the-provider-adapters.md), [ADR-0012](../adr/0012-run-agents-as-durable-actors.md), [ADR-0010](../adr/0010-keep-provider-credentials-on-the-master.md), [Agent runtime](../architecture/agent-runtime.md), [Desktop app](../architecture/desktop-app.md) |

## Intent

The owner wants general chat with AI models in the desktop app, using OpenAI models and their own OpenAI-compatible model servers. Conversations are kept, and a reload, a crash or a provider hiccup never loses a reply.

## Behavior

- Conversations: the owner creates, opens, renames and deletes conversations. The list shows the most recent first. A new conversation takes its title from the first message until the owner renames it. Deleting asks for confirmation and removes the conversation for good.
- Sending: the owner writes a message, and the reply streams in as the model generates it. When the model provides reasoning summaries, they appear collapsed above the answer.
- Rendering: replies render as Markdown with code blocks and a copy button. Raw HTML is not rendered, remote images do not load, and links open in the system browser only after the owner confirms the full URL.
- Stop and retry: the owner can stop a reply at any time, and the partial text stays, marked as stopped. A failed, stopped or interrupted reply can be retried, and the new attempt replaces it.
- Durability: a reply keeps streaming when the window reloads. After a crash, an unfinished reply is marked as interrupted and can be retried.
- Errors: an error names its cause, for example an invalid key, a rate limit with the time until the next attempt, a context overflow or a server error. Transient errors are retried automatically, and the reply shows the retry.
- Provider profiles: the owner adds, edits and removes profiles. A profile has a name, an API interface (Responses or Chat Completions), a base URL, a model, a credential, the context window, the output limit and the reasoning effort. Credentials go to the operating system credential store. One profile is the default, and each conversation can switch to another.
- Testing a profile: a test sends a short request and reports what worked and what failed. The tool round trip is added to the test with [system tools](system-tools.md).
- Context: a meter shows how much of the model's context the conversation uses. When it runs low, Mannie compacts older turns into a checkpoint automatically. The owner can view the checkpoint and start a compaction by hand. The stored conversation never changes.
- Instructions: the owner can write global instructions that apply to every conversation.
- Not included: attachments and images, search, export, branching from a message, and more than one window. Each of these would be a feature of its own.

## Fit

- Extension points: provider adapters for Responses and Chat Completions and their quirk profiles; transcript parts for text, reasoning and errors; run events; instruction sections for the base instructions and the owner's instructions; IPC commands and channels for conversations, runs and profiles; UI shell slots for the conversation list, the conversation view and the settings sections for providers and instructions; store tables for conversations, turns, parts, events and profiles; messages under the `chat_` key prefix.
- Follows [ADR-0004](../adr/0004-keep-system-access-in-the-rust-core.md), [ADR-0005](../adr/0005-build-the-ui-with-sveltekit-as-a-static-spa.md), [ADR-0006](../adr/0006-use-shadcn-svelte-through-its-cli.md), [ADR-0007](../adr/0007-generate-ipc-bindings-from-rust.md), [ADR-0009](../adr/0009-persist-state-in-sqlite.md), [ADR-0010](../adr/0010-keep-provider-credentials-on-the-master.md), [ADR-0011](../adr/0011-own-the-provider-adapters.md), [ADR-0012](../adr/0012-run-agents-as-durable-actors.md) and [ADR-0019](../adr/0019-keep-visible-text-in-message-catalogs.md).

## Open questions

None.

## Decisions

- 2026-10-13: the first providers are the OpenAI Responses API and OpenAI-compatible Chat Completions servers (D7).
- 2026-10-13: the interface is in English, with every piece of visible text in the message catalog (D10).
- 2026-10-13: what happens when the window closes during a reply is decided in the [window and tray](window-and-tray.md) feature (D13).

## Design

Written when M1 starts.

## Acceptance

- Replies stream from recorded fixtures of every supported server (V8), and live from OpenAI.
- Reloading the webview during a reply shows the whole reply without gaps.
- Killing the app during a reply leaves the conversation intact and the reply retryable.
- Stop takes effect within one second and keeps the partial text.
- Transient failures are retried with backoff and honor `Retry-After`. Permanent failures show their cause.
- Compaction keeps a conversation usable at each supported model's context limit, and the stored transcript stays unchanged.
- Model output from the corpus of hostile Markdown and HTML runs no script and loads nothing remote.
- A planted test credential appears in no log, database or crash file.
