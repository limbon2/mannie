# Plugins

|              |                                                                                                                                                                                                                                                                                                                           |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                  |
| Milestone    | M5                                                                                                                                                                                                                                                                                                                        |
| Last updated | 2026-10-16                                                                                                                                                                                                                                                                                                                |
| Related      | [ADR-0030](../adr/0030-split-mannie-into-a-core-and-plugins.md), [ADR-0031](../adr/0031-move-workers-into-the-first-plugin.md), [ADR-0002](../adr/0002-grow-through-extension-points.md), [Workers](workers/README.md), [Workers as a plugin](workers/plugin.md), [Extension points](../architecture/extension-points.md) |

## Intent

The owner wants Mannie's core to hold what every agent harness needs, such as chat, tool use and child agents, and everything else to be a plugin that each owner turns on when they need it. In the owner's words, not everybody needs every feature, so things like a Jira integration, git, a code editor, a terminal, databases and log viewers become plugins that users can enable. The worker system is the first plugin, since only owners with several machines need it. Plugins are built into Mannie for now, and an API or SDK for third-party plugins may follow later.

## Behavior

- Settings has a Plugins page that lists every plugin built into this version of Mannie, each with its name, a sentence on what it adds and a switch. The first version lists one plugin, Workers.
- Turning a plugin on takes effect at once, without restarting Mannie. Its parts of the interface appear, such as the Workers group in the sidebar, its settings page and its commands, and replies that start from then on can use its tools.
- Turning a plugin off also takes effect at once. Its parts of the interface disappear, replies that start from then on no longer get its tools, and its background work ends. When the plugin has work in progress, the switch asks first and says what happens to it. For Workers, running tasks keep running on their workers, and Mannie follows them again when Workers is back on.
- A reply that is already running keeps the tools it started with, whatever the owner switches meanwhile. If it calls a tool of a plugin that was turned off, the call does not run, and the model reads that the plugin is off.
- Turning a plugin off keeps its data: its settings, its records and what it added to conversations. Turning it on again continues with that data. While a plugin is off, conversations still show what it added, read from what was stored, and an action that needs the plugin, such as opening a worker task's conversation, says that the plugin is off and links to the Plugins page.
- While a plugin turns on or off, its switch says so. A plugin that fails to start shows as failed, with what failed and "Try again", and the rest of Mannie keeps working.
- Each switch is the owner's lasting choice. Mannie starts the plugins that are on when it starts, and quitting keeps the choices.
- A new installation starts with every plugin off. After the update from 0.2.0, Workers is on when at least one worker is paired, so an owner who uses workers keeps them, and off otherwise.
- Not included: installing, updating or removing plugins apart from Mannie, which ships them; plugins from anywhere but Mannie's own repository; removing a plugin's data; turning a plugin on for one project or conversation only; plugins on a worker's host.

## Fit

