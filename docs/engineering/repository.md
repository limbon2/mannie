# Repository layout

|              |                                                                                                                                                                                                                                                                    |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Draft for review                                                                                                                                                                                                                                                   |
| Last updated | 2026-10-16                                                                                                                                                                                                                                                         |
| Related      | [ADR-0008](../adr/0008-one-workspace-two-binaries.md), [ADR-0030](../adr/0030-split-mannie-into-a-core-and-plugins.md), [Extension points](../architecture/extension-points.md), [Standards](standards.md), [Releasing](releasing.md), [CI runners](ci-runners.md) |

## 1. Principles

1. One repository holds the desktop app, the core's crates, the internal plugins with their programs, such as the worker, and the documentation, and one version covers them all.
2. Contracts are the sources of truth: Rust types for IPC, protobuf files for the link, and SQL migrations for stored data. Everything generated from them is generated during the build and never committed.
3. Toolchains and tools are pinned, so a given commit always builds the same way.
4. Every workflow is one task in `Taskfile.yml`.

## 2. Layout

M0 to M4 built this layout, and M5 moved the worker's code into `plugins/workers/` ([ADR-0030](../adr/0030-split-mannie-into-a-core-and-plugins.md), [ADR-0031](../adr/0031-move-workers-into-the-first-plugin.md)):

```text
mannie/
├── AGENTS.md  CLAUDE.md  README.md  CONTRIBUTING.md  SECURITY.md  CHANGELOG.md  LICENSE
├── Cargo.toml  Cargo.lock       # workspace: members, shared dependencies, lints
├── rust-toolchain.toml          # pinned stable toolchain
├── deny.toml                    # cargo-deny: advisories, licenses, bans, sources
├── about.toml  about.hbs        # cargo-about: the third-party notices of a release
├── clippy.toml  tombi.toml      # Clippy settings; TOML formatting and linting
├── eslint.config.js             # ESLint for TypeScript, Svelte and JavaScript
├── buf.yaml                     # the buf module of the link's contracts in plugins/workers/proto/
├── Taskfile.yml                 # every workflow
├── package.json  pnpm-workspace.yaml  pnpm-lock.yaml   # repository tooling, pinned
├── .mcp.json  .codex/           # the Svelte MCP server for Claude Code and Codex
├── .claude/skills/  .agents/skills/  skills-lock.json   # the shadcn-svelte agent skill
├── .github/                     # CI, nightly FreeBSD, live provider, evaluation and release workflows
├── crates/
│   ├── mannie-types/            # shared domain, IPC and settings types; the secret type
│   ├── mannie-store/            # SQLite, migrations, repositories (the only rusqlite user)
│   ├── mannie-logging/          # tracing to rotating files, redaction, crash files
│   ├── mannie-secrets/          # operating system credential store (master only)
│   ├── mannie-provider/         # provider adapters, quirk profiles; fixtures in tests/fixtures/
│   ├── mannie-tools/            # the system tools, their policy and ceiling, file safety and the process supervisor
│   ├── mannie-runtime/          # runs as actors, the event log's hubs, context, compaction, tool calls, tasks, task executors and plugins' contributions; evaluations in tests/evals/
│   └── mannie-plugins/          # the plugin definition, the start context and its grants, and the plugin host
├── plugins/
│   └── workers/                 # the Workers plugin
│       ├── crates/mannie-link/  # the link: contracts, TLS with pinned keys, pairing, the master's and the worker's sides
│       ├── crates/mannie-workers/   # the master's side of workers, its executor and store module, and the plugin's definition
│       ├── worker/              # the mannie-worker crate, the worker service
│       └── proto/mannie/link/   # the link's protobuf packages, one folder for each service
├── apps/
│   └── desktop/                 # SvelteKit app (package.json, vite.config.ts, src/)
│       ├── messages/            # message catalogs, one file per locale (ADR-0019)
│       ├── project.inlang/      # the Paraglide project
│       └── src-tauri/           # the mannie-desktop crate (the only tauri user), the composition root that lists the plugins
├── scripts/                     # checks, the pseudo-locale, release steps, CI helpers, local ESLint rules
├── release/                     # the public key that release signatures are checked against
├── tests/
│   ├── apps/desktop/            # Vitest unit tests of the desktop app
│   ├── e2e/                     # WebdriverIO end-to-end tests (V1, V2, V3, V11, the features of M1 and M2), perf/ for NFR-PERF
│   └── scripts/                 # tests of the repository scripts
└── docs/                        # the documentation; the owner's setup guides are in docs/guides/
```

