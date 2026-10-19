# Mentions

|              |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Milestone    | M6                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Last updated | 2026-10-17                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| Part of      | [Chat](README.md)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Related      | [ADR-0032](../../adr/0032-let-features-and-plugins-add-kinds-of-mentions.md), [ADR-0004](../../adr/0004-keep-system-access-in-the-rust-core.md), [ADR-0023](../../adr/0023-organize-conversations-into-projects.md), [Conversation view](conversation-view.md), [Conversations](conversations.md), [Projects](projects.md), [Conversation mentions](conversation-mentions.md), [Tools and results](../system-tools/tools-and-results.md), [Operations and approvals](../system-tools/operations-and-approvals.md) |

## Intent

The owner wants to mention files in a message with "@", and folders too: "read the @docs folder and implement a feature of something-something". A mention gives the model the file's text, but in the owner's words, "lets try to avoid filling up the whole context just by one file mention". The list shows every file of the project, also the ones that `.gitignore` excludes and hidden ones, because hiding them confused models in other harnesses. Nothing is blocked; "a small warning would be enough to remind the user not to send sensible data unless they truly mean that". Features and plugins can add more kinds of mentions later, such as a sibling conversation or a worker ([ADR-0032](../../adr/0032-let-features-and-plugins-add-kinds-of-mentions.md)).

## Behavior

### The list

- Typing "@" at the start of the message, or after a space, a line break, or an opening parenthesis or bracket, opens a list above the message box. The text between "@" and the caret filters it, and `@"` starts a query that can hold spaces.
- The list groups its entries by kind, under the kind's heading. Each kind shows its own loading state, its failure, and a note when it has more entries than it lists.
- In a conversation without a project, the list says that nothing can be mentioned there, and that the conversations of a project can mention its files and folders.
- Up and Down move through the entries, and go around at either end. Enter or Tab inserts the highlighted entry, and a click inserts the one clicked. While the list shows no entry, Enter sends the message as before.
- An Enter or Tab pressed before the list shows the matches for what the owner typed inserts the first of those matches once they arrive. It inserts nothing when the owner types on first, and never a match for an earlier query.
- Escape closes the list until the owner types in the token or leaves it, and the next Escape stops the reply as before. The focus stays in the message box.
- A file inserts its token and a space, such as `@src/main.rs`. A folder inserts `@docs/` and keeps the list open with the folder's entries, so the owner can go deeper, or type a space to mention the folder itself. A path with spaces is written in quotes, such as `@"notes/release plan.md"`.

### Mentions in the message

- A token is a mention while its exact text stays in the message. Changing or deleting it turns it back into plain text.
- A token that the owner typed without choosing it counts when it equals an entry that the list showed while they typed. Text pasted into the box never becomes a mention.
- Each mention of the message shows as a chip below the box, with the kind's icon and the mention's label. A file that may hold secrets says so on its chip and in the list. Nothing is blocked.
- A message has at most 20 mentions. Sending more fails with a message that says so, and the text stays in the box.

### Files and folders

- In a project's conversations, and in a new conversation in a project, the list offers the files and folders of the project's folder. The best matches come first: an exact name or path, then a name or path that starts with the query, then a name that contains it, then a path that contains it, then a path with the query's characters in order. Among equal matches, entries that `.gitignore` excludes come last.
- The list includes files that `.gitignore` excludes and hidden files. It lists `.git`, but nothing inside it, and it lists links without following them.
- The list shows at most 50 files and folders. A project with more than 100,000 entries is listed up to that number, with the files that `.gitignore` does not exclude first, and the list says that it holds more.
- A file may hold secrets when its name is one that commonly holds keys, passwords or tokens, such as `.env`, or when it lies inside `.ssh`, `.aws` or `.gnupg`. The decisions below list the names.

### What the model gets

- The model reads each mention after the owner's text, in the same message.
  - A file gives its text as `read_file` shows it: the path, the revision and numbered lines. A file that does not fit gives its first lines and a note to read on with `read_file`.
  - A folder gives its entries, breadth first, with the size of each file. Each folder shows at most 50 entries, and a note says that `list_directory` and `find_files` show the rest.
- The mentions of one message share the room of one `read_file` result: 64 KiB, or a quarter of the profile's context budget when that is smaller. Small mentions get all they need, and the others share the rest equally.
- A file that is not text, that is larger than 16 MiB or that cannot be read gives a note instead of its text. So does a file inside Mannie's own data or log folders, unless the conversation is in full access. The message is sent in every case.
- The model reads what the file held when the message was sent, and retrying the reply sends the same text again.

### The transcript

Each mention shows as a chip below the owner's message, with the kind's icon, the label and what the model got: "120 lines", "lines 1 to 1,600 of 5,000", "34 entries", or why nothing was sent.

