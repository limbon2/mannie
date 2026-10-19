# Projects

|              |                                                                                                                                                                                                                                                                                                                                                                              |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                     |
| Milestone    | M2                                                                                                                                                                                                                                                                                                                                                                           |
| Last updated | 2026-10-15                                                                                                                                                                                                                                                                                                                                                                   |
| Part of      | [Chat](README.md)                                                                                                                                                                                                                                                                                                                                                            |
| Related      | [ADR-0023](../../adr/0023-organize-conversations-into-projects.md), [ADR-0020](../../adr/0020-complete-the-ui-shell-slots.md), [Conversation view](conversation-view.md), [Conversations](conversations.md), [System tools](../system-tools/README.md), [Operations and approvals](../system-tools/operations-and-approvals.md), [Tool calls](../system-tools/tool-calls.md) |

## Design

### Projects

- A project is a folder on this computer that the owner added, and the conversations that work in it. Its name is the folder's name until the owner renames it ([ADR-0023](../../adr/0023-organize-conversations-into-projects.md)).
- The folder is stored with its links resolved, as the core checks it when the project is added: it must exist and be a folder. Adding a folder that already is a project opens that project instead of adding a second one.
- A conversation belongs to at most one project. Its workspace is the project's folder ([operations and approvals](../system-tools/operations-and-approvals.md#the-workspace)), so its tools read inside the folder without asking. A general conversation, outside every project, has no workspace.

### The sidebar

