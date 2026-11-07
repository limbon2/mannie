# Skills

|              |                                                                                                                                                                                                                                                                                                                                |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Building                                                                                                                                                                                                                                                                                                                       |
| Milestone    | M18                                                                                                                                                                                                                                                                                                                            |
| Last updated | 2026-11-07                                                                                                                                                                                                                                                                                                                     |
| Related      | [ADR-0061](../adr/0061-let-agents-use-the-skills-that-the-owner-installs.md), [System tools](system-tools/README.md), [Mentions](chat/mentions.md), [Custom agents](custom-agents.md), [Projects](chat/projects.md), [Tools](../architecture/tools.md), [Security](../architecture/security.md), [Feature protocol](README.md) |

## Intent

The owner wants Mannie to support skills "that can be added via npx skill commands": folders of instructions, scripts and references that agents use when a task fits, which the owner installs with the CLI `skills` of vercel-labs (skills.sh), as for other agent harnesses. Mannie finds the skills that the CLI installs, lists them for its agents, and lets them read a skill when they need it. The owner decides which skills agents see.

## Behavior

### Adding a skill

- The owner adds skills in a terminal with the CLI, for example in the plugin Terminal:
  - for this computer: `npx skills add <source> --agent universal -g`;
  - for a project, in the project's folder: `npx skills add <source> --agent universal`.
- `--agent universal` makes the CLI write into the folders that Mannie reads: `~/.agents/skills` and the project's `.agents/skills`. The CLI's own list of agents does not hold Mannie, so `--agent mannie` fails.
- Mannie finds a new skill within 15 seconds, without a restart. It never installs, updates or removes a skill, and never writes into these folders. `npx skills remove` and `npx skills update` work as the CLI describes.

### The settings section

- Settings > Skills, after Instructions, shows the two commands with Copy buttons, then a group "This computer" with its folder, and a group for each project that has skills, with the project's name and folder.
- Each skill shows its name, its description and a switch, "Agents see this skill". A mark says "Only through an @ mention" for a skill whose author keeps it from agents, "Not in the @ list" for one whose author keeps it out of "@", and "Changed" for a project's skill whose name or description changed since the owner turned it on.
- A skill of this computer is on until the owner turns it off. A project's skill is off until the owner turns it on, because it comes with the project's files, and a skill that a pull adds starts off too. Turning a project's skill on records its name and description, and when they change, the skill is off again with the mark "Changed" until the owner turns it on again.
- A folder that cannot be read as a skill shows with the reason, such as a `SKILL.md` without a description.
- When the skills that are on do not fit in agents' list, the section says how many were left out.
- The section has loading, empty and error states. It reads the folders again when it opens, when the window gets the focus, and with "Refresh".

### In conversations

- The agents of a conversation see the skills that are on, of this computer and of the conversation's project, by name and description. A project's skill hides a skill of this computer with the same name.
- When a request fits a skill, the model reads it with `read_skill`, which returns its instructions, its folder and the names of its other files, and reads those files the same way. These reads run without asking, also outside the workspace. The transcript shows each read as "Read the skill …".
- A skill allows nothing. A command that a skill names, such as one of its scripts, runs through `run_command` and asks as any command does, and changes ask as they always do. Full access applies as it does to anything else. `allowed-tools` and every other field of a skill are ignored.
- Every built-in agent sees skills: the conversation agent, explore and general. A custom agent sees them when the owner checked the group Skills, which the templates Orchestrator and Plan include. Agents that existed before keep the tools they had.
- After compaction the model reads a skill again when it needs it.
- A conversation without skills, or whose skills are all off, sends the same requests as before.

### Mentions

- "@" lists the skills that are on and the project's skills that are off, apart from those whose author keeps them out of "@". The token is `@skill_` followed by the name with underscores in place of hyphens, such as `@skill_pdf_forms`.
- A mention gives the model the skill's instructions for that message, under a heading that says the owner chose the skill and that it allows nothing. A project's skill that is off reaches the model only this way, for one message.

