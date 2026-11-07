# Custom agents

|              |                                                                                                                                                                                                                                                                                                                 |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                        |
| Milestone    | M17                                                                                                                                                                                                                                                                                                             |
| Last updated | 2026-11-07                                                                                                                                                                                                                                                                                                      |
| Related      | [ADR-0059](../adr/0059-let-the-owner-define-agents.md), [Plans](plans.md), [Child agents](child-agents/README.md), [Questions to the owner](questions.md), [Extension points](../architecture/extension-points.md#32-agent-definitions), [Security](../architecture/security.md), [Feature protocol](README.md) |

## Intent

The owner wants to create and edit agents in the app, "easily created and edited by the user instead of writing custom js code". The first one should work like Codex's Ultra, which does little on its own and uses child agents for different tasks, with a list of the agents that are active and done. The second is a [plan agent](plans.md).

## Behavior

### The Agents page

- An entry "Agents" in the activity bar shows a list in the sidebar and an editor in the main area. The list has two groups: conversation agents (Default, Orchestrator, Plan and the owner's) and child agents (Explore, General and the owner's).
- Default, Explore and General show read-only with a button "Duplicate", which starts a new agent from their values.
- "New agent" offers a blank conversation agent, a blank child agent, and the templates Orchestrator and Plan, so a starter that the owner deleted can come back.
- The editor has these fields:
  - The name, and a description that other agents read when they choose a child.
  - The kind, conversation agent or child agent, fixed when the agent is created.
  - "Only reads": the agent's tools only read, and only child agents that only read work for it.
  - The model: a provider profile, or the conversation's for a conversation agent and the parent's for a child.
  - The instructions, which the model reads after Mannie's base instructions.
  - The steps: an ordered list, each with a title, what to do, who does it, "Several at the same time" and "Ask me first". "Move up" and "Move down" change the order.
  - The tools, as groups, and a switch for each plugin with tools.
  - The child agents it may give tasks to: all, or those the owner checks.
  - The limits, under a collapsed heading: tasks per reply, tool rounds and tool calls.
- "Save" stores the agent, which applies from the next reply or task. A reply that is running keeps what it started with. Leaving with changes that are not saved asks first.
- "Delete" asks first. Conversations that use the agent go back to Default, and a child agent's earlier tasks stay in their conversations but take no follow-ups. An agent that named a deleted child no longer offers it, and its editor no longer lists it.

### Choosing an agent

- A picker in the message box's row lists the conversation agents, in a new conversation and in an existing one. A new conversation starts with Default. The choice applies from the next reply.
- Choosing an agent that names a model also sets the conversation's profile, which the owner can still change.
- A message box of a child agent's conversation has no picker, because a child's conversation is read-only.

### The Orchestrator

- A starter that the owner can edit or delete. It reads and searches, gives tasks, asks the owner, and has no tools that change files or run commands, and no plugins' tools. It can still give tasks to workers.
- It may start 50 tasks in one reply. As for every agent, the setting of child agents limits how many work at once.
- Its steps are "Look around" with explore agents, several at the same time, "Make the changes" with general agents, several at the same time and in separate files, and "Check the work" with a general agent. It skips what a request does not need, and answers a simple question directly.

### What an agent can do

- A custom agent can only have less than its base. Its operations go through the policy and ask as anyone's do, and full access applies to it as to anyone. Nothing about an agent grants a permission.
- A custom child agent's tasks show as other tasks do, with the agent's name on the card. It cannot ask the owner or give tasks of its own.
- A custom child agent is also offered to the built-in conversation agent, whose `start_task` then lists it.

This version does not:

- run the steps with an engine, or track or enforce them;
- let child agents give tasks;
- run custom agents on workers;
- offer a checklist of single tools;
- import, export or keep agents per project;
- limit tokens or cost;
- let models edit agents.

## Prior art

Verified on 2026-11-07, from [Codex's documentation](https://developers.openai.com/codex), [a Hacker News discussion of OpenAI's announcement](https://hn.svelte.dev/item/48689338) and [a third-party package that imitates Ultra](https://pi.dev/packages/pi-ultra). Check again before relying on version-specific behavior.

| Harness       | Shape                                                                                                                                                                          |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Codex, custom | A TOML file per agent, with a name, a description and instructions. Settings limit open threads (6 by default) and nesting                                                     |
| Codex, Ultra  | OpenAI says only that it uses child agents for complex work. The third-party package treats it as a delegation policy over the highest reasoning effort, which is an inference |
| Claude Code   | Workflows are scripts that call agents in a fixed order, written in JavaScript                                                                                                 |

Mannie takes the form over files and code, and the delegation policy over an engine, as the owner chose.

## Fit

- [Agent definitions](../architecture/extension-points.md#32-agent-definitions): a stored source of definitions, and the fields `base`, `read_only`, `description`, `plugins`, `children` and `profile` ([ADR-0059](../adr/0059-let-the-owner-define-agents.md)).
- [Instruction sections](../architecture/extension-points.md#34-instruction-sections): the agent's section holds its instructions and steps, and the tasks section lists a roster that is not the default one.
- [Transcript parts and run events](../architecture/extension-points.md#35-transcript-parts-and-run-events): a run failure and a task refusal, by addition.
- [IPC commands](../architecture/extension-points.md#36-ipc-commands-and-channels) and [message catalogs](../architecture/extension-points.md#311-message-catalogs): the commands below and the prefix `agents_`.
- [UI shell slots](../architecture/extension-points.md#37-ui-shell-slots): a navigation entry, a view, a sidebar panel, a side panel and a conversation control with a `draft`.
- Store tables and migrations: `runtime_agents`, and `runtime_conversations.agent_key`.
- Must not change: a conversation with the built-in agent and no custom child sends the same requests as before; the policy, the journal and full access; explore and general; plugins, which need no change; the Workers link.

## Open questions

None.

## Decisions

- 2026-11-07: the owner chose that steps guide the model and that there is no workflow engine.
- 2026-11-07: the owner accepted ADR-0059 by approving the plan (D49), with these defaults: the feature is in the core, a custom child may name its own model, any conversation agent may use `ask_owner`, children still give no tasks, and the editor is a page of its own in the activity bar.

## Design

### Contracts

- `StartConversation.agent_key`, with `with_agent`, and `ConversationSummary.agent_key`, which a summary without an agent leaves out.
- `RunFailure::AgentMissing`, and the `TaskRefusal` values `AgentGone`, and `NotOffered` for a follow-up outside the roster.
- Commands, in `apps/desktop/core/src/commands/agents.rs`:

  | Command                  | What it does                                                                             |
  | ------------------------ | ---------------------------------------------------------------------------------------- |
  | `list_agents`            | The built-in, the plugins' and the owner's agents, each with what the editor shows       |
  | `agent_options`          | The tool groups for each kind, the plugins with tools, and the templates                 |
  | `create_agent`           | Stores a new agent from an `AgentDraft`, and returns its view with its new key           |
  | `update_agent`           | Replaces an owner's agent's draft. Its kind stays                                        |
  | `delete_agent`           | Removes an owner's agent, and sets back the conversations that use it                    |
  | `set_conversation_agent` | Sets a conversation's agent for its next reply, and its profile when the agent names one |

- `AgentError`, with the codes `agents.not_found`, `agents.name_taken`, `agents.field_not_valid`, `agents.builtin`, `agents.kind_changed`, `agents.too_many`, `agents.storage_failed` and `agents.store_unavailable`, and `ChatError::AgentNotFound` for a key that names no conversation agent.
- `ConversationDraft` gains `agentKey` and `chooseAgent`, which a control of the new conversation pane's message box uses.

### Limits

| What                | Limit                                           | Where it is checked |
| ------------------- | ----------------------------------------------- | ------------------- |
| Agents of the owner | 100                                             | The runtime         |
| A name              | 60 characters, not blank, unique                | The types           |
| A description       | 300 characters, not blank for a child           | The types           |
| Instructions        | 32,768 characters, as the owner's instructions  | The types           |
| Steps               | 20                                              | The types           |
| A step's title      | 80 characters, not blank                        | The types           |
| A step's text       | 4,096 characters, not blank                     | The types           |
| Tasks per reply     | 0 to 200, and 0 without the group of task tools | The types           |
| Tool rounds         | 1 to 500                                        | The types           |
| Tool calls          | 1 to 2,000                                      | The types           |

### The tool groups

| Group       | Tools                                                                                                      | For             |
| ----------- | ---------------------------------------------------------------------------------------------------------- | --------------- |
| `read`      | `system_info`, `list_directory`, `file_info`, `find_files`, `search_text`, `read_file`, `read_tool_output` | Both kinds      |
| `change`    | `write_file`, `edit_file`, `create_directory`, `copy_path`, `move_path`, `delete_path`                     | Agents that act |
| `commands`  | `run_command`, `start_command`, `read_command_output`, `stop_command`                                      | Agents that act |
| `questions` | `ask_owner`                                                                                                | Conversation    |
| `tasks`     | `start_task`, `follow_up_task`, `wait_for_tasks`, and the plugins' executors                               | Conversation    |
| `plans`     | `save_plan`                                                                                                | Conversation    |
| `skills`    | `read_skill`, which is offered only where the conversation has skills that are on ([Skills](skills.md))    | Both kinds      |

`move_to_project` comes with every conversation agent that is not read-only.

### Data

- `0039_create_agents.sql`: `runtime_agents(key, document, created_at, updated_at)`.
- `0040_add_conversation_agents.sql`: `runtime_conversations.agent_key`, `NULL` for the built-in agent.
- The settings key `agents` holds which starters the desktop wrote.

`runtime_runs.agent` and `runtime_conversations.task_agent` hold keys as they hold names.

### The text the model sees

The texts live in `crates/mannie-runtime/src/texts.rs` and are reviewed like code.

| Text                                        | Wording                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Orchestrator's instructions                 | You lead the work and child agents do it. Split the owner's request into tasks, give each task to the child agent that suits it, and combine their reports into one answer for the owner. Use your own tools only to read what a report points to, to check a result, or to answer a question that needs no work. Start independent tasks in the same step, so they run at the same time, and give tasks that change files separate files, so they do not overwrite each other.                                                                        |
| Steps' heading                              | Work through these stages in order. Skip a stage that the request does not need, and say which ones you skipped. ("Stage", because the guidance on tasks already uses "step" for one model call.)                                                                                                                                                                                                                                                                                                                                                      |
| A step that a child does                    | `<number>. <title>: <text>` then "Give this to the `<agent>` agent." or "Give this to `<agent>` agents, several at the same time."                                                                                                                                                                                                                                                                                                                                                                                                                     |
| A step that the agent does                  | `<number>. <title>: <text>` then "Do this yourself."                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| A step that asks first                      | Then "Ask the owner with ask_owner before you start this stage."                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| The roster's heading (agent's instructions) | The child agents you can give tasks to, by the name that start_task takes:                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `start_task` (tool), other rosters          | Gives a self-contained task to a child agent on this computer. The child works in a conversation of its own, in this conversation's workspace, and reports back. The owner approves a child's changes and commands the same way as yours. The child knows nothing of this conversation, so the instruction must contain everything it needs. Tasks started in one step run at the same time. With background false, the result is the child's report. With background true, the result gives the task's number, and wait_for_tasks returns the report. |
| `agent` (argument), other rosters           | The child agent to give the task to, one of those your instructions list.                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Tasks guidance without `run_command`        | As the guidance on tasks, with "by reading them, or by giving a child the task of running their tests" in place of "by reading them or running their tests".                                                                                                                                                                                                                                                                                                                                                                                           |
| An agent that vanished (result)             | Not run: the owner deleted this kind of agent, so this task cannot continue. Choose another agent with start_task if you still need the work.                                                                                                                                                                                                                                                                                                                                                                                                          |
| A follow-up outside the roster (result)     | Not run: you cannot give tasks to this child's kind of agent. Choose an agent that your instructions list with start_task if you still need the work.                                                                                                                                                                                                                                                                                                                                                                                                  |

### Messages

The core's catalog gains the prefix `agents_`, and `child_agents_` gains the panel's texts.

### Threat notes

- A custom definition can only narrow what a run is offered, and the policy does not read it, so a definition grants nothing (T-87).
- A read-only agent's guarantee covers each path by which a run could act (T-86).
- A plugin cannot take over a stored agent's key, because `custom.` is refused for plugins (T-85).
- Instructions are the owner's words, stored in the database and sent to the provider as the owner's instructions are. The editor's note says that the model reads them, so they should hold no secrets.

## Acceptance

| Check                                                                                                  | How it is tested                                                                 |
| ------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------- |
| The built-in agent's requests are byte-identical without custom children                               | `crates/mannie-runtime/tests/custom_agents.rs`                                   |
| A custom agent gets only its groups and plugins, and a plugin's section reads its base                 | The same, and the unit tests of `agents.rs` and `plugin_tools.rs`                |
| A read-only agent has no tool with side effects, no executor and no general child, also in full access | The same                                                                         |
| A follow-up outside the roster, and a worker's task on a read-only run, are refused                    | The same                                                                         |
| A plugin's agent named `custom.` is refused                                                            | The unit tests of `agents.rs`                                                    |
| Deleting an agent in a reply, in a queue and before a follow-up never fails internally                 | The same integration test, and each window hears of the change                   |
| A custom child's profile is used, and falls back when deleted or without tools                         | The same                                                                         |
| A wake-up holds for an agent without command tools, and the note names no missing tool                 | The same                                                                         |
| Fields are checked, and a forged kind or key is refused                                                | The unit tests of `mannie-types`, and the runtime's tests                        |
| Upgrades keep every conversation on the built-in agent, and write the starters once                    | The store's tests and `crates/mannie-runtime/tests/custom_agents.rs`             |
| The link's documents stay readable in both directions                                                  | `plugins/workers/crates/mannie-link/tests/compatibility.rs`                      |
| The form's state, its refusals and the steps' moves                                                    | Vitest: `tests/apps/desktop/src/lib/state/agent-form.test.ts`                    |
| Create, edit, duplicate and delete, and the picker in a new and an existing conversation               | End to end: `tests/e2e/specs/custom-agents.test.ts`, from the keyboard, with axe |
| The child agents' panel reads 10,000 parts within 5 s, and input answers within 50 ms (p95) meanwhile  | `task e2e:perf`: `tests/e2e/perf/performance.test.ts`                            |
| Every text is in the catalog, and the layout holds in the pseudo-locale                                | End to end: `tests/e2e/specs/chat-locale.test.ts`                                |
| The Orchestrator gives its work to children and edits no file itself                                   | `crates/mannie-runtime/tests/evals.rs`                                           |
