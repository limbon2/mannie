# Extension points

|              |                                                                                                                                                                          |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Draft for review                                                                                                                                                         |
| Last updated | 2026-10-15                                                                                                                                                               |
| Related      | [ADR-0002](../adr/0002-grow-through-extension-points.md), [Feature protocol](../features/README.md), [Repository](../engineering/repository.md), [Overview](overview.md) |

Mannie adds features by registration. A feature is a vertical slice: Rust modules, IPC types, UI components, migrations and tests, plus its feature document. The slice plugs into the extension points below and leaves other features alone ([ADR-0002](../adr/0002-grow-through-extension-points.md)).

## 1. Rules for every extension point

1. A feature owns what it registers. Other features reach it only through its public interface or through another extension point.
2. Registration is explicit code in a composition root (`mannie-desktop`, `mannie-worker`) or in a registry module of the UI shell. Nothing is discovered by scanning or loaded at runtime.
3. Contracts grow by addition. A reader that meets a variant it does not know skips it or shows it in a generic form, and does not fail.
4. Each extension point has an owning module and a test that registers a fake entry, so a broken extension point fails its own tests first.
5. A new extension point, or a change to an existing one, needs an ADR. An extension point is added when the first feature needs it.

## 2. Catalog

| Extension point                 | Owning module                 | What a feature registers                                                                            | Introduced in |
| ------------------------------- | ----------------------------- | --------------------------------------------------------------------------------------------------- | ------------- |
| Tool registry                   | `mannie-tools`                | A tool: name, schema, availability, permission class and executor                                   | M2            |
| Agent definitions               | `mannie-runtime`              | An agent definition: instructions, tool set, budgets, requester and default profile                 | M1            |
| Provider adapters               | `mannie-provider`             | An adapter for an API interface, or a quirk profile                                                 | M1            |
| Instruction sections            | `mannie-runtime`              | A section of the system prompt and its position                                                     | M1            |
| Transcript parts and run events | `mannie-types`                | A part type or event variant, with its rendering for the model                                      | M1            |
| IPC commands and channels       | `mannie-desktop`              | Commands and channel message types for the feature's UI                                             | M0            |
| UI shell slots                  | The desktop app's shell       | Navigation entries, views, panels, side panels, conversation controls, settings, actions, renderers | M0            |
| Settings sections               | `mannie-types`                | A typed settings section with defaults and validation                                               | M0            |
| Store tables and migrations     | `mannie-store`                | The feature's tables and its forward-only migrations                                                | M0            |
| Link services                   | `mannie-link`                 | A protobuf package with its feature level                                                           | M4            |
| Delegation port                 | `mannie-runtime`              | The link's way of running a task on another machine, given to the runtime by a composition root     | M4            |
| Message catalogs                | The desktop app's `messages/` | The feature's visible text, under its own key prefix                                                | M0            |

## 3. The extension points

### 3.1 Tool registry

A tool implements the `Tool` trait. It declares its name, a description written for the model, an input schema generated from a Rust type with schemars, its permission class: read-only, or with side effects, and whether a model call is offered it, which can depend on the conversation's project and the owner's projects. Its executor follows the operation lifecycle ([ADR-0013](../adr/0013-journal-every-side-effect.md)): prepare, then execute after authorization. Results are typed values with a rendering for the model. An agent definition names the tools its agent may use, and a run gets those of them that the registry holds, so a tool that a platform's composition root does not register, such as `run_command` on FreeBSD until workers bring it, is not offered there. The registry rejects duplicate names when tools register. [Tools](tools.md) has the contract in full. Tools that act on runs rather than on the machine, such as the task tools of child agents, belong to the runtime and stay outside the registry ([ADR-0024](../adr/0024-run-child-agent-tasks-in-the-parent-run.md)).

### 3.2 Agent definitions