### What it does not do

- It mentions nothing outside the project's folder, and nothing in a general conversation.
- A mention never asks for an approval. What would need one gets a note.
- Files from elsewhere cannot be added, such as by dragging them into the window.

## Fit

- Adds an extension point: mention providers and the slot of mention kinds ([ADR-0032](../../adr/0032-let-features-and-plugins-add-kinds-of-mentions.md)). The core's first provider, `files`, belongs to this part.
- Uses the transcript parts, with the part `mention`, its rendering for the model and its renderer; IPC commands, with `suggest_mentions` and the `mentions` of `send_message` and `start_conversation`; and chat's messages, under `chat_mention`.
- Reads files through the public interface of system tools: the path checks and the policy of a model's read ([ADR-0004](../../adr/0004-keep-system-access-in-the-rust-core.md), decision 2), and the page that `read_file` shows.
- Must not change: what `read_file` returns, what runs without asking, the owner's message that lets `move_to_project` run without a question (T-19), and retry and compaction of turns without mentions.
- Follows [ADR-0013](../../adr/0013-journal-every-side-effect.md), [ADR-0019](../../adr/0019-keep-visible-text-in-message-catalogs.md), [ADR-0023](../../adr/0023-organize-conversations-into-projects.md), [ADR-0029](../../adr/0029-let-the-owner-give-conversations-and-hosts-full-access.md) and [ADR-0030](../../adr/0030-split-mannie-into-a-core-and-plugins.md).

## Open questions

None.

## Decisions

- 2026-10-16: the owner decided:
  - A mention gives the model the file's text, bounded so that one mention does not fill the context.
  - The list includes files that `.gitignore` excludes and hidden files. Nothing is blocked, and files that may hold secrets get a warning.
  - Folders can be mentioned.
  - Mentions are M6, which ships together with M5 in v0.3.0 (D29).
- 2026-10-17: the owner decided that mentions are an extension point, so that other features and plugins can add kinds of mentions ([ADR-0032](../../adr/0032-let-features-and-plugins-add-kinds-of-mentions.md)).
- 2026-10-17: the defaults below were chosen when shaping. Each one waits for the owner's review.
  - A message has at most 20 mentions.
  - The mentions of one message share the room of one `read_file` result.
  - The list shows at most 50 files and folders, and a folder's listing at most 50 entries for each folder.
  - The list's index of a project holds at most 100,000 entries, and is kept for 15 seconds.
  - These files may hold secrets: `.env`, and `.env.*` except names that end in `.example`, `.sample` or `.template`; names that end in `.pem`, `.key`, `.p12`, `.pfx`, `.jks` or `.keystore`; `id_rsa`, `id_dsa`, `id_ecdsa` and `id_ed25519`; `.npmrc`, `.pypirc`, `.netrc`, `_netrc`, `.pgpass`, `.git-credentials`, `.htpasswd`, `credentials`, `credentials.json` and `secrets.*`; and every file inside `.ssh`, `.aws` or `.gnupg`.
  - A typed token counts when the list showed it, and pasted text never does.
- 2026-10-17: the owner accepted ADR-0032 and the defaults above, and building started (D30).

## Design

### Contracts

- The command `suggest_mentions` (`apps/desktop/core/src/commands/chat.rs`) takes the place and the query, and returns a group for each provider that the place offers. A group has its entries, whether the provider holds more than it looked through, and a failure code. An entry has a key, a label, the token that it inserts, a variant, a warning code and whether it expands.
- `send_message` and `start_conversation` gain `mentions`, a list of references with a provider and a key, which is empty when left out.
- Errors: `too_many_mentions` with the limit, `mention_unavailable` with the provider, and `mention_refused` with the provider and its reason. `suggest_mentions` also fails with `conversation_not_found`, `project_not_found` and `storage_failed`.
- The part `mention` holds its provider, key, label and variant, and a summary: the provider's code with its numbers by name.
- The codes of the files provider:
  - the variants `file` and `folder`, and the warning `secret`;
  - the refusals `outside_workspace` and `no_workspace`, and the list's failure `folder_unreadable`;
  - the summaries `lines` with `first`, `last` and `total`, `entries` and `entries_cut` with `shown`, `too_large` with `limitMebibytes`, and `not_text`, `unreadable`, `needs_approval` and `not_found`.

### Data

- No migration. A mention is a row of `runtime_parts` with the kind `mention` and no run, at the positions after the owner's text, written in the transaction that starts the turn. Its text is what the model reads, and its document holds the view.
- `turn_started` carries every part of the owner, and the policy reads the owner's words from the `user_text` part alone.

### The text the model sees

The headers and notes live in `crates/mannie-runtime/src/texts.rs`, and the notes of pages and listings in `crates/mannie-tools/src/texts.rs`.