- Follows [ADR-0030](../adr/0030-split-mannie-into-a-core-and-plugins.md) for what a plugin is, how it starts and stops, and where its code lives, and [ADR-0031](../adr/0031-move-workers-into-the-first-plugin.md) for the extension points that let workers become a plugin.
- Extension points: settings sections (the owner's choices), UI shell slots (the Plugins page, and the plugin that each entry belongs to), IPC commands and a channel of plugin states, and messages under the `plugins_` prefix.
- Every later plugin follows this feature: it gets a switch on the Plugins page, starts and stops at once, keeps its data while off, and has a feature document of its own.
- Must not change: the core behaves the same whatever plugins are on, and Workers behaves as it does in 0.2.0 while it is on.

## Open questions

None.

## Decisions

- 2026-10-16: the owner decided that Mannie becomes a core and plugins, that workers are the first plugin, that plugins are internal for now, that turning a plugin on or off takes effect at once, and that each plugin's code lives in `plugins/<id>/` ([D26](../roadmap/milestones.md#5-decisions)).
- 2026-10-16: the owner approved ADR-0030, ADR-0031 and these defaults, which were chosen when shaping, and building started ([D28](../roadmap/milestones.md#5-decisions)):
  - A plugin is on or off for the whole of Mannie.
  - A new installation starts with every plugin off, and the update from 0.2.0 turns Workers on when a worker is paired.
  - Turning a plugin off keeps its data, and nothing in this feature removes it.
  - Turning off a plugin with work in progress asks first.
- 2026-10-16: chosen while building, for the owner's review:
  - A plugin without a stored choice follows its default once, and the host keeps the outcome as the owner's choice, so pairing or removing a worker later never turns Workers on or off by itself.
  - The host starts the plugins that are on in the background after the core's recovery, so a slow plugin never holds up the window.
  - The Plugins page is the last page of the settings.

## Design

### The host and its states

`mannie-plugins` holds the plugin definition, the start context and the host ([ADR-0030](../adr/0030-split-mannie-into-a-core-and-plugins.md), decision 4). The desktop app's composition root lists the plugins, gives each one its grants, and runs the host.

- A plugin's state is `off`, `starting`, `on`, `stopping` or `failed`. The host makes one change at a time, and a change that arrives while another runs waits for it.
- Turning a plugin on stores the choice first, then starts the plugin, whose contributions go to the runtime; a start that fails leaves the plugin `failed` with a code, and the choice stays on, so "Try again" starts it again. Turning it off stores the choice, drops the contributions, then stops the plugin.
- The host's own failure codes are `grant_missing`, for a start that needs a service that the composition root did not grant, and `contributions_refused`, for contributions with a name that is taken. A plugin adds codes of its own, which it words in its UI registration.
- Quitting stops the plugins that are on, after the runtime, and keeps the owner's choices.

### Contracts

- IPC: `subscribe_plugins` follows the list of plugins through a channel, whose messages carry the whole list after every change and end with `lagged` or `shutdown`; `set_plugin_enabled` turns a plugin on or off and returns its view; `plugin_work` counts the tasks of the plugin's executor that have not ended. A plugin's view has its ID, its state, the owner's choice and the code of a failed start. The errors are `unknown_plugin`, `storage_failed`, `store_unavailable` and `shutting_down`.
- The error `plugin_off` names the plugin in chat's and tools' errors, a plugin's own commands return it while the plugin is off, and channels of a plugin end with the reason `plugin_off`.
- A call of a stopped plugin's tool ends with the reason `plugin_off`, and the model reads: Not run: the owner turned off the plugin "workers", which this tool belongs to. Continue without it, or tell the owner. A call for a task of the plugin's executor reads the same with the task's number.

### Data

- The owner's choices are the core's settings section `plugins`: the choice for each plugin by its ID.
- The store records the migrations of each module in `store_modules`, and a database from before M5 keeps its `user_version` in `store_legacy`, so a module that a later version registers still carries over its migrations ([extension points](../architecture/extension-points.md#39-store-tables-and-migrations)).
- The core keeps the executors' columns of `runtime_tasks` and `runtime_conversations` and the outbox `runtime_executor_outbox` ([orchestration](../architecture/orchestration.md#4-task-executors)).

### Interface

- The Plugins page lists the plugins in the composition root's order, each with its name, its description and a switch, which shows "Starting…" or "Stopping…" while the plugin changes. A failed plugin shows what failed with "Try again". Turning off a plugin with work in progress opens a question that says what happens to the work, with "Turn off" and "Cancel".
- The slots show a plugin's entries only while the plugin is on. An address of a view, a settings page or a side panel of a plugin that is off shows that the plugin is off, with "Open the Plugins settings", and a task conversation that cannot be followed while its plugin is off says so and follows again once the plugin is on. A failed Stop or approval answer for a task of a plugin that is off has the same link beside its message.
- Messages: `plugins_` for the Plugins page, its states and errors and the state of a part of a plugin that is off; a plugin's name, description, question and failures under its own prefix.

### Threat notes

- Internal plugins run with the core's rights. A plugin reaches the credential store only in its own entries and provider credentials only through the core's lender, as its grant allows (T-30 in [security](../architecture/security.md#5-threats-and-mitigations)).
- A plugin that is off can act through nothing: its contributions leave first, its tools do not run, its commands refuse and its channels end (T-31).

## Acceptance

- The host starts plugins in order after recovery, keeps a default as the choice, takes a change at once and keeps it, leaves a failed start failed until "Try again", stops the plugins that are on when Mannie quits, and counts a plugin's work (`crates/mannie-plugins/tests/host.rs`).
- A plugin's tool, agent and section reach the runs that start while it is on; a run keeps what it started with and its calls into a stopped plugin do not run; the runtime takes all of a plugin's contributions or none (`crates/mannie-runtime/tests/plugins.rs`).
- The store keeps the records of each module, refuses two modules with one name, carries over the migrations of a module registered after the conversion, and opens a module's store only to its modules (`crates/mannie-store/src/store/tests.rs`).
- The UI's plugin states, the Plugins page's actions, the slots, the composition root and the messages pass their unit tests (`tests/apps/desktop/`), and the lint rule refuses an import of a plugin's UI from the core or another plugin (`tests/scripts/eslint/plugin-boundaries.test.mjs`).
- A new installation lists Workers off with none of its parts; turning it on from the keyboard shows its settings page and its command at once, turning it off hides them, and the address of its settings page then says that it is off and leads to the Plugins page, all without a reload and with axe passing in both themes (`tests/e2e/specs/plugins.test.ts`, on WebView2 and WebKitGTK).
