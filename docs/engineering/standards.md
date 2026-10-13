# Engineering standards

|              |                                                                                                                                                                                        |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                       |
| Last updated | 2026-10-14                                                                                                                                                                             |
| Related      | [Repository](repository.md), [Releasing](releasing.md), [Security](../architecture/security.md), [Extension points](../architecture/extension-points.md), [AGENTS.md](../../AGENTS.md) |

## 1. Definition of Done

Production quality from the first commit (D2) means that every merged change meets all of the following. Early milestones get no exceptions.

| Area          | Done means                                                                                                                                                                                                                          |
| ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Behavior      | The change does what its feature document says, including the failure cases.                                                                                                                                                        |
| Tests         | Unit tests for the logic and integration tests against real dependencies (SQLite, real processes, the link over loopback). UI flows that a feature adds have end-to-end tests. No test is flaky.                                    |
| Security      | Threat notes in the feature document when a trust boundary is touched. Input validated in Rust. No secret in logs, the database, URLs, command arguments or child environments.                                                     |
| Contracts     | IPC, link and tool contracts changed by addition only. Bindings regenerated, and `buf lint` and `buf breaking` pass.                                                                                                                |
| Data          | Migrations are forward-only and tested from the previous release's fixture database.                                                                                                                                                |
| UI            | Components added through the shadcn-svelte CLI. All visible text comes from the message catalog. Keyboard operable, WCAG 2.1 AA, with loading, empty and error states. Checked on WebView2 and WebKitGTK, and in the pseudo-locale. |
| Observability | New paths have tracing spans and structured fields, and errors carry stable codes.                                                                                                                                                  |
| Documentation | The feature document is current, an ADR exists for every architectural change, and the changelog has a line.                                                                                                                        |
| Formatting    | `task fmt:check` and `task lint` pass without new suppressions. A justified suppression is narrow and explained in a comment.                                                                                                       |
| Dependencies  | A new dependency passes the license gate, and the change says why it is needed.                                                                                                                                                     |
| Performance   | No benchmark gets more than 10% worse without a recorded reason.                                                                                                                                                                    |

Nothing reaches `main` as a placeholder: no stubbed security checks, no TODO in place of error handling, no temporary bypass.

## 2. Rust

- Toolchain: the stable release pinned in `rust-toolchain.toml`, the 2024 edition, rustfmt with default settings, and clippy with warnings denied plus a selected set of pedantic lints configured in the workspace.
- `unsafe` is forbidden everywhere except in the platform modules of the process supervisor. Each `unsafe` block there carries a `// SAFETY:` comment that explains why it is sound.
- Structure: library crates hold the logic, and the binaries compose it. Modules follow responsibilities, public APIs stay small, and there is no global mutable state. Dependencies are passed in through constructors.
- Errors: libraries define `thiserror` enums with stable codes. `anyhow` is allowed only in binaries and tests. `unwrap` and `expect` appear only in tests, or for an invariant with a message that explains why it holds. A panic is a bug.
- Async: tokio. No blocking call runs on an async thread; blocking work goes to `spawn_blocking` or a dedicated thread. Cancellation uses cancellation tokens, timeouts are set at the boundaries, and channels are bounded.
- Serialization: serde with explicit field naming. Enums that cross versions accept unknown variants.
- Time and IDs: UTC everywhere, UUIDv7 for IDs, and an injected clock in code that depends on time.
- Secrets: the redacting secret type from `mannie-types`, never a plain `String`.

## 3. TypeScript and Svelte