| Mention        | What the model reads                                                                                                                                                                                                              |
| -------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A file         | "The owner mentioned the file {path}. Its text follows, as read_file shows it.", then the page that `read_file` shows from line 1, which ends with "More follows: read_file reads on with offset {next}." when lines are left out |
| A folder       | "The owner mentioned the folder {path}. Its entries follow, breadth first, as paths below the workspace; a folder's path ends with /.", then a line for each entry, and a note when entries are left out                          |
| Not text       | "The owner mentioned the file {path}, which is not text, so its content is not included."                                                                                                                                         |
| Too large      | "The owner mentioned the file {path}, which is larger than 16 MiB, so its content is not included."                                                                                                                               |
| Unreadable     | "The owner mentioned {path}, which could not be read, so its content is not included."                                                                                                                                            |
| Needs approval | "The owner mentioned the file {path}. Reading it needs the owner's approval, so its content is not included.", or the folder                                                                                                      |
| Not found      | "The owner mentioned {path}, which does not exist."                                                                                                                                                                               |
| In a summary   | "Owner's mention:" and the first 500 characters of the mention's text                                                                                                                                                             |

In a folder's listing, a file's line has its size, a folder's line notes "({shown} of {held} entries shown)" or "(entries not shown)" when not all its entries show, and the listing ends with "Not every entry is shown: list_directory and find_files show the rest." when it leaves entries out.

### Message keys

Keys under `chat_mention_`, and `chat_error_too_many_mentions`, `chat_error_mention_unavailable` and `chat_error_mention_refused`. The message box's hint names "@".

### Loading, empty and error states

| View         | Loading                              | Empty                                                                                                        | Error                                                                                           |
| ------------ | ------------------------------------ | ------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------- |
| The "@" list | A spinner and "Looking for matches…" | "Nothing matches.", or, where no kind is offered, that nothing can be mentioned in this conversation and why | What failed, such as a project that no longer exists; a kind that failed says so below the list |
| Sending      | The send button waits for the core   | Not applicable                                                                                               | The cause, with the kind's reason for a refused mention; the text stays in the box              |

### Threat notes

- The webview is untrusted, so the core checks every reference again. A key that is absolute or leads out of the workspace is refused before anything touches the file system. The path checks of system tools apply to the rest, with every link target checked again, and the policy decides on the resolved path as it does for the model's read ([ADR-0004](../../adr/0004-keep-system-access-in-the-rust-core.md), decision 2). A mention never asks: a file that would need approval, such as one in Mannie's own folders, gives the model a note.
- The suggestions return names inside the project's folder, which the model can already list without asking, and the view of a mention carries no content, so nothing new reaches the webview.
- What a mention gives the model never counts as the owner's words: a move into a project that a mentioned text names still asks (T-19).
- Content read for a mention grants no permission. Providers log labels and content only at `debug`.

### Platform differences

Tokens, labels and keys separate names with `/` on every platform. On Windows, a key also takes `\`, and the checks of paths refuse what Windows treats specially, such as UNC and device paths.

## Acceptance

- The index lists hidden and ignored entries, and `.git` without its contents, keeps the project's own files under its cap, and follows no link. The ranking, the names that may hold secrets, the checks of keys, and the pages and listings within their shares behave as described (unit tests in `crates/mannie-tools/src/mentions/`).
- The budget is split fairly, and the files provider gives the notes above (unit tests in `crates/mannie-runtime/src/mentions/`). The model reads a mention after the owner's text, and a summary reads its start (unit tests of the context and of compaction).
- A turn stores its mentions in order, `turn_started` carries them, and the policy's turn request holds only the owner's text (store tests).
- The runtime tests (`crates/mannie-runtime/tests/mentions.rs`) show that:
  - a file and a folder reach the model after the owner's text, within the shared budget;
  - a file in Mannie's folders gives a note, unless the conversation has full access;
  - a refused reference stores nothing, and a retry sends the same text;
  - a fake plugin's provider is offered where it says, leaves with its plugin, and needs an ID of its own;
  - the budget is divided across providers, and a provider that renders too much is cut;
  - a mention never lets `move_to_project` run without asking.
- The tokens, the list's state, the words of the files kind, the fallbacks and the slot of mention kinds pass their Vitest tests. The chips below the message box and below the owner's message show the words of their kind, or generic words without one, and a retry keeps a message's mentions in the transcript.
- The end-to-end test (`tests/e2e/specs/chat-mentions.test.ts`) types "@" in a project, sees hidden and ignored entries and the warning on `.env`, goes into a folder and inserts files from the keyboard, sends them, and finds the chips in the transcript and the text in the model's request. Escape closes the list before it stops a reply, and a general conversation lists nothing. Axe finds no WCAG 2.1 AA violation with the list open.