An agent definition names an agent's instructions, its tool set (tool names from the registry), its budgets and its default provider profile. It also says who the agent answers to, the owner or another agent, which chooses the wording of its base instructions, its notes and its denials, and how many tasks a reply may give to child agents. Built-in definitions include the conversation agent, the explore and general child agents, and the worker agent, which only the worker registers, so the master never offers it to `start_task`. A run names the definition it runs with, and a feature that needs a new kind of agent registers a definition. It does not change the loop.

### 3.3 Provider adapters and quirk profiles

An adapter implements the `Provider` trait for one API interface ([ADR-0011](../adr/0011-own-the-provider-adapters.md)). A quirk profile is data: it matches a server or a model and switches documented normalizations on, such as reading tool calls that the model wrote as text. A feature that supports a new provider adds an adapter or a quirk profile, together with recorded fixtures for its tests.

### 3.4 Instruction sections

The system prompt is assembled from sections that modules register, each with a fixed position. Stable sections come first (the base instructions, the agent definition, the tool guidance and the owner's instructions), and sections specific to a host or a session come after them, so that providers can cache the stable prefix. Context that changes from turn to turn goes into the latest user message instead of the system prompt. A section can depend on what a run is offered: the guidance on tasks reaches only runs that are offered the task tools. The context carries the host of a worker (its name, purpose, host instructions and host facts) and the workers that the master can delegate to, and a composition root can assemble a registry of its own, as the worker does.

### 3.5 Transcript parts and run events

Parts and events are tagged enums in `mannie-types` with a version-tolerant encoding. A feature adds a variant together with its rendering for the model and its renderer in the UI. Code that reads parts handles unknown variants: the model sees a short placeholder, and the UI shows a generic card. An unknown event changes nothing that the UI shows, and the UI moves its cursor past it.

M1 adds the parts of chat: the owner's text, the reply's text, reasoning, and errors, each with a state (streaming, complete, stopped, interrupted, incomplete or discarded) and an audience (the owner, the model or both). Its events cover turns, runs, parts with their deltas, discarded attempts, checkpoints, the context's use and changes to a conversation ([conversations](../features/chat/conversations.md)).

M3 adds the part `task`, which records a call of a task tool together with its task, and the run activity `waiting_for_tasks` ([tasks](../features/child-agents/tasks.md#the-task-part)). Code in the store and the runtime that handles calls treats `task` parts as it treats `tool_call` parts.

M4 adds the worker to a task's section, for tasks that run on a worker, and the run activity `waiting_for_lease` ([delegation](../features/workers/delegation.md#contracts)).

### 3.6 IPC commands and channels

Each feature defines its command arguments, results, errors and channel messages in `mannie-types` and registers its commands with tauri-specta in `mannie-desktop`, whose commands live in `src/commands/`, one module per area ([ADR-0007](../adr/0007-generate-ipc-bindings-from-rust.md)). Every command needs a capability grant ([ADR-0004](../adr/0004-keep-system-access-in-the-rust-core.md)). A changed contract gets a new command, and the old one is deprecated.

### 3.7 UI shell slots

The shell defines typed slots in `apps/desktop/src/lib/state/shell-slots.svelte.ts`, with their entry types in `types/shell-slots.ts` ([ADR-0020](../adr/0020-complete-the-ui-shell-slots.md)):

- navigation entries and views; a view's title can be content, such as a conversation's title, which the pseudo-locale check then skips;
- sidebar panels, below the navigation, such as the projects and the conversation list;
- overlays, mounted once at the shell's root, such as dialogs that several places open;
- settings sections, each a page of the settings that the settings sidebar lists by its icon and title;
- command palette actions, which can say when they are available;
- part renderers, keyed by part type, with a generic card for a type that no feature registered;
- side panels, each a kind of content that opens beside the main view with an argument, such as a conversation's ID, and whose component can close its panel and name its tab; open panels show as tabs, belong to the view that opened them through its address, and get a generic body when no feature registered their kind ([ADR-0025](../adr/0025-add-side-panels-to-the-shell.md));
- conversation controls, each a feature's compact button or menu in a conversation's toolbar, which receives the conversation's summary and the turns that the view has loaded, and renders nothing when it has nothing to offer ([ADR-0026](../adr/0026-add-conversation-controls-back-for-child-agents.md)).

A feature registers its Svelte components in its registration, `src/lib/registrations/<feature>.ts`, and the shell renders whatever is registered. The shell never imports a feature directly. The window has a fixed height, so a view can keep parts of itself in place, such as a message box below a transcript. The frontend is sorted by kind, so the feature's components, state and helpers sit with the others of their kind ([desktop app](desktop-app.md#6-frontend-structure)).

### 3.8 Settings sections

A feature declares a settings section as a Rust type with defaults and validation. The store persists it, the settings show it as a page of its own through the feature's settings slot, and the core validates every change. Settings never contain secrets; a credential is a reference to the credential store.

### 3.9 Store tables and migrations

A feature owns its tables, named with its module's prefix, and the forward-only migrations that create and change them ([ADR-0009](../adr/0009-persist-state-in-sqlite.md)). It reaches other modules' data through their repositories. A destructive change takes two releases: the first adds the new shape and stops using the old one, and the second removes the old shape.

### 3.10 Link services

A feature that needs traffic between the master and workers adds a protobuf package `mannie.link.<service>.v1` under `proto/mannie/link/<service>/v1/` with its own feature level ([ADR-0016](../adr/0016-link-machines-over-pinned-mtls-grpc.md)). The two sides agree on feature levels when they connect, and a feature checks the agreed level before it uses its service. `buf breaking` guards compatibility, and fixture tests guard the JSON documents that travel inside the messages.

### 3.11 Message catalogs

A feature adds its visible text as messages in `apps/desktop/messages/en.json`, with keys under the feature's own prefix, such as `chat_` or `tools_` ([ADR-0019](../adr/0019-keep-visible-text-in-message-catalogs.md)). Paraglide JS compiles the catalogs into typed message functions, so a component that uses a missing key fails the build. Error codes from the core map to messages in the same catalog. A feature never edits another feature's messages, and translations of a feature's messages go into the catalog of each added locale.

### 3.12 Delegation port

The runtime runs a task on another machine through a port, which the master's composition root gives it and `mannie-link` implements ([ADR-0027](../adr/0027-delegate-tasks-to-workers-through-a-runtime-port.md)). The port lists the machines that take tasks and carries what the runtime asks of them: submitting, following and cancelling tasks, delivering credential leases, and reading and deciding in their task conversations. The runtime never depends on the link, and a runtime without the port offers no delegation.

## 4. Anatomy of a feature slice

| Layer            | What the feature adds                                        | Where                                                                                                        |
| ---------------- | ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------ |
| Feature document | Intent, behavior, fit, design and acceptance                 | `docs/features/<slug>.md`, or a folder for a big feature                                                     |
| Rust logic       | A module in the crate that owns the capability               | `crates/<crate>/src/<feature>/`                                                                              |
| Shared types     | IPC types, parts, events and settings                        | `crates/mannie-types/src/<feature>/`                                                                         |
| Registration     | Calls to the registries                                      | The composition roots in `apps/`                                                                             |
| UI               | Components, state and slot registrations                     | `apps/desktop/src/lib/`, sorted by kind: `components/app/<feature>/`, `state/`, `registrations/<feature>.ts` |
| Data             | Tables and migrations                                        | The store's migration list, in the feature's module                                                          |
| Link contract    | A protobuf package, when machines talk                       | `proto/mannie/link/<service>/v1/`                                                                            |
| Visible text     | Messages under the feature's key prefix                      | `apps/desktop/messages/`                                                                                     |
| Tests            | Unit tests beside the code, integration and end-to-end tests | Next to the code and in each crate's `tests/`                                                                |
| Changelog        | One line                                                     | `CHANGELOG.md`                                                                                               |

## 5. When an extension point is missing

If a feature cannot be built without editing modules that belong to other features, an extension point is missing. Stop, and write an ADR that adds the extension point in a general form, with the feature as its first user. Build the extension point, then the feature on top of it. A one-off exception in another feature's code is not an acceptable shortcut.