Tests live under `tests/` and mirror the folders of the code they test. Rust unit tests are the exception: they sit in a `tests.rs` beside the module, as Rust requires for private items.

SvelteKit 3 keeps its configuration in the `sveltekit()` call of `apps/desktop/vite.config.ts`, so there is no `svelte.config.js`. The app imports its own modules through the `#lib/*` subpath import of its `package.json`, which replaces the `$lib` alias.

## 3. Later crates

Each crate arrives with the milestone that first needs it, so no crate is a placeholder. M1 brought `mannie-provider` and `mannie-runtime`, M2 brought `mannie-tools`, and M4 brought `mannie-link`, together with the link's contracts and `buf.yaml`. M5 brought `mannie-plugins` and `mannie-workers`, and moved `mannie-link`, the contracts and the worker service into `plugins/workers/` with their names unchanged.

A plugin's crates, programs and contracts live in `plugins/<id>/`, and the core's crates in `crates/`. A plugin's UI and its Tauri commands stay in the desktop app's folders under the plugin's ID, because Tauri and the UI bundle belong to the desktop app ([desktop app](../architecture/desktop-app.md#6-frontend-structure)).

## 4. Crates and dependency rules

| Crate             | Responsibility                                                                 | May depend on                                                                                                            |
| ----------------- | ------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------ |
| `mannie-types`    | Shared types and the redacting secret type. No I/O.                            | serde, specta, thiserror, zeroize                                                                                        |
| `mannie-store`    | Persistence                                                                    | `mannie-types`, rusqlite                                                                                                 |
| `mannie-logging`  | Logging to rotating files, redaction and crash files                           | `mannie-types`, tracing and its subscriber and appender, regex                                                           |
| `mannie-provider` | Provider adapters and quirk profiles                                           | `mannie-types`, reqwest, rustls                                                                                          |
| `mannie-tools`    | Tools, operations, the policy ceiling and process containment                  | `mannie-types`, `mannie-store`, ignore, rustix on Unix, libc in the FreeBSD supervisor, windows-sys on Windows           |
| `mannie-runtime`  | The agent runtime                                                              | `mannie-types`, `mannie-store`, `mannie-provider`, `mannie-tools`                                                        |
| `mannie-plugins`  | What plugins build on, and the plugin host                                     | `mannie-types`, `mannie-store`, `mannie-runtime`                                                                         |
| `mannie-secrets`  | The operating system credential store                                          | `mannie-types`, keyring-core and its stores                                                                              |
| `mannie-link`     | The Workers plugin's link between master and workers                           | `mannie-types`, `mannie-runtime`, `mannie-logging`, tonic, prost, rustls, tokio-rustls, ring                             |
| `mannie-workers`  | The Workers plugin: the master's side of workers and the plugin's definition   | The core's library crates except `mannie-secrets`, `mannie-plugins`, `mannie-link`, tonic                                |
| `mannie-desktop`  | The Tauri application, its IPC and the composition root that lists the plugins | Every library crate and every plugin's crates, tauri, rfd                                                                |
| `mannie-worker`   | The Workers plugin's worker service                                            | The core's library crates except `mannie-secrets` and `mannie-plugins`, `mannie-link`, windows-service, clap, toml, ring |

cargo-deny enforces the rules that matter most: only `mannie-desktop` depends on `tauri`, only `mannie-store` on `rusqlite`, only `mannie-secrets` on keyring-core, and only `mannie-desktop` on `mannie-secrets`; ring is the only crypto provider, and `openssl-sys` and `aws-lc-sys` are banned ([ADR-0008](../adr/0008-one-workspace-two-binaries.md)). It also keeps the plugins' boundaries: only the Workers plugin's crates depend on `mannie-link` and `tonic`, and only `mannie-desktop` depends on `mannie-workers`, so no core crate and no other plugin reaches a plugin ([ADR-0030](../adr/0030-split-mannie-into-a-core-and-plugins.md), decision 10). `scripts/check-worker-dependencies.mjs` checks that no crate of Tauri, GTK or WebKitGTK reaches the worker on any target (V5).

## 5. Generated artifacts

