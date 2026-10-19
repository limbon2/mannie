# Chat

|              |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Milestone    | M1                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Last updated | 2026-10-17                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Related      | [ADR-0011](../../adr/0011-own-the-provider-adapters.md), [ADR-0012](../../adr/0012-run-agents-as-durable-actors.md), [ADR-0010](../../adr/0010-keep-provider-credentials-on-the-master.md), [ADR-0020](../../adr/0020-complete-the-ui-shell-slots.md), [ADR-0021](../../adr/0021-render-model-output-as-markdown-with-micromark.md), [Agent runtime](../../architecture/agent-runtime.md), [Desktop app](../../architecture/desktop-app.md), [Window and tray](../window-and-tray.md) |

## Intent

The owner wants general chat with AI models in the desktop app, using OpenAI models and their own OpenAI-compatible model servers. Conversations are kept, and a reload, a crash or a provider hiccup never loses a reply.

## Behavior

- Conversations: the owner creates, opens, renames and deletes conversations. The list shows the most recent first. A new conversation takes its title from the first message until the owner renames it. Deleting asks for confirmation and removes the conversation for good.
- Projects: the owner adds folders as projects in the sidebar, above the general conversations. A conversation started in a project works in its folder, a general conversation moves into a project when the owner's request is about it, and removing a project deletes its conversations ([projects](projects.md)).
- Mentions: "@" in the message box lists what the message can mention, such as the files and folders of the conversation's project, and the model reads a bounded part of each mention after the message ([mentions](mentions.md)).
- Images: the plugin Images adds images to a message, which the model reads after the message and its mentions ([images](../../../plugins/images/docs/README.md)). The message box and the transcript show them, also while the plugin is off.
- Sending: the owner writes a message, and the reply streams in as the model generates it. When the model provides reasoning summaries, they appear collapsed above the answer.
- Rendering: replies render as Markdown with code blocks and a copy button. Raw HTML is not rendered, remote images do not load, and links open in the system browser only after the owner confirms the full URL.
- Stop and retry: the owner can stop a reply at any time, and the partial text stays, marked as stopped. A failed, stopped or interrupted reply can be retried, and the new attempt replaces it.
- Durability: a reply keeps streaming when the window reloads. After a crash, an unfinished reply is marked as interrupted and can be retried.
- Errors: an error names its cause, for example an invalid key, a rate limit with the time until the next attempt, a context overflow or a server error. Transient errors are retried automatically, and the reply shows the retry.
- Provider profiles: the owner adds, edits and removes profiles. A profile has a name, an API interface (Responses or Chat Completions), a base URL, a model, a credential, the context window, the output limit and the reasoning effort. Credentials go to the operating system credential store. One profile is the default, and each conversation can switch to another. A profile can also turn off the model's use of tools ([system tools](../system-tools/README.md)).
- Testing a profile: a test sends a short request and reports what worked and what failed, including a round trip with a tool ([system tools](../system-tools/README.md)).
- Context: a meter shows how much of the model's context the conversation uses. When it runs low, Mannie compacts older turns into a checkpoint automatically. The owner can view the checkpoint and start a compaction by hand. The stored conversation never changes.
- Instructions: the owner can write global instructions that apply to every conversation.
- Not included: attachments other than mentions and images, search, export, branching from a message, and more than one window. Each of these would be a feature of its own.

## Fit

- Extension points: provider adapters for Responses and Chat Completions and their quirk profiles; transcript parts for text, reasoning and errors; run events; the conversation agent's definition; instruction sections for the base instructions, the owner's instructions and the checkpoint; IPC commands and channels for conversations, runs and profiles; UI shell slots for the conversation list, the conversation view, the part renderers and the settings sections for providers and instructions ([ADR-0020](../../adr/0020-complete-the-ui-shell-slots.md)); store tables for projects, conversations, turns, runs, parts, events, checkpoints and profiles; messages under the `chat_` and `markdown_` key prefixes.
- Follows [ADR-0004](../../adr/0004-keep-system-access-in-the-rust-core.md), [ADR-0005](../../adr/0005-build-the-ui-with-sveltekit-as-a-static-spa.md), [ADR-0006](../../adr/0006-use-shadcn-svelte-through-its-cli.md), [ADR-0007](../../adr/0007-generate-ipc-bindings-from-rust.md), [ADR-0009](../../adr/0009-persist-state-in-sqlite.md), [ADR-0010](../../adr/0010-keep-provider-credentials-on-the-master.md), [ADR-0011](../../adr/0011-own-the-provider-adapters.md), [ADR-0012](../../adr/0012-run-agents-as-durable-actors.md), [ADR-0019](../../adr/0019-keep-visible-text-in-message-catalogs.md) and [ADR-0021](../../adr/0021-render-model-output-as-markdown-with-micromark.md).