The conversation list ([conversation view](conversation-view.md#layout)) has two groups:

- "Projects", above. Its heading collapses and expands the group, and "+" ("Add a project") opens the system's folder dialog from the core. The chosen folder becomes a project, which opens with a new conversation in it.
- Each project is a row with a folder icon, open while the project is expanded, and its name, most recently active first: by its newest conversation, or by the time it was added while it has none. Activating the row collapses or expands the project. On hover or focus, the row offers "New conversation in" the project and a menu with the folder's full path, "Rename…" and "Remove…". A collapsed project shows when one of its conversations waits for approval or writes a reply.
- Below an expanded project, its conversations, most recent first: five at first, then "Show more" and "Show less". A conversation row is the same as in the general list, with its menu to rename or delete it.
- "Conversations", below: the general conversations, with "+" for a new one, as before.
- Whether the "Projects" group and each project are collapsed is remembered in the webview's storage, as the theme is. It is a convenience of one window: when the storage is empty or unreadable, everything starts expanded. "Show more" lasts while the window runs. A project's list always shows the open conversation, also beyond its first five.

### Starting a conversation in a project

"New conversation in" a project opens the pane of a new conversation with the address `?view=chat&project=<id>`, and the project's row shows as the current one. The pane's heading reads "Start a conversation in" the project, and nothing about the project appears near the message box (owner's decision of 2026-10-15). The first message starts the conversation in the project, with the project's folder as its workspace.

### Moving a conversation into a project

- A general conversation whose profile uses tools gets the system tool `move_to_project` while the owner has projects. The environment section of its instructions lists the newest 50 projects with their folders and asks the model to call the tool first when the owner's request is about one of them ([tool calls](../system-tools/tool-calls.md#the-text-the-model-sees)). The call names the project by its folder, or by its name when no other project has it.
- The move is a journaled tool call and shows as a tool row, such as "Moved to mannie". It runs without a question when the owner's message of the turn names the project and no other tool call of the turn came before it, in this reply or a reply that a retry replaced. A message names a project by the project's name or its folder's own name as a word, without regard to case, or by the folder's path. Otherwise Mannie asks: "Move this conversation to the project" with the project's name and folder, with "Allow once" and "Deny". Content that the model read before the move can never move the conversation without the owner, and no grant or rule covers a move.
- The move gives the conversation the project and its folder, from the next tool call on, and the sidebar shows the conversation under the project, which expands if it was collapsed. A conversation in a project never moves; the tool is not offered there, and a call of it fails with `already_in_project`.

### Renaming and removing

- "Rename…" opens a dialog with the name. The core keeps names of 1 to 100 characters without control characters, with white space trimmed.
- "Remove…" asks first: "Remove this project?", with how many conversations are removed with their history and that the folder and its files stay as they are. Its button reads "Remove and delete N conversations", or "Remove" for a project without conversations. The confirmation deletes each conversation as deleting one does ([conversations](conversations.md#data)), stopping a reply that is being written, then the project and the grants for it. When the project has another number of conversations by then, the core removes nothing (`project_changed`), and the dialog shows the new number to confirm. A conversation whose reply is starting or being compacted keeps the project until that ends (`project_busy`), and the dialog says so.

### Adding a project from a typed path

On a Linux desktop without the XDG desktop portal or `zenity`, the folder dialog cannot open. The command palette's "Add a project from a path…" opens a dialog with a text field for the folder, which the core checks as it checks a chosen folder. "+" opens the same dialog, with the cause, when the folder dialog fails or Mannie refuses the chosen folder.

### Commands

The command palette offers "Add a project…" and "Add a project from a path…" in the group "Chat".

### Contracts

- Commands (`apps/desktop/core/src/commands/projects.rs`): `add_project`, which shows the folder dialog and returns the project, or nothing when the owner closed the dialog; `add_project_from_path`; `rename_project`; `remove_project`, with the number of conversations that the owner confirmed. In validation builds, `add_project` takes the folder that `validation_set_folder_choice` set.
- `ProjectView`: the ID, the name, the folder and the time it was added. `ConversationSummary` gains `projectId`, and `StartConversation` gains `projectId`.
- The conversation list's channel: the snapshot gains the projects, and a message follows each project that is added, renamed or removed.
- Errors: `project_not_found`, `project_name_invalid`, `project_busy`, `project_changed` with the number of conversations, and the folder errors `path_refused`, `folder_not_found`, `not_a_folder`, `folder_unreadable` and `folder_dialog_failed`.

### Data

- Migration `0012` creates `runtime_projects`: the ID, the name or none for the folder's name, the folder (unique), and the times of creation and change.
- Migration `0013` adds `project_id` to `runtime_conversations`. It turns each folder that conversations already used as their workspace into a project, with the ID and time of the earliest of them, so they keep their folder.
- A project is deleted only after its conversations, so the reference never dangles.

### Message keys

Keys under `chat_project_` and `chat_projects_`.

### Loading, empty and error states

| View              | Loading                       | Empty                         | Error                                                                    |
| ----------------- | ----------------------------- | ----------------------------- | ------------------------------------------------------------------------ |
| Projects group    | The list's skeleton rows      | "Add a folder to work in it." | The "Conversations" group below shows the list's failure and "Try again" |
| Add from a path   | The check of the folder       | The text field                | The cause, such as a folder that does not exist                          |
| Rename and remove | The button waits for the core | Not applicable                | The cause in the dialog                                                  |

### Threat notes

- The folder dialog runs in the core, and the webview never reads the file system ([ADR-0004](../../adr/0004-keep-system-access-in-the-rust-core.md)).
- A prompt injection may ask the model to move a conversation into a project, so that the model can read the project's files without asking. The move asks the owner unless the owner's own message named the project before the model read anything.
- Removing a project deletes conversations, so the confirmation says how many, and deletion overwrites their text as deleting one conversation does.

### Ideas

- When a general conversation starts working in a folder that is no project yet, Mannie could offer to make it one (deferred by the owner on 2026-10-15).
- The owner could move a conversation back to the general list, or into another project, from its menu. Only the model moves conversations now, and only from the general list (owner's decision of 2026-10-15).

## Acceptance

- "+" with the folder dialog's stand-in adds a project, which opens with a new conversation; its conversation reads inside the folder without asking, and adding the same folder again opens the same project (end-to-end test).
- The group and each project collapse and expand from the mouse and the keyboard, a project shows five conversations and then the rest after "Show more", and a collapsed project shows a reply that waits for approval (end-to-end test).
- Renaming a project changes its row, and removing it says how many conversations go and deletes them (end-to-end test and runtime tests).
- A general conversation whose message names a project moves into it without a question; one whose message does not, or whose move follows another tool call, asks first; a move into a collapsed project expands it (runtime tests and end-to-end test).
- "This project" lets another conversation of the project run what the grant covers, and "Every conversation" lets a general conversation run it (runtime tests and end-to-end test).
- A database at schema 11 with conversations in two workspaces opens with two projects and its conversations in them (store test).
- Axe finds no WCAG 2.1 AA violation in the sidebar and the dialogs, in the light and dark themes, and the pseudo-locale shows only catalog text outside project names and conversation titles.