- TypeScript in strict mode, with no `any` and no non-null assertions.
- Svelte 5 runes only, forced for project files. `$app/state` instead of `$app/stores`. State lives in `.svelte.ts` modules.
- The structure of `apps/desktop/src/lib/` is described in the [desktop app](../architecture/desktop-app.md#6-frontend-structure) document. Code is sorted by kind: components, state, helpers, types and slot registrations each have a folder, and an area's files carry its name. Feature UI reaches the shell only through its slots.
- IPC goes only through the generated bindings. There is no `invoke` with a string command name, and no `fetch`.
- Visible text comes only from the message functions that Paraglide JS generates from `apps/desktop/messages/` ([ADR-0019](../adr/0019-keep-visible-text-in-message-catalogs.md)). Components contain no literal UI text, and dates, numbers and durations go through `Intl`.
- UI components follow [ADR-0006](../adr/0006-use-shadcn-svelte-through-its-cli.md): CLI-added shadcn-svelte components in `components/ui/`, composition in `components/app/`.
- Styling uses Tailwind CSS utilities and the theme tokens. There are no hard-coded colors and no per-feature global stylesheets.
- Tools: ESLint with typescript-eslint and eslint-plugin-svelte; Prettier with prettier-plugin-svelte and prettier-plugin-tailwindcss; `svelte-check` with zero warnings.
- Svelte code is checked against the official Svelte documentation through the Svelte MCP server, and `svelte-autofixer` runs on new and changed components until it reports nothing.

## 4. UI and visible text

- The app opens into the last conversation. There are no landing, welcome or onboarding screens, slogans or decorative pages.
- Labels say literally what happens: "Stop", "Retry", "Allow once", "Deny".
- Errors say what failed and what the owner can do next.
- Every action works from the keyboard, focus is always visible, and the app meets WCAG 2.1 AA. End-to-end tests run axe checks.
- English comes first, and adding a language must not need code changes (D10). Every piece of visible text comes from the message catalog, and the core returns codes with arguments that the UI turns into messages ([ADR-0019](../adr/0019-keep-visible-text-in-message-catalogs.md)).
- Layouts leave room for text that grows by about 40% in translation, and Mannie's own components use logical (start and end) styling where direction matters.
- Density suits a working tool, and spacing comes from the theme tokens.

## 5. Contracts

- IPC commands are named `verb_noun` in Rust, and the generated functions use camelCase. A command takes one argument struct and returns a typed result or a typed error.
- Channel messages are typed and carry sequence numbers.
- Contracts grow by addition ([ADR-0002](../adr/0002-grow-through-extension-points.md)). A changed signature becomes a new command, and the old one is kept for at least one minor release.
- Link contracts follow buf's default lint rules, in packages named `mannie.link.<service>.v1`. Field numbers are never reused, and removed fields are reserved.
- Tool names are `snake_case`. Tool descriptions and other text that the model reads are reviewed like code, because they are a contract with the model.

## 6. Database

- Tables are named `<module>_<entity>` in `snake_case`, plural, and belong to the module that owns them.
- IDs are UUIDv7 strings. Timestamps are integers in UTC milliseconds. JSON columns hold only versioned documents with a schema.
- Migrations are forward-only, one logical change each, and owned by their module. A destructive change takes two releases: the first adds the new shape and stops using the old one, and the second removes the old shape.
- A test prepares every SQL statement against the fully migrated schema.
- Writes go through the store's writer, transactions stay short, and no I/O happens inside a transaction.

## 7. Observability

- `tracing` writes JSON logs to daily rotating files in each machine's log directory, capped in size and kept for 14 days. Development builds also log to the console.
- Spans cover each run, step, operation and task, with structured fields.
- Levels: `error` needs attention, `warn` means degraded behavior, `info` records lifecycle events, and `debug` and `trace` are for development.
- A redaction layer removes secrets. Model content is not logged at `info` level.
- There is no telemetry, metrics export or crash reporting service.

## 8. Testing

| Level                | What it covers                                                                                                                 | How                                                                  |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------- |
| Unit                 | Pure logic                                                                                                                     | `cargo test`, Vitest                                                 |
| Integration          | Modules against real dependencies: SQLite, processes, the link over loopback                                                   | `cargo test` in each crate's `tests/`                                |
| Provider conformance | Each adapter and quirk profile                                                                                                 | Recorded server-sent event fixtures; live endpoints every night (V8) |
| Loop                 | The agent loop without a network                                                                                               | A scripted fake provider                                             |
| Model evaluations    | Prompts and tool descriptions still work with real models                                                                      | Evaluation suites, on demand and every night                         |
| End to end           | The owner's flows in the real app                                                                                              | tauri-driver (WebDriver) on Windows and Linux, with axe checks       |
| Platform             | Tools and process containment on each platform                                                                                 | CI on Windows and Linux; a FreeBSD virtual machine for the worker    |
| Security             | Hostile Markdown and HTML, prompt injection, planted secrets                                                                   | Corpora and scenario tests                                           |
| Crash                | Recovery at every persistence point                                                                                            | Tests that end the process at each journal state                     |
| Upgrade              | Data from every earlier release                                                                                                | Fixture databases from each release                                  |
| Pseudo-locale        | No visible text bypasses the catalog, and layouts survive longer text                                                          | End-to-end tests run in the pseudo-locale                            |
| Performance          | Streaming, store throughput and startup against the targets in [goals and scope](../product/goals-and-scope.md#41-performance) | Benchmarks                                                           |

A flaky test is quarantined within a day and fixed within a week.

## 9. Supply chain

- `Cargo.lock` and `pnpm-lock.yaml` are committed, and CI installs from them with `--locked` and `--frozen-lockfile`.
- cargo-deny checks advisories, licenses ([ADR-0017](../adr/0017-license-and-dependency-policy.md)), bans ([ADR-0008](../adr/0008-one-workspace-two-binaries.md)) and sources (crates.io only, unless an exception is listed).
- pnpm's minimum release age stays in effect. Tooling versions are pinned exactly.
- GitHub Actions are pinned by commit SHA and run with minimal permissions.
- Release signing keys are kept outside the repository.

## 10. Versioning, branching and changelog

- One SemVer version covers the desktop app and the worker ([ADR-0018](../adr/0018-release-from-tagged-builds.md)).
- Development is trunk-based on `main`, with short-lived branches and squash merges.
- Commit messages follow Conventional Commits with a scope: `type(scope): summary` in lowercase imperative. The types are `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `build`, `ci` and `chore`.
- `CHANGELOG.md` follows Keep a Changelog 1.1.0. Each change adds one line, written for the owner, under Unreleased. A release turns that section into the version's notes.

## 11. Documentation

- Documents live in `docs/` as Markdown, with Mermaid diagrams.
- Every document opens with a status table. Product, roadmap, architecture and engineering documents number their second-level headings.
- Facts about third-party components carry the date they were verified.
- Documents link to each other instead of repeating each other.
- Prose is plain and specific: sentence-case headings, no dashes as connectors, no decoration.
- [ADR-0001](../adr/0001-record-architecture-decisions.md) governs architecture decisions, and the [feature protocol](../features/README.md) governs features.
- `task docs:check` must pass.