## Open questions

None.

## Decisions

- 2026-10-13: the first providers are the OpenAI Responses API and OpenAI-compatible Chat Completions servers (D7).
- 2026-10-13: the interface is in English, with every piece of visible text in the message catalog (D10).
- 2026-10-13: what happens when the window closes during a reply is decided in the [window and tray](../window-and-tray.md) feature (D13).
- 2026-10-14: the defaults below were chosen when building started. Each one waits for the owner's review, and the part documents give the details.
  - Archived outputs move to M2, together with the first tool results that need them. M1 has no tool results.
  - A new conversation is created when the owner sends its first message, so the list never holds empty conversations. Its title is the first line of that message, at most 80 characters.
  - Only the latest turn can be retried, after its reply failed, was stopped or was interrupted.
  - Mannie retries a transient failure up to five times (six attempts), with a randomized delay between one second and one minute, and waits for `Retry-After` when the provider sends it. A provider that asks for more than ten minutes ends the reply with the time of the next possible attempt.
  - At most eight model calls run at once across all conversations. Replies over the limit wait in order.
  - A credential travels over plain HTTP only to a loopback address or a private network address written as an IP address.
  - Raw HTML in model output is shown as text, and images are never loaded: their description and a link take their place.
  - Links open after a confirmation in Mannie's window that shows the URL as the core checked it. Only http and https links open.
  - Escape stops the reply while the message box has the focus.
- 2026-10-15: the owner decided:
  - Projects in the sidebar replace the workspace menu near the message box: "+" turns a folder into a project, and the conversations of a project work in its folder. Projects and each project collapse ([ADR-0023](../../adr/0023-organize-conversations-into-projects.md)).
  - Removing a project deletes its conversations, after a confirmation that says how many.
  - A general conversation moves into a project when the owner's request is about it: the model moves it with a tool call, without a question when the owner's message names the project, and otherwise after asking. A conversation in a project never moves. Creating a project automatically for another folder is deferred.
  - Nothing about the project appears near the message box of a new conversation.
- 2026-10-15: the defaults below were chosen when building started. Each one waits for the owner's review, and [projects](projects.md) gives the details.
  - Projects are listed most recently active first, and each shows its five most recent conversations before "Show more".
  - A project's name is its folder's name until the owner renames it, and adding a folder that already is a project opens that project.
  - A move without a question needs the owner's message to name the project, and no other tool call of the turn may come before it. The model's instructions list the newest 50 projects.
  - Whether the projects are collapsed is remembered in the webview's storage, and a move into a collapsed project expands it.
  - The permissions of a conversation open from the command palette, since the workspace menu that held them is gone, and the Tools settings list the grants for projects and for every conversation.

## Parts

- [Providers](providers.md): provider profiles and their credentials, the provider test, the adapters and their quirk profiles, and the recorded fixtures of V8.
- [Conversations](conversations.md): runs, the transcript and its events, durability, stop and retry, the context budget and compaction, the owner's instructions and the text the model sees.
- [Conversation view](conversation-view.md): the conversation list and view, the Markdown renderer, links, and the settings sections for providers and instructions.
- [Projects](projects.md): projects in the sidebar, conversations in a project, moving a conversation into one, and removing a project.
- [Mentions](mentions.md): the "@" list of the message box, its kinds of mentions from features and plugins, the files and folders of a project, and what the model reads of them.
- [Conversation mentions](conversation-mentions.md): an idea, mentioning a sibling conversation.