| Artifact                     | Generated from                        | Location                                            | When                                     |
| ---------------------------- | ------------------------------------- | --------------------------------------------------- | ---------------------------------------- |
| TypeScript IPC bindings      | Rust types and command registrations  | `apps/desktop/src/lib/generated/`                   | `task gen`, and debug builds             |
| Message functions            | `apps/desktop/messages/`              | `apps/desktop/src/lib/paraglide/`                   | `task gen`, and every build of the app   |
| SvelteKit's types            | The routes and the Vite configuration | `apps/desktop/.svelte-kit/`                         | `task gen`, and every build of the app   |
| Pseudo-locale project        | The English catalog                   | `apps/desktop/.pseudo-locale/`                      | `task gen:pseudo-locale`, and `task e2e` |
| Tauri capability schemas     | Tauri's build step                    | `apps/desktop/src-tauri/gen/`                       | Every build                              |
| Tauri command permissions    | The command list in `build.rs`        | `apps/desktop/src-tauri/permissions/autogenerated/` | Every build                              |
| Microsoft Edge WebDriver     | The installed WebView2 version        | `target/edgedriver/`                                | `task e2e` on Windows                    |
| Performance figures          | A run of the performance suite        | `target/perf/results.json`                          | `task e2e:perf`                          |
| Link code                    | `plugins/workers/proto/mannie/link/`  | Cargo's build output                                | Every build, from M4                     |
| Third-party notices and SBOM | Lockfiles                             | Release artifacts                                   | Release builds                           |

Generated files are listed in `.gitignore` and never committed. To change one, change its source and run the generator.

The application icons in `apps/desktop/src-tauri/icons/` are assets, not code: the bundler reads them, so they are committed. They come from `apps/desktop/app-icon.svg` through `pnpm --dir apps/desktop exec tauri icon app-icon.svg`; delete the Android, iOS, macOS and Microsoft Store variants it also writes, which Mannie does not ship.

## 6. Tool pinning

| Tool                                                     | Pinned by                                                                         |
| -------------------------------------------------------- | --------------------------------------------------------------------------------- |
| Rust                                                     | `rust-toolchain.toml`                                                             |
| Node.js                                                  | `engines` in `package.json` (24 or newer)                                         |
| pnpm                                                     | `packageManager` in `package.json`                                                |
| Task                                                     | 3.40 or newer; CI installs 3.40.1                                                 |
| Prettier, ESLint, Tombi, buf and the documentation tools | Exact versions in `package.json` and `pnpm-lock.yaml`                             |
| Vitest                                                   | The pnpm catalog in `pnpm-workspace.yaml`, shared by the root and the desktop app |
| shadcn-svelte CLI, Tauri CLI, Paraglide JS               | Exact devDependencies of the desktop app                                          |
| WebdriverIO, axe-core, the Svelte MCP server             | Exact devDependencies at the root                                                 |
| cargo-deny, tauri-driver                                 | Exact versions installed by `task setup` and by CI                                |
| cargo-about, cargo-cyclonedx, cross                      | Exact versions installed by CI                                                    |
| GitHub Actions                                           | Commit SHAs, with the release tag in a comment                                    |
| Microsoft Edge WebDriver                                 | The version of the installed WebView2 runtime (`scripts/edgedriver.mjs`)          |
| The shadcn-svelte agent skill                            | `skills-lock.json`, written by the skills CLI 1.7.2                               |

## 7. Tasks

