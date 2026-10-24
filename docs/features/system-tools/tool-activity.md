# Tool activity

|              |                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Milestone    | M2                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Last updated | 2026-10-24                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Part of      | [System tools](README.md)                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Related      | [ADR-0020](../../adr/0020-complete-the-ui-shell-slots.md), [ADR-0023](../../adr/0023-organize-conversations-into-projects.md), [ADR-0019](../../adr/0019-keep-visible-text-in-message-catalogs.md), [ADR-0043](../../adr/0043-let-plugins-offer-the-tools-of-tool-servers.md), [ADR-0044](../../adr/0044-let-tool-results-carry-images-to-the-model.md), [Conversation view](../chat/conversation-view.md), [Operations and approvals](operations-and-approvals.md) |

## Design

### Tool calls in the reply

A part renderer for `tool_call` shows each call as one row in the reply, in the order the model made them. A row looks like the row of the model's reasoning: a small button with an icon, a line of text and a chevron, so that a reply that reads, searches and runs commands stays a short list of what the model did.

- The row shows the call's activity: the few words in which the model said what the call does, such as "Reading package.json" ([tool calls](tool-calls.md#the-activity-of-a-call)). A call without an activity shows a line that Mannie builds from its arguments instead, such as "Read src/main.rs" or "Run cargo test", and a call that the model still writes names its tool's action, such as "Read a file".
- The icon is the tool's, or a spinner while the call is prepared or runs.
- After the activity, the row names the statuses that need the owner's eye: "Waiting for your approval", "Failed", "Denied", "Canceled", "May have run" for an uncertain operation, and "Not run". A running command shows how long it has run, such as "Running for 12 sec". A call that is prepared, waits to run, runs or ended well shows only its icon, and screen readers hear its status: "Preparing the call", "Waiting to run", "Running" or "Done".
- Activating the row opens the details below it, indented like the text of reasoning:
  - the line that Mannie built from the call, such as "Read src/main.rs, lines 1 to 200", "Edited src/main.rs, 2 replacements" or "Ran cargo test: exit code 0, 3.2 sec", when the row shows an activity;
  - for a failed, denied, canceled, uncertain or skipped call, one sentence on what happened, from the catalog;
  - the canonical paths with their access modes, or the command with its shell and working directory, or for a call of a tool server's tool the tool, the server with its program or URL, the arguments and the server's hints, which say that Mannie does not check them;
  - the arguments as the model wrote them, what allowed the call, and the result as the model read it;
  - "Images the model saw", with a button for each image of the result that opens it in the image viewer, with its size.
