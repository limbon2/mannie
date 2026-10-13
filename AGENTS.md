# AGENTS.md

Instructions for coding agents working in this repository, and for the people who review their work. This file is the single source of agent rules, and `CLAUDE.md` only imports it.

## Project

- What: Mannie is a desktop AI agent harness for one owner. It chats with AI models, runs system tools on the owner's machine under approvals, and spreads work across machines through workers that own their hosts. The core is Rust. The desktop app is Tauri 2 with a SvelteKit UI built from shadcn-svelte components, and the worker is a headless service for Windows, Linux and FreeBSD.
- Phase: documentation baseline. There is no application code yet, and [M0 (Foundation)](docs/roadmap/milestones.md#m0-foundation) is next.
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
- Contracts and stored data change only by addition, with a deprecation period before removal ([ADR-0002](docs/adr/0002-grow-through-extension-points.md)).
- New dependencies must pass the license policy ([ADR-0017](docs/adr/0017-license-and-dependency-policy.md)).
- All visible text comes from the message catalogs in `apps/desktop/messages/` ([ADR-0019](docs/adr/0019-keep-visible-text-in-message-catalogs.md)). Never write UI text as a literal in a component. Add a message under your feature's key prefix and call its generated function. The core sends codes and arguments, never display text.

## UI components: shadcn-svelte

This rule exists because coding agents keep rebuilding shadcn components from memory. Follow it exactly ([ADR-0006](docs/adr/0006-use-shadcn-svelte-through-its-cli.md)):

- Add components only with the pinned CLI, from `apps/desktop`: `pnpm shadcn-svelte add <name>`. Never write a registry component by hand, paste one from another project, or port one from the React version of shadcn/ui.
- Before you use or add a component, read its documentation page, `https://shadcn-svelte.com/docs/components/<name>.md`. The index of all pages is `https://shadcn-svelte.com/llms.txt`. Then check `apps/desktop/components.json` and `apps/desktop/src/lib/components/ui/` for what is already installed.
- Use the shadcn-svelte agent skill once M0 has installed it in the repository.
- Do not edit files in `apps/desktop/src/lib/components/ui/`. Customize through the theme tokens and through components in `apps/desktop/src/lib/components/app/`. If a change inside `ui/` is unavoidable, record it with its reason in `apps/desktop/src/lib/components/ui/PATCHES.md`.
- When shadcn-svelte has no matching component, compose one in `apps/desktop/src/lib/components/app/` from installed components and bits-ui primitives. Read the bits-ui documentation first: `https://bits-ui.com/llms.txt` lists a page for every primitive.
- Update components only in a dedicated change: bump the pinned CLI, re-add the components with `--overwrite`, review the diff and apply the registered patches again.
- Icons come from `@lucide/svelte`.

## Svelte

- Use Svelte 5 runes only. Do not use legacy syntax: `export let`, `$:` statements, `on:` event directives, or stores for new state. Use `$app/state`, not `$app/stores`.
- Look up Svelte and SvelteKit APIs in the official documentation through the Svelte MCP server (`@sveltejs/mcp`: `list-sections`, then `get-documentation`) instead of relying on memory. M0 adds its configuration to the repository.
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

`Taskfile.yml` defines every workflow, and `task --list` shows them. Today the tasks are:

- `task setup` installs the pinned tooling from the lockfile;
- `task fmt` formats and sorts all code and documentation (ESLint fixes, Prettier, Tombi), and `task fmt:check` checks it;
- `task lint` runs ESLint (`lint:ts`) and Tombi's TOML lint (`lint:toml`);
- `task test:scripts` runs the tests of the repository scripts, such as the local ESLint rules;
- `task docs:fmt` formats the documentation, and `task docs:check` checks its formatting, Markdown rules, spelling, links and consistency;
- `task ci` runs `docs:check`, `fmt:check`, `lint` and `test:scripts`, the checks every change must pass.

The Rust tasks `fmt:rust`, `fmt:check:rust` and `lint:rust` already exist. They join `fmt`, `fmt:check` and `lint` with the first crate in M0, which also adds `gen`, `dev`, `build`, `test`, `e2e` and `release:prepare` ([repository](docs/engineering/repository.md#7-tasks)). Update this section when it does.

## Environment

- The owner's workstation runs Windows 11, and the shell is PowerShell. Development also happens on Linux, and FreeBSD builds of the worker run in CI.
- Installed tools: Node.js 24, pnpm 11.8, Task 3.40 and Git. Rust is not installed yet; M0 adds `rust-toolchain.toml` and the setup steps.
- Task commands call real executables only, never Unix utilities such as `rm` or `mkdir`, which Windows lacks.
- Third-party facts in the documentation were verified on 2026-10-13. Check them again before relying on version-specific behavior.
