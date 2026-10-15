# Child agent activity

|              |                                                                                                                                                                                                                                                                                                                     |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                            |
| Milestone    | M3                                                                                                                                                                                                                                                                                                                  |
| Last updated | 2026-10-15                                                                                                                                                                                                                                                                                                          |
| Part of      | [Child agents](README.md)                                                                                                                                                                                                                                                                                           |
| Related      | [ADR-0025](../../adr/0025-add-side-panels-to-the-shell.md), [ADR-0020](../../adr/0020-complete-the-ui-shell-slots.md), [ADR-0019](../../adr/0019-keep-visible-text-in-message-catalogs.md), [Tool activity](../system-tools/tool-activity.md), [Conversation view](../chat/conversation-view.md), [Tasks](tasks.md) |

## Design

### The card of a task

A part renderer for `task` shows each start and follow-up as a card in the reply:

- The child's number and title, the title as content, and on the same line the kind of agent: "Explore" or "General".
- The task's state: "Waiting for a free slot", "Working", "Waiting for your approval", "Reported", "Failed", "Stopped" or "Interrupted". Screen readers hear a change of state through a status region, which never takes the focus.
- While the child works: the activity of its current tool call, as content, and how many calls it made.
- "Open conversation", which opens the child's conversation in the side panel with the number and title as the tab's label, and "Stop" while the child works.
- "Details", which opens the start of the instruction, as content, and the report, rendered as Markdown with the renderer of [ADR-0021](../../adr/0021-render-model-output-as-markdown-with-micromark.md), or the failure's message.

A follow-up shows "Follow-up" beside the agent, and a call that never started shows why, from the catalog. A `wait_for_tasks` shows as one row, like a tool call's, that names the tasks it waits for and whether their reports came.

### A child's approval in the parent

While a child waits for the owner, its card shows the waiting call below the card's own lines, without a click. The card builds the call's part from what the parent's part mirrors and renders it through the shell's part outlet with the child's conversation, so the row and the request of [tool activity](../system-tools/tool-activity.md#approval-requests), with "Allow once", "Allow for…", "Save rule…" and "Deny", work there as for the parent's own calls. "Allow for… This conversation" allows it for the parent conversation and all its children. The request is announced through the status region and never takes the focus from the message box.

"Jump to the approval request" brings the owner to the part that waits, which may be a card above later parts of the reply.

### The child's conversation in a side panel

"Open conversation" opens the panel kind `chat.conversation` of the [side panels](../../adr/0025-add-side-panels-to-the-shell.md) with the child's conversation:

- The panel shows the child's whole conversation as the conversation view does: its turns, its tool calls with their details and approvals, its status line, and its earlier turns as the owner scrolls up.
- It is read-only: it has no message box and offers no Retry, and its footer says that the conversation belongs to a child agent, with "Stop" while the child works.
- Several open conversations show as tabs, each with its Close inside the tab, and Delete closes the focused tab. The panels belong to the parent conversation: opening another conversation leaves them behind, and coming back through the settings or a reload shows them again.
- The panel's heading takes the focus when the owner opens it. Closing the last panel returns the focus to the card's button, and closing one of several moves it to the tab that shows.

A conversation view that opens a task conversation by its address shows it in the same read-only way.

### The child agents of a conversation

The conversation's toolbar shows "Child agents" before the context meter once the conversation has child agents, through the conversation controls of [ADR-0026](../../adr/0026-add-conversation-controls-back-for-child-agents.md). While children work, the button shows how many, and its accessible name says so. Its menu lists the child agents of the turns that the view has loaded, one for each child with its latest task: the ones that work, then the ones that ended, each with its number, title and state. Choosing one opens the child's conversation in the side panel, as "Open conversation" does, and closing that panel returns the focus to the button. A child's own conversation has no child agents, so its toolbar shows no button.

### Status and the conversation list

- Below the reply, the status line says "Waiting for 2 child agents…" while the reply waits for its tasks, and "Waiting for your approval." while a child waits for the owner.
- The conversation list marks the parent conversation as waiting for an approval while one of its children waits, and lists no task conversation.

### Settings

The section "Child agents", with the ID `child-agents.settings`, after the Tools section, holds "Child agents at once", a choice from 1 to 16 with 4 marked as the default, and a description of what waits when the limit is reached. A change applies at once: waiting children start when the limit grows.

### Message keys

The feature's components use keys under `child_agents_`. The states that chat shows for the runtime (the status line, the read-only footer and the failure `task_conversation`) use keys under `chat_`, and the frame of the side panel uses keys under `shell_`.

### Loading, empty and error states

| View                    | Loading                                                          | Empty                                                         | Error                                                                                   |
| ----------------------- | ---------------------------------------------------------------- | ------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| Card of a task          | The state "Waiting for a free slot" or "Working", with a spinner | None: a card exists once its call exists                      | The task's failure and what the parent does next                                        |
| Conversation in a panel | Skeleton turns                                                   | Not possible: a task conversation starts with its instruction | What failed and "Try again"; a conversation that no longer exists says so, with "Close" |
| Child agents settings   | Skeleton row                                                     | Not possible: the setting has a default                       | What failed and "Try again"; a refused change shows its cause                           |

### Threat notes

- T-3: titles, activities, instructions and reports are content, written by models. They render as text in elements marked `data-content`, and a report as Markdown through Mannie's own renderer, never as HTML.
- T-23: a child's request shows inside the child's card, with the child's number and title, and its buttons belong to the request's own layout, outside every piece of content.

### Tests

The scripted provider of the end-to-end tests plays the calls that a message lists, and a child's instruction is the message of its turn, so a parent's message can script its children: `tests/e2e/specs/child-agents.test.ts` covers the acceptance below. The scripted provider leaves Mannie's notes for the model out when it reads the latest message, so a child's report carries the result it read last.

## Acceptance

- A parent starts a general child whose write asks the owner: the request shows in the child's card with its number and title, "Allow once" runs the write, and the card shows the report (end-to-end test).
- "Open conversation" opens the child's conversation in the side panel, read-only with "Stop", as a tab beside the other open children; a tab's Close or Delete closes it, and closing the last tab returns the focus to the card (end-to-end test).
- With "Child agents at once" set to 1, the second of two tasks shows "Waiting for a free slot" until the first ends, and the setting stays after a reload (end-to-end test).
- "Stop" on a card ends that child, the card says so, and the parent's reply continues (end-to-end test).
- The toolbar's "Child agents" button says how many child agents work, lists the conversation's child agents, and opens a child's conversation in the side panel; closing the panel returns the focus to the button (end-to-end test).
- Every action works from the keyboard, including resizing the side panel (end-to-end test).
- Axe finds no WCAG 2.1 AA violation in the cards, the requests inside them, the side panel and the settings, in the light and dark themes.
- In the pseudo-locale, every piece of text outside content comes from the catalog, and no layout overflows.
