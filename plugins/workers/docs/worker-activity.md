# Worker activity

|              |                                                                                                                                                                                                                                                                                                                                                                                           |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                                  |
| Milestone    | M4                                                                                                                                                                                                                                                                                                                                                                                        |
| Last updated | 2026-10-21                                                                                                                                                                                                                                                                                                                                                                                |
| Part of      | [Workers](README.md)                                                                                                                                                                                                                                                                                                                                                                      |
| Related      | [ADR-0027](../../../docs/adr/0027-delegate-tasks-to-workers-through-a-runtime-port.md), [ADR-0020](../../../docs/adr/0020-complete-the-ui-shell-slots.md), [ADR-0025](../../../docs/adr/0025-add-side-panels-to-the-shell.md), [Child agent activity](../../../docs/features/child-agents/child-agent-activity.md), [Tool activity](../../../docs/features/system-tools/tool-activity.md) |

## Design

### Workers in the sidebar

Once a worker is paired, the sidebar shows a "Workers" group between the projects and the conversations, with one row for each worker: its name, as content, and its status as an icon with a text for screen readers: "Connected", "Connecting", "Offline", "Needs an update", "Key changed" or "No longer paired". A row's menu offers "Details", which opens the worker in the Workers settings, "Rename…" and "Remove…". The group's "+" opens "Pair a worker". Without paired workers the group is not shown, so the sidebar of an owner without workers stays as it was.

### Pairing a worker

"Pair a worker…" in the Workers settings, in the command palette and in the sidebar's group opens a dialog that says to enter the address and code that `mannie-worker pair` prints on the worker's host. It opens in the Address field. Each field's help sits behind a "(?)" button beside its label:

- "Address", with the example `build-01.local:7445`: the host's name or address, with the port when it is not 7445.
- "Code": the code that `mannie-worker pair` printed, which works once, for 10 minutes.
- "Name", optional: the name that Mannie and the conversation agent use for the worker; empty keeps the name in the worker's configuration ([naming a worker](#naming-a-worker)).
- "Pair" pairs and closes the dialog, and the worker appears with its name; while it pairs, the dialog says "Pairing…" through a status region. A refusal shows in the dialog, with what the owner can do: check the code, run `mannie-worker pair` again for a new one, check the address and the firewall, reset a worker that another app paired, or choose a shorter name or one that no other worker has. The master checks the name before it contacts the worker, so a refused name leaves the code unused.

### Naming a worker

