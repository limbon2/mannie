# AGENTS.md

Instructions for coding agents working in this repository, and for the people who review their work. This file is the single source of agent rules, and `CLAUDE.md` only imports it.

## Project

- What: Mannie is a desktop AI agent harness for one owner. It chats with AI models, runs system tools on the owner's machine under approvals, and spreads work across machines through workers that own their hosts. Optional features are plugins that the owner turns on and off, with workers as the first. The core is Rust. The desktop app is Tauri 2 with a SvelteKit UI built from shadcn-svelte components, and the worker is a headless service for Windows, Linux and FreeBSD.
- Phase: [M5 (Plugins)](docs/roadmap/milestones.md#m5-plugins) is being built: Mannie becomes a core and plugins ([ADR-0030](docs/adr/0030-split-mannie-into-a-core-and-plugins.md)), and workers become the first plugin, in `plugins/workers/` ([ADR-0031](docs/adr/0031-move-workers-into-the-first-plugin.md)), which the owner accepted in [D28](docs/roadmap/milestones.md#5-decisions). [M6 (Mentions)](docs/roadmap/milestones.md#m6-mentions) is being built alongside it and ships with it in v0.3.0 ([D29](docs/roadmap/milestones.md#5-decisions)): "@" in the message box mentions a project's files and folders, and features and plugins add kinds of mentions ([ADR-0032](docs/adr/0032-let-features-and-plugins-add-kinds-of-mentions.md)). [M7 (Images)](docs/roadmap/milestones.md#m7-images) ships with them in v0.3.0 ([D31](docs/roadmap/milestones.md#5-decisions)): the plugin Images, in `plugins/images/`, adds images to the owner's messages through a button, a paste and a drop, and plugins add sources of images ([ADR-0033](docs/adr/0033-let-plugins-attach-images-to-the-owners-messages.md)). New features go into new milestones as the owner accepts them ([later](docs/roadmap/milestones.md#later)). M0 built the platform: the workspace, the desktop shell, the store, credentials, logging, the worker skeleton, CI and the release pipeline. M1 built [chat](docs/features/chat/README.md) and [window and tray](docs/features/window-and-tray.md), with the `mannie-provider` and `mannie-runtime` crates. M2 built [system tools](docs/features/system-tools/README.md), with the `mannie-tools` crate, and [projects](docs/features/chat/projects.md) in the sidebar. M3 built [child agents](docs/features/child-agents/README.md) and side panels beside the main view ([ADR-0025](docs/adr/0025-add-side-panels-to-the-shell.md)). M2, M3 and M4 shipped together in v0.2.0 on 2026-10-16 ([D23](docs/roadmap/milestones.md#5-decisions)). v0.1.0, prepared for M1, was not published, so v0.2.0 is the first release. M4 built [workers](docs/features/workers/README.md): the `mannie-link` crate and the link's contracts, the worker service on Windows, Linux and FreeBSD, delegation from the runtime ([ADR-0027](docs/adr/0027-delegate-tasks-to-workers-through-a-runtime-port.md)) and tasks under a host's policy ceiling ([ADR-0028](docs/adr/0028-run-assigned-tasks-under-the-host-ceiling.md)). Its checks ran on Windows, Linux and FreeBSD 15.1 ([M4 validation results](docs/roadmap/m4-validation.md)), and the owner accepted every Proposed ADR ([D25](docs/roadmap/milestones.md#5-decisions)).
- Read first: [docs/README.md](docs/README.md). Architecture decisions are in [docs/adr/](docs/adr/README.md), and the owner's decisions are in the [decision record](docs/roadmap/milestones.md#5-decisions).

## Precedence

Apply the rules below to the parts of the repository that a task touches. When rules conflict, the higher item wins:

1. Explicit instructions from the owner for the current task.
2. Accepted ADRs in `docs/adr/`. If an instruction conflicts with one, say so. The change needs a superseding ADR ([template](docs/adr/0000-template.md)), never a silent deviation.
3. The [engineering standards](docs/engineering/standards.md), including the Definition of Done, and the feature documents in `docs/features/`.
4. The rest of this file.

## New requests

The owner often describes what they want informally. Before writing code for anything new, follow the [feature protocol](docs/features/README.md):

- Restate the request in plain terms, and check `docs/features/` for a feature it changes.
- Size it. A tweak changes no contract, stored data, dependency, trust boundary or UI surface, and needs only the change, its tests and a changelog line. A feature needs a feature document. An architectural change needs an ADR, and you wait for the owner to approve it.
- Ask only the questions whose answers change the outcome. State your other assumptions in the feature document.
- "Do X" means build it now. "Maybe later we want Y" means record it as an unscheduled feature. Ask when it is unclear which one the owner means.
- Build a feature as a slice through the [extension points](docs/architecture/extension-points.md). If the slice needs edits inside modules that belong to other features, stop: an extension point is missing, and adding one needs an ADR.
- Do not write specification, plan or task documents beyond the feature document.

## Project rules

- Production quality from the first commit ([D2](docs/roadmap/milestones.md#5-decisions)). No prototypes, placeholder implementations, stubbed security checks or temporary bypasses. Every change meets the [Definition of Done](docs/engineering/standards.md#1-definition-of-done).
- The stack is fixed by the ADRs: Tauri 2, Rust, Svelte 5 with SvelteKit, shadcn-svelte, SQLite through rusqlite, and gRPC with tonic for the link. Check [technology stack](docs/architecture/technology-stack.md) before adding a dependency. A significant new dependency needs an ADR, and every new dependency needs a reason in the change.
- The webview is untrusted ([ADR-0004](docs/adr/0004-keep-system-access-in-the-rust-core.md)). Logic, file access, processes, network access and credentials stay in Rust. Do not add Tauri's fs, shell or http plugins. Every command needs a capability grant and validates its input.
- The UI talks to the core only through the generated IPC bindings ([ADR-0007](docs/adr/0007-generate-ipc-bindings-from-rust.md)). Generated code is never committed.
- Every side effect is a journaled, authorized operation ([ADR-0013](docs/adr/0013-journal-every-side-effect.md)). Content that a model reads never grants a permission.
- Provider credentials live only in the master's operating system credential store ([ADR-0010](docs/adr/0010-keep-provider-credentials-on-the-master.md)). Secrets never appear in logs, the database, URLs, command arguments, child process environments, transcripts or error messages.
- Workers own their hosts ([ADR-0015](docs/adr/0015-run-an-agent-runtime-on-every-machine.md)). The master never runs tools on a worker's host; it delegates tasks.
- The core never depends on a plugin, and a plugin never depends on another plugin ([ADR-0030](docs/adr/0030-split-mannie-into-a-core-and-plugins.md)). A plugin's crates, programs and contracts live in `plugins/<id>/`, its UI files in the desktop app's folders carry its ID, and only `registrations/plugin-list.ts` imports them. cargo-deny and the ESLint rule `mannie/plugin-boundaries` check this.
- Contracts and stored data change only by addition, with a deprecation period before removal ([ADR-0002](docs/adr/0002-grow-through-extension-points.md)).
- New dependencies must pass the license policy ([ADR-0017](docs/adr/0017-license-and-dependency-policy.md)).
- All visible text comes from the message catalogs in `apps/desktop/messages/` ([ADR-0019](docs/adr/0019-keep-visible-text-in-message-catalogs.md)). Never write UI text as a literal in a component. Add a message under your feature's key prefix and call its generated function. The core sends codes and arguments, never display text.

## UI components: shadcn-svelte

This rule exists because coding agents keep rebuilding shadcn components from memory. Follow it exactly ([ADR-0006](docs/adr/0006-use-shadcn-svelte-through-its-cli.md)):

- Add components only with the pinned CLI, from `apps/desktop`: `pnpm shadcn-svelte add <name>`. Never write a registry component by hand, paste one from another project, or port one from the React version of shadcn/ui. The agent skill's examples run `shadcn-svelte@latest`; in this repository, run the pinned CLI instead.
- Before you use or add a component, read its documentation page, `https://shadcn-svelte.com/docs/components/<name>.md`. The index of all pages is `https://shadcn-svelte.com/llms.txt`. Then check `apps/desktop/components.json` and `apps/desktop/src/lib/components/ui/` for what is already installed.
- Use the shadcn-svelte agent skill in `.claude/skills/shadcn-svelte/` (Codex: `.agents/skills/shadcn-svelte/`). It is vendored as its authors wrote it; update it with `pnpm dlx skills@1.7.2 add huntabyte/shadcn-svelte --skill shadcn-svelte --agent claude-code codex --copy -y`, which records it in `skills-lock.json`.
- Do not edit files in `apps/desktop/src/lib/components/ui/`. Customize through the theme tokens and through components in `apps/desktop/src/lib/components/app/`. If a change inside `ui/` is unavoidable, record it with its reason in `apps/desktop/src/lib/components/ui/PATCHES.md`, which also lists the components' built-in English text and how the app avoids it.
- When shadcn-svelte has no matching component, compose one in `apps/desktop/src/lib/components/app/` from installed components and bits-ui primitives. Read the bits-ui documentation first: `https://bits-ui.com/llms.txt` lists a page for every primitive.
- Update components only in a dedicated change: bump the pinned CLI, re-add the components with `--overwrite`, review the diff and apply the registered patches again.
- Icons come from `@lucide/svelte`.

## Svelte

- Use Svelte 5 runes only. Do not use legacy syntax: `export let`, `$:` statements, `on:` event directives, or stores for new state. Use `$app/state`, not `$app/stores`.
- Look up Svelte and SvelteKit APIs in the official documentation through the Svelte MCP server (`@sveltejs/mcp`: `list-sections`, then `get-documentation`) instead of relying on memory. `.mcp.json` (Claude Code) and `.codex/config.toml` (Codex) start the pinned server from `node_modules`, so run `task setup` first.
- SvelteKit 3 keeps its configuration in `apps/desktop/vite.config.ts`, and the app imports its own modules through the `#lib/*` subpath import instead of `$lib`.
- Code is sorted by kind. In `apps/desktop/src/lib/`, components go in `components/` (shadcn-svelte's in `ui/`, Mannie's in `app/<area>/`), state in `state/`, helpers in `utils/`, shared types in `types/` and slot registrations in `registrations/` ([desktop app](docs/architecture/desktop-app.md#6-frontend-structure)). The desktop crate's IPC commands go in `apps/desktop/src-tauri/src/commands/`. Put new code beside code of its kind, and name an area's files after it.
- Run `svelte-autofixer` on every new or changed component until it reports no issues.

## Working agreements

### Readable source code

- Readable source code is part of the deliverable. Write for the person who will review, debug and change it. Never minify or compress source by hand.
- Use each language's formatter:

  | Language                       | Formatter                                                                    |
  | ------------------------------ | ---------------------------------------------------------------------------- |
  | Rust                           | rustfmt                                                                      |
  | TypeScript, Svelte, JavaScript | ESLint fixes, which sort imports, type members and attributes, then Prettier |
  | CSS, JSON, YAML, Markdown      | Prettier                                                                     |
  | TOML                           | Tombi                                                                        |
  | Protobuf                       | `buf format`                                                                 |

  `task fmt` runs them in that order.

- Use descriptive names and straightforward control flow. Avoid nested ternaries, dense chains and several side effects in one expression.
- Extract modules around real responsibilities. Avoid speculative abstractions and layers with a single implementation, unless an ADR asks for them.
- Comments explain what the code cannot: security reasoning, ordering requirements, protocol quirks, the ADR behind a choice. Do not write comments that repeat the code.
- Improve the code you touch, but do not reformat unrelated files.

### Scope and implementation

- Keep to the scope of the request and of the current milestone, and do not pull later features forward.
- Read the relevant code, configuration and documentation before assuming anything. Take the build, test and format commands from `Taskfile.yml`.
- Make the smallest coherent change that fully solves the problem. No unrelated refactors, features or dependency upgrades.
- Keep the owner's changes and existing behavior outside the requested scope. Investigate unexpected differences before overwriting or reverting anything.

### Interfaces and visible text

- The app opens straight into the working view. Do not add landing, welcome or onboarding screens, slogans or decorative pages.
- Labels say literally what happens, such as "Stop", "Retry" or "Allow once".
- Every view has loading, empty and error states, and an error says what failed and what the owner can do.
- The UI meets WCAG 2.1 AA and works fully from the keyboard.
- The interface is English for now and must stay ready for other languages: messages from the catalog, dates and numbers through `Intl`, layouts that survive longer text, and logical (start and end) styling in Mannie's own components.

### Writing

Documentation, commit messages and UI text use plain, specific English. Say what is true and stop:

- no em dashes or en dashes as connectors;
- no "not X but Y" framing and no contrasts with claims nobody made;
- no closing lines that repeat the paragraph;
- no bold label on every list item;
- no inflated words such as "crucial", "robust", "seamless" or "pivotal".

Documents follow the conventions in [docs/README.md](docs/README.md): a status table, numbered headings where required, verification dates for third-party facts, and links instead of repetition.

### Verification

- Run `task fmt`, then `task fmt:check`. Never add a suppression just to make a check pass. A justified suppression is narrow and states its reason: `#[expect(lint, reason = "...")]` in Rust, never `#[allow]`, and `// eslint-disable-next-line rule -- reason` in TypeScript and Svelte, never `@ts-ignore`. A suppression that no longer suppresses anything fails the check.
- Run the checks that the change touches. Behavior changes need tests, including the failure paths. A passing build alone does not show that a feature works.
- Documentation changes must pass `task docs:check`. Before reporting a change as done, run `task ci`.
- CI repeats these checks and blocks nothing. A change, a milestone or a release is done when it compiles, builds, runs without runtime errors and its tests pass on the owner's machines ([D24](docs/roadmap/milestones.md#5-decisions)).
- Tell existing failures apart from failures your change introduced.

### Generated files and caches

- Generated code lives only in ignored folders. To change it, edit its source and run the generator.
- Never put tool caches in the repository: the Cargo registry, the pnpm store and the Vite and Vitest caches stay in their default user locations.

### Working and reporting

- Continue authorized work through implementation and verification. Make reasonable low-risk assumptions and state the ones that affect the result.
- Ask when a missing decision would change the outcome materially: a conflict with an ADR, an unclear license, a security trade-off, an action outside the requested scope.
- Report what changed, what was verified and what remains open. Never claim that a check passed unless it ran and passed.

## Where things are

| Need                                   | Document                                                                                               |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| What Mannie is and why                 | [Vision](docs/product/vision.md), [goals and scope](docs/product/goals-and-scope.md)                   |
| How to add a feature                   | [Feature protocol](docs/features/README.md), [extension points](docs/architecture/extension-points.md) |
| Vocabulary and invariants              | [Glossary](docs/glossary.md), [domain model](docs/architecture/domain-model.md)                        |
| How the parts fit                      | [Architecture overview](docs/architecture/overview.md)                                                 |
| Threats and rules for secrets          | [Security](docs/architecture/security.md)                                                              |
| Coding, testing and contract standards | [Engineering standards](docs/engineering/standards.md)                                                 |
| Layout, generated files and tasks      | [Repository](docs/engineering/repository.md)                                                           |
| Milestones, decisions and risks        | [Milestones](docs/roadmap/milestones.md)                                                               |

## Commands

`Taskfile.yml` defines every workflow, and `task --list` shows them. The ones you use most:

- `task setup` installs the pinned pnpm packages, cargo-deny and tauri-driver;
- `task gen` generates the IPC bindings, compiles the message catalogs and writes SvelteKit's types; run it after changing a command, a type that crosses IPC or a message;
- `task dev` runs the desktop app, and `task build` builds the installers and the worker;
- `task fmt` formats everything, and `task fmt:check` checks it;
- `task lint` runs ESLint, svelte-check, tsc, Tombi, Clippy, cargo-deny, the npm license check and the catalog check;
- `task test` runs the Rust, Vitest and script tests;
- `task e2e` builds the validation app and runs the end-to-end tests through tauri-driver (V1, V2, V3, V11);
- `task docs:check` checks the documentation;
- `task ci` runs every check a change must pass;
- `task release:prepare VERSION=x.y.z` prepares a release ([releasing](docs/engineering/releasing.md)).

## Environment

- The owner's workstation runs Windows 11, and the shell is PowerShell. Development also happens on Linux, and FreeBSD builds of the worker run in CI.
- Installed tools: Node.js 24, pnpm 11.8, Task 3.40, Git, and Rust through rustup, which installs the toolchain that `rust-toolchain.toml` pins. [CONTRIBUTING](CONTRIBUTING.md) lists the system packages, such as WebKitGTK on Linux and the WebDriver for the end-to-end tests.
- Task commands call real executables only, never Unix utilities such as `rm` or `mkdir`, which Windows lacks.
- Third-party facts in the documentation were verified on 2026-10-13. Check them again before relying on version-specific behavior.