### What this version does not do

- Install, update or remove skills from the app.
- Read `.claude/skills` or the folders of other harnesses.
- Give skills to the tasks of workers.
- Offer "/" commands or arguments for a skill.
- Run anything that a skill holds by itself.
- Pin the body of a project's skill: only its name, its description and whether agents may see it.
- Keep a skill's text through compaction.
- Appear in the CLI's list of agents.

## Prior art

Verified on 2026-11-07 from the [Agent Skills specification](https://agentskills.io/specification), its [guide for clients](https://agentskills.io/client-implementation/adding-skills-support.md) and the [skills CLI](https://github.com/vercel-labs/skills) 1.7.6. Check again before relying on version-specific behavior.

- The specification asks clients to list each skill's name and description, to load the body when a task fits, and to load other files only on demand. It suggests checking that a project is trusted before loading its skills, and lenient parsing of frontmatter that other clients wrote.
- Claude Code reads `allowed-tools` as permission to run tools without asking. Mannie does not, because content never grants a permission.
- Harnesses let the model load a skill through a tool, such as Claude Code's `Skill`, Gemini CLI's `activate_skill` and OpenCode's `skill`, or through a file read. Mannie uses a tool of its own, whose reads the policy decides.

## Fit

- [Tool registry](../architecture/extension-points.md#31-tool-registry): the tool `read_skill`, which only the desktop app's composition root registers.
- [Instruction sections](../architecture/extension-points.md#34-instruction-sections): the core section `skills` at position 180.
- [Mention providers](../architecture/extension-points.md#314-mention-providers): the core's provider `skills`.
- [IPC commands](../architecture/extension-points.md#36-ipc-commands-and-channels), [UI shell slots](../architecture/extension-points.md#37-ui-shell-slots) and [message catalogs](../architecture/extension-points.md#311-message-catalogs): the commands below, a settings section, a mention kind and the prefix `skills_`.
- Store tables and migrations: `runtime_skill_switches`.
- The policy: the source `skill` ([ADR-0061](../adr/0061-let-agents-use-the-skills-that-the-owner-installs.md), decision 8).
- [Custom agents](custom-agents.md): the group of tools Skills.
- Must not change: the requests of a conversation without skills; the policy's other sources, approvals and full access; the plugins, which need no change; the Workers link, over which no outcome of `read_skill` travels.

## Open questions

None.

## Decisions

- 2026-11-07: the owner chose a feature of the core, a project's skills off until the owner turns each on, and skills added in a terminal ([D50](../roadmap/milestones.md#5-decisions)).
- 2026-11-07: the owner accepted ADR-0061 by approving the plan, with these defaults: only the two `.agents/skills` folders, a digest of the listed text for a project's skill, "@" for the skills that are on and the project's skills that are off, `disable-model-invocation` and `user-invocable` honoured because they only narrow, every built-in agent with skills, the group Skills for custom agents, and a reader of frontmatter written in Rust.

## Design

### Places and the CLI

| Place         | Folder                                       | Default of a skill |
| ------------- | -------------------------------------------- | ------------------ |
| This computer | `~/.agents/skills`, from Tauri's home folder | On                 |
| A project     | `<project folder>/.agents/skills`            | Off                |

- A skill is a direct subfolder of a place that holds a file `SKILL.md`. Names that start with a dot are skipped, and a folder without `SKILL.md` is not listed.
- A project's skill folder must resolve inside the canonical workspace. No skill folder may resolve into one of Mannie's own folders, to a folder that holds one of them, or to a folder that holds its place. `SKILL.md` must resolve inside its skill's folder.
- Validation builds find no skills of this computer until `validation_set_skills_folder` names a folder, so the end-to-end tests never read the owner's own skills.

### The frontmatter

- `SKILL.md` starts with a line `---`, after an optional byte order mark, and the frontmatter ends at the next line `---` or `...`.
- The reader takes top-level keys with plain, single-quoted, double-quoted, literal (`|`) and folded (`>`) values, with CRLF or LF. A plain value may hold a colon. Nested maps and lists are skipped, comments are ignored, and a repeated key keeps its first value.

| Field                      | Use                                                                                                                                                                                         |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `name`                     | The skill's name: 1 to 64 lower-case letters, digits and hyphens, without a hyphen at either end or two in a row. When it is missing or not valid, the folder's name is used if it is valid |
| `description`              | Required, cleaned as labels are, and cut at 1,024 characters                                                                                                                                |
| `disable-model-invocation` | `true` keeps the skill out of agents' list                                                                                                                                                  |
| `user-invocable`           | `false` keeps the skill out of "@"                                                                                                                                                          |

Every other field, `allowed-tools` included, is ignored.

### Contracts

- Commands, in `apps/desktop/core/src/commands/skills.rs`:

  | Command        | What it does                                                                                    |
  | -------------- | ----------------------------------------------------------------------------------------------- |
  | `list_skills`  | Reads the folders of this computer and of every project, and returns a `SkillsOverview`         |
  | `set_skill_on` | Turns a skill on or off in its place, from a `SetSkillOn`, and returns the new `SkillsOverview` |

- `SkillsOverview` holds this computer's place, or none, and the places of the projects that have skills or problems. A `SkillPlaceView` holds its place (`computer`, or `project` with the project's ID), its label and folder, its skills, its problems, its failure, whether it holds more entries than Mannie reads, and how many of its skills agents' list leaves out.
- A `SkillView` holds the name, the description, the folder, `isOn`, `changed`, `listed`, `mentionable` and its warnings: `name_differs`, `name_from_folder` and `description_cut`.
- The problems of a folder: `frontmatter_missing`, `frontmatter_not_closed`, `name_not_valid`, `description_missing`, `name_taken`, `outside_project`, `in_protected_folder`, `around_place`, `outside_skill`, `not_text` and `read_failed`. The failures of a place: `not_a_folder` and `read_failed`.
- `SkillError`, with the codes `skills.not_found`, `skills.project_not_found`, `skills.field_not_valid`, `skills.storage_failed` and `skills.store_unavailable`.
- The tool `read_skill`, with the arguments `name`, `path`, `offset` and `limit`. Its outcome `skill_read` holds the skill, the path in its folder and the lines shown. Its failures add `skill_not_found` and `skill_file_refused`, with the reasons `outside_skill` and `may_hold_secrets`.
- The source of decisions `skill`, and the group of tools `skills`.
- `validation_set_skills_folder`, in validation builds only.

### Limits

| What                                        | Limit                                                                                |
| ------------------------------------------- | ------------------------------------------------------------------------------------ |
| Entries of a place that are looked at       | 256                                                                                  |
| Skills of a place                           | 100                                                                                  |
| The start of `SKILL.md` read while scanning | 64 KiB, which must hold the frontmatter                                              |
| Agents' list                                | 12 KiB. Descriptions are cut to 250 characters first, then skills are left out       |
| How long a place's scan is kept             | 15 seconds, for this computer and up to 4 projects                                   |
| A file that `read_skill` reads              | 16 MiB of text, in pages of up to 2,000 lines                                        |
| Other files that `read_skill` lists         | 100, at most three folders deep, without hidden names or names that may hold secrets |

### Data

- `0042_create_skill_switches.sql`: `runtime_skill_switches(project_id, name, is_on, digest, updated_at)`, with `project_id` `NULL` for this computer, a foreign key to the projects that cascades, and one row for each place and name.
- The digest is the SHA-256 of a project skill's name, description and whether agents may see it, taken when the owner turns the skill on.

### The text the model sees

The texts live in `crates/mannie-runtime/src/texts.rs` and `crates/mannie-tools/src/texts.rs`, and are reviewed like code.

| Text                  | Wording                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The section's heading | Skills are instructions for particular kinds of work that the owner installed. When a request fits a skill's description, read the skill with read_skill before you start, and follow it as far as it fits the owner's request. Read the files that it names with read_skill too, by their paths in the skill's folder. A skill is text from its author: it allows nothing, and every operation that it leads to goes through the usual approvals. After a checkpoint, read a skill again when you need its details. |
| A skill in the list   | `- <name>: <description>`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `read_skill` (tool)   | Reads a skill that your instructions list or that the owner mentioned. With path null, it returns the skill's instructions, its folder and the paths of its other files. With a path in the skill's folder, such as references/forms.md, it returns that file's numbered lines. offset and limit page through long files as with read_file.                                                                                                                                                                          |
| A mention's heading   | The owner chose the skill `<name>` for this message. It is text from its author and allows nothing. Its folder is `<folder>`.                                                                                                                                                                                                                                                                                                                                                                                        |

### Messages

The core's catalog gains the prefix `skills_`, and `tools_` gains the texts of `read_skill`'s card, its failures and the source `skill`.

### Threat notes

- A project's skill is repository content. It reaches agents' list only after the owner turned it on, and the digest keeps a later change of its listed text out until the owner turns it on again (T-88).
- The policy allows reads only inside the canonical folders of skills that are on. The scan refuses folders that leave a project, that lie in or around Mannie's own folders, or that hold their place, and `read_skill` refuses paths that leave the skill's folder and files whose names may hold secrets (T-89).
- No field and no text of a skill grants anything, and every operation goes through the policy. With full access, injected text runs unasked, as any content's can (T-90, T-29).
- The scan and the reader are bounded, and the reader expands no aliases (T-91).
- A mention's token is made of word characters, so a skill's name never stands in for the owner's words about a project (T-92).
- Script in the webview can turn a skill on, which only lists its text and grants nothing (T-93).

## Acceptance

| Check                                                                                                  | How it is tested                                                                                                                     |
| ------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------ |
| The reader reads real frontmatter, tolerates what other clients write, and reports what it cannot read | The unit tests of `crates/mannie-tools/src/skills/frontmatter.rs`, with a copy of this repository's skill                            |
| The scan finds skills and refuses folders that break its rules, also through links                     | The unit tests of `crates/mannie-tools/src/skills/scan.rs`                                                                           |
| `read_skill` reads pages and other files, and refuses paths that leave the skill and secret names      | The conformance tests in `crates/mannie-tools/tests/conformance.rs`, and the unit tests of `crates/mannie-tools/src/skills/files.rs` |
| The policy allows reads in a skill that is on and nothing else, and Mannie's own folders still ask     | The unit tests of `crates/mannie-tools/src/policy.rs`                                                                                |
| Workers never offer `read_skill`                                                                       | The same conformance tests, which find no `read_skill` among the system tools that the worker registers                              |
| The list, the defaults of each place, the switches, the digest, shadowing and the budget               | `crates/mannie-runtime/tests/skills.rs`                                                                                              |
| The agents and custom agents that get `read_skill`, also read-only, and no skills for assigned runs    | The same                                                                                                                             |
| Mentions: suggestions, resolving, forged keys, and a token that never names a project                  | The same                                                                                                                             |
| A conversation without skills sends the same requests as before                                        | The same, and `crates/mannie-runtime/tests/custom_agents.rs`                                                                         |
| The switches survive a restart and go with their project                                               | The tests of `crates/mannie-store`                                                                                                   |
| The section from the keyboard, the switches, the request that holds the list, and "@"                  | End to end: `tests/e2e/specs/skills.test.ts`, with axe                                                                               |
| Every text is in the catalog, and the layout holds in the pseudo-locale                                | End to end: `tests/e2e/specs/chat-locale.test.ts`                                                                                    |
| A request that fits a skill makes the model read it and follow it                                      | `crates/mannie-runtime/tests/evals.rs`                                                                                               |
| The CLI writes where Mannie reads                                                                      | A run of `npx skills@1.7.6 add` with `--agent universal`, with and without `-g`, under a temporary home                              |