| Task                                   | Does                                                                                                                                                                                                                   |
| -------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `task`                                 | Lists the tasks                                                                                                                                                                                                        |
| `task setup`                           | Installs the pinned pnpm packages, cargo-deny and tauri-driver                                                                                                                                                         |
| `task gen`                             | Generates the IPC bindings, compiles the message catalogs and writes SvelteKit's types                                                                                                                                 |
| `task gen:pseudo-locale`               | Generates the pseudo-locale project that the end-to-end tests use                                                                                                                                                      |
| `task dev`                             | Runs the desktop app in development mode                                                                                                                                                                               |
| `task build`                           | Builds the desktop app's installers and the worker in release mode                                                                                                                                                     |
| `task fmt`                             | Formats and sorts all code and documentation: ESLint fixes, Prettier, Tombi, rustfmt and `buf format`                                                                                                                  |
| `task fmt:check`                       | Checks that all code and documentation is formatted and sorted, the contracts with `buf format` included                                                                                                               |
| `task fmt:rust`, `task fmt:check:rust` | Formats, or checks, the Rust code with rustfmt                                                                                                                                                                         |
| `task lint`                            | Runs the linters and the dependency and catalog checks below                                                                                                                                                           |
| `task lint:ts`                         | Lints TypeScript, Svelte and JavaScript with ESLint, including the local rules and the order of imports, types and attributes                                                                                          |
| `task lint:types`                      | Type-checks the desktop app and its unit tests with svelte-check, and the end-to-end tests with tsc                                                                                                                    |
| `task lint:toml`                       | Lints the TOML files against their schemas with Tombi                                                                                                                                                                  |
| `task lint:rust`                       | Checks that every crate inherits the workspace lints, then runs Clippy with warnings denied                                                                                                                            |
| `task lint:deps`                       | Runs cargo-deny, the worker's dependency check (V5) and the license check of the desktop app's npm packages                                                                                                            |
| `task lint:messages`                   | Checks the message catalogs with the inlang CLI                                                                                                                                                                        |
| `task lint:proto`                      | Lints the link's contracts with `buf lint`, and checks with `buf breaking` that they only grew since the merge base with `origin/main` (or the branch in `MANNIE_CONTRACTS_BASE`) and the newest release that has them |
| `task test`                            | Runs the Rust tests, the desktop app's Vitest tests and the tests of the repository scripts                                                                                                                            |
| `task test:linux`                      | Runs Clippy and the Rust tests in a Linux container, without the desktop crate, for a workstation that runs Windows                                                                                                    |
| `task e2e`                             | Builds the validation app and the worker, and runs the end-to-end tests through tauri-driver; the workers' tests pair the app with a real worker                                                                       |
| `task e2e:perf`                        | Builds the validation app and measures NFR-PERF-01 to 04; `MANNIE_PERF_REPORT_ONLY=1` reports without enforcing the targets                                                                                            |
| `task bench`                           | Runs the store's benchmarks                                                                                                                                                                                            |
| `task provider:live`                   | Runs the live conformance tests against the server that the `MANNIE_LIVE_*` variables name (V8)                                                                                                                        |
| `task provider:record`                 | Records the fixtures of that server as the live tests run (V8)                                                                                                                                                         |
| `task tools:eval`                      | Runs the system tools' evaluations with the model that the `MANNIE_EVAL_*` variables name, or with the scripted oracle                                                                                                 |
| `task release:prepare VERSION=x.y.z`   | Sets the version and turns the changelog's Unreleased section into the release's section ([releasing](releasing.md))                                                                                                   |
| `task docs:fmt`                        | Formats the documentation and repository configuration with Prettier                                                                                                                                                   |
| `task docs:check`                      | Checks formatting, Markdown rules, spelling, links and the consistency of the documents                                                                                                                                |
| `task ci`                              | Runs every check a change must pass: `gen`, `docs:check`, `fmt:check`, `lint` and `test`                                                                                                                               |

Task commands call real executables only, never Unix utilities, so every task runs the same on Windows, Linux and FreeBSD.

## 8. Environment

- The owner's workstation runs Windows 11. Development happens natively on Windows and on Linux.
- FreeBSD builds and tests of the worker run in CI, in a FreeBSD virtual machine.
- CI runs on the owner's self-hosted runners, except the FreeBSD virtual machine and the release's own jobs, which run on GitHub's runners ([CI runners](ci-runners.md)).
- Tool caches stay in their default user locations, never in the repository: the Cargo registry, the pnpm store, and the Vite and Vitest caches. Build output (`target/`, `build/`, `.svelte-kit/`) stays in its ignored folders.

## 9. Agent tooling

- The official shadcn-svelte agent skill is vendored in `.claude/skills/shadcn-svelte/` for Claude Code and `.agents/skills/shadcn-svelte/` for Codex ([ADR-0006](../adr/0006-use-shadcn-svelte-through-its-cli.md)). The skills CLI installs and updates it, and `skills-lock.json` records its source and hash.
- `.mcp.json` and `.codex/config.toml` start the Svelte MCP server from the pinned `@sveltejs/mcp` package, for documentation lookups and the `svelte-autofixer`.
- [AGENTS.md](../../AGENTS.md) describes how agents use both.