A worker's name in Mannie is the owner's name for it, or else the name in its configuration ([host and ceiling](host-and-ceiling.md#name-purpose-and-host-instructions)). The sidebar, the Workers settings, the dialogs, the cards of new tasks and their requests, and the conversation agent's list of workers and `delegate_task` all use it. A task keeps the name it started with, so earlier cards show the name of their time.

"Rename…" in the worker's menu and in the Workers settings opens a dialog with the name in the worker's configuration, as content, and a "Name" field that holds the owner's name, or nothing. "Save" stores the name and closes the dialog, and an empty name goes back to the configured one. A name has at most 64 characters, line breaks become spaces, and the master refuses a name that another worker shows, ignoring case, so the owner and the conversation agent can tell the workers apart. Workers whose configured names are the same still pair, as before, and the owner can rename one of them.

### Removing a worker

"Remove…" asks first, in a dialog a little wider than the default for its long explanation: the worker forgets this app and cancels its tasks; a worker that is offline keeps its pairing until the owner runs `mannie-worker pair --reset` on its host. "Remove" removes it, and the conversations of its tasks stay in their parents, unreadable.

### The Workers settings

The section "Workers", with the ID `workers.settings`, after the Child agents section, lists the paired workers with their status. Each worker shows its address, its version, and, on demand, its purpose, its system, shells and tools, its ceiling (the mode and the allowed folders) and how many tasks it runs at once, all as content, and the first 12 characters of its key's pin, with "Rename…" and "Remove…". A worker that needs an update says which version to install, and that the owner updates it on its host with `mannie-worker update`. Without workers, the section explains how to install and pair one, with "Pair a worker…".

### A worker's task in the conversation

The card of a task ([child agent activity](../../../docs/features/child-agents/child-agent-activity.md#the-card-of-a-task)) shows a worker task with the worker's name, as content, in place of the kind of agent, and otherwise as a child's: its number and title, its state, its activity and its tool calls, "Open conversation", "Stop" and "Details". A task that the master let go when Mannie stopped says so, and its conversation shows what happened since.

The toolbar's "Child agents" button lists only child agents. The status line counts worker tasks among the tasks that a reply waits for: "Waiting for 1 worker…", or "Waiting for 3 tasks…" when children and workers are both among them.

### A worker's approval in the conversation

While the worker agent waits for the owner, the card shows its call below the card's own lines, as a child's call, with the host's name as the card shows it: "On {worker}". It offers "Allow once" and "Deny". The decision goes to the worker over the link; while the worker cannot be reached, the request says so and the decision waits for the owner to try again.

### The worker's conversation in a side panel

"Open conversation" opens the task's conversation in the side panel, as a child's: read-only, live, with its tool calls, their details and its status line, and its earlier turns as the owner scrolls up. The cards of its calls of `start_command` show their commands' states; Stop and the output panel are offered only for the master's own commands, so the owner stops a worker's command by stopping its task ([background commands](../../../docs/features/system-tools/background-commands.md)). Its footer says that the conversation belongs to a task on the worker, with "Stop" while the task runs. A run that waits for its lease shows "Waiting for the provider's credential…". While the worker cannot be reached, the panel says so, with "Try again".

### Message keys

The feature's components use keys under `workers_`. The states that chat shows for the runtime (the read-only footer of a worker task, the activity `waiting_for_lease`, the failure `worker_unreachable` and the status line's count) use keys under `chat_`, the card's worker label and the button's list keys under `child_agents_`, and the host and the ceiling's reasons and refusals in tool calls keys under `tools_`.

### Loading, empty and error states

| View                         | Loading                         | Empty                                                         | Error                                                                                        |
| ---------------------------- | ------------------------------- | ------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| Workers in the sidebar       | Nothing until the list is known | The group is not shown                                        | The group shows what failed and "Try again"                                                  |
| Workers settings             | Skeleton rows                   | How to install and pair a worker, with "Pair a worker…"       | What failed and "Try again"                                                                  |
| Pairing dialog               | "Pairing…"                      | Not possible: the fields start empty                          | The refusal and what to do                                                                   |
| Rename dialog                | Not possible: it opens at once  | An empty name keeps the configured name                       | The refusal under the field                                                                  |
| A worker task's conversation | Skeleton turns                  | Not possible: a task conversation starts with its instruction | The worker cannot be reached, with "Try again"; a deleted conversation says so, with "Close" |

### Threat notes

- T-3: a worker's name, the owner's or the configured one, purpose, host facts and folders, a task's title, activity, instruction and report, and a worker's conversation are content. They render as text in elements marked `data-content`, and a report as Markdown through Mannie's own renderer.
- T-23: a worker's request shows inside its task's card with the task's number, title and the host's name, and its buttons belong to the request's own layout.

## Acceptance

- The pairing dialog opens in the Address field, and a field's help shows when its "(?)" button gets the keyboard focus (end-to-end test).
- Pairing with a wrong code shows the refusal, and with a good code adds the worker under the owner's name, which shows as connected in the sidebar and the settings (end-to-end test with a real worker).
- A delegated task names the worker by the owner's name, and shows its card and its request with that name; its conversation opens live in the side panel, an approval in the card runs the operation on the worker with "Allow once", and "Stop" ends the task (end-to-end test).
- An empty name in "Rename…" goes back to the configured name (end-to-end test); a name that is too long or that another worker shows is refused before the code is used, and a `Hello` keeps the owner's name (link tests).
- A worker that stops shows as offline, its task's conversation says that it cannot be reached, and the worker shows as connected again when it comes back (end-to-end test).
- Removing a worker asks first and removes it (end-to-end test).
- Every action works from the keyboard, and axe finds no WCAG 2.1 AA violation in the sidebar's group, the dialogs, the settings, a worker task's card with its request and its side panel, in the light and dark themes.
- In the pseudo-locale, every piece of text outside content comes from the catalog, and no layout overflows.