- Activities, paths, commands, arguments, file contents and output are content: they render as plain text inside elements marked `data-content`, never as Markdown or HTML. An activity renders in its own direction, isolated from the text around it. A block that scrolls takes the keyboard focus, so it scrolls from the keyboard.
- Consecutive rows sit closer together than the other parts of a reply, so a run of tool calls reads as one list.
- For a command or another archived output, "Show full output" or "Show full error output" opens a dialog that loads the output 500 lines at a time through `read_tool_output`, with "Load more".
- The calls of background commands show their command's state, and the conversation's menu and a side panel show the commands and their output ([background commands](background-commands.md#interface)).

### Approval requests

- A call that waits for the owner shows the request below its row, without a click: what will happen, exactly as the core described it from the call's canonical accesses, why Mannie asks, and what "Allow for…" would cover. The request never repeats the activity as what will happen.
- The buttons read "Allow once", "Allow for…", "Save rule…" and "Deny". "Allow for…" appears only when the request offers grants, and opens a menu with "This conversation", "This project" in a project's conversation, and "Every conversation" ([operations and approvals](operations-and-approvals.md#grants)). "Save rule…" appears only for commands that a rule can cover. "Save rule…" opens a dialog that lists the command's leading words as choices, with the whole command chosen at first, and a switch "Only in this folder and the folders inside it", on at first; it ends with "Save rule and allow" or "Cancel".
- The request is announced through a status region, and it never takes the focus from the message box. Every button works from the keyboard.
- A decision that the core refuses, because the request changed or no longer waits, shows the cause below the row, and the row shows the current request.
- The transcript keeps following the reply while the owner is at its bottom, so a request that arrives while the owner reads the newest message shows with its buttons ([conversation view](../chat/conversation-view.md#layout)). When the owner has scrolled up, the button that brings the owner back reads "Jump to the approval request" while a call waits.

### Status and the conversation list

- Below the reply, the status line says "Waiting for your approval." while a call waits, and "Running a command…" or "Running a tool…" while one runs.
- The conversation list marks a conversation that waits for an approval with an icon labeled "Waiting for your approval", in place of the spinner of a reply that is being written, so the owner finds it from another conversation.

### Projects

A conversation works in the folder of its project, which the owner adds in the sidebar ([projects](../chat/projects.md)). The core opens the system's folder dialog over the main window through the `rfd` crate: the system's dialog on Windows, and on Linux the dialog of the XDG desktop portal, which GNOME, KDE and other desktops provide, through the system's D-Bus library that `rfd` loads when it runs. Without a portal, `rfd` runs `zenity` instead, and without either the dialog does not open; the owner then types the path.

A move of a general conversation into a project shows as a tool row, such as "Moved to mannie", and asks like any other call when the owner's message did not name the project ([projects](../chat/projects.md#moving-a-conversation-into-a-project)).

### Permissions of a conversation

A dialog lists the grants that apply to the conversation, such as "Changes in D:\Projects\app\src" or "The command cargo test in D:\Projects\app", under "This conversation", "This project" and "Every conversation", each with "Revoke". The command palette opens it.

### Full access

- A conversation's message box has the button "Full access" in the row below its text ([ADR-0029](../../adr/0029-let-the-owner-give-conversations-and-hosts-full-access.md), [ADR-0042](../../adr/0042-place-conversation-controls-in-a-menu-and-the-message-box.md)), a toggle button whose pressed state screen readers hear. While full access is on, it shows in the destructive color.
- Turning it on asks first, in a dialog: "Turn on full access?", what will run without asking, Mannie's own files and the requests of worker tasks included, and the risk that text the model reads makes it run commands; "Turn on full access" and "Cancel". Turning it off happens at once; when that fails, a dialog says so, with "Try again" and "Close".
- The conversation list marks a conversation in full access with an open lock labeled "Full access".
- The details of a call that full access allowed say "Full access allowed this.", or "The worker's full access allowed this." for a worker in that mode.
- A task conversation follows its parent and shows no button.

### Settings

The Tools section of the settings holds:

- a switch "Allow changes inside the workspace without asking", off by default, with a description of what it allows;
- a switch "Full access for new conversations", off by default, which asks first, in a dialog, before it turns on;
- a switch "Wake the agent when a background command ends", on by default ([wake-ups](wake-ups.md));
- the command rules, each with its words, its folder and "Delete";
- the grants for every conversation and for each project, each with "Revoke"; a grant for a tool server's tool names the tool, the server and its program or URL, and says that it covers any arguments;
- the extra environment variables that commands receive, one name per line, with "Save"; a refused name shows why.

The section's ID is `tools.settings`, after chat's sections and before the window's.

The provider profile dialog of chat gets the switch "Use tools", and the provider test lists its Tools check.

### Commands

The command palette offers "Show the permissions of this conversation", in the group "Tools", while a conversation is open.

### Message keys

The feature's components use keys under `tools_`. The states that chat shows for the runtime (the status lines, the run failure `tools_unsupported`, the Tools check and the "Use tools" switch) use keys under `chat_`.

### Loading, empty and error states

| View               | Loading        | Empty                                                                       | Error                                                         |
| ------------------ | -------------- | --------------------------------------------------------------------------- | ------------------------------------------------------------- |
| Full output dialog | Skeleton lines | "This output is empty"                                                      | What failed and "Try again"                                   |
| Permissions dialog | Skeleton rows  | "None" under each of its three groups                                       | What failed and "Try again"                                   |
| Tools settings     | Skeleton rows  | "No rules yet" in the list of rules, "No permissions" in the list of grants | What failed and "Try again"; a refused change shows its cause |

### Threat notes

- T-3: activities, file contents, output, paths and commands render as text in their own elements. No content can look like a button of the approval, because the buttons are part of the row's own layout, outside the content.
- An activity is the model's description of its own call, and a compromised model can make it lie, such as "Reading package.json" for a call that deletes a folder. The approval request shows what the core prepared, so that call asks to delete the folder, and nothing that authorizes a call reads the activity.
- A decision names the digest of the request the owner saw, so a request that changed after the owner read it is shown again instead of being approved.

### Tests

WebDriver cannot reach the system's folder dialog. In validation builds, `add_project` answers with the folder that the validation command `validation_set_folder_choice` set, or as a closed dialog, as link opening records links instead of opening them. The fixture provider of the end-to-end tests answers the provider test's Tools check, and its tools scenario calls the tool that each message of a test names. `tests/e2e/specs/tools.test.ts` covers the acceptance below, in a conversation of a project, and `projects.test.ts` the grants for a project and for every conversation; the pseudo-locale spec covers a row that waits for approval, with its details and the menu of "Allow for…", and the Tools settings.

## Acceptance

- A read inside the workspace runs without a question, a change asks, "Allow once" runs it, "Deny" reaches the model, "For this conversation" lets the next change run, and "Save rule…" saves a rule that the settings list (end-to-end tests).
- A call shows as one row with its activity, and its details show the line that Mannie built and the command or paths (end-to-end test).
- A reply that makes many calls and then asks shows the request with its buttons in view, without the owner scrolling; the transcript never offers to jump while the owner did not scroll, and nothing around it can scroll (end-to-end test and unit tests of the following).
- Stop during a running command ends it, and a reload while an approval waits shows the request again (end-to-end tests).
- Killing the app during a command leaves the call marked as "May have run", and Retry continues without running it again (end-to-end test).
- "Allow for…", the permissions dialog and the Tools settings work from the keyboard (end-to-end tests).
- A conversation in full access runs a change and a command without a request, its button and the conversation list mark it, turning it off asks again, and the settings' switch makes new conversations start in full access after its confirmation (end-to-end tests, with axe on the dialogs and the marks in both themes).
- Axe finds no WCAG 2.1 AA violation in the rows, their details, the approval requests, the dialogs and the Tools settings, in the light and dark themes.
- In the pseudo-locale, every piece of text outside content comes from the catalog, and no layout overflows.
