# Repository layout

|              |                                                                                                                                                                                                                                                                                                                                                   |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                                                                                                                                                                                  |
| Last updated | 2026-10-21                                                                                                                                                                                                                                                                                                                                        |
| Related      | [ADR-0008](../adr/0008-one-workspace-two-binaries.md), [ADR-0030](../adr/0030-split-mannie-into-a-core-and-plugins.md), [ADR-0035](../adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md), [Extension points](../architecture/extension-points.md), [Standards](standards.md), [Releasing](releasing.md), [CI runners](ci-runners.md) |

## 1. Principles

1. One repository holds the desktop app, the core's crates, the internal plugins with their programs, such as the worker, and the documentation, and one version covers them all.
2. Contracts are the sources of truth: Rust types for IPC, protobuf files for the link, and SQL migrations for stored data. Everything generated from them is generated during the build and never committed.
3. Toolchains and tools are pinned, so a given commit always builds the same way.
4. Every workflow is one task in `Taskfile.yml`.

## 2. Layout

M0 to M4 built this layout, M5 moved the worker's code into `plugins/workers/` and made each plugin a Cargo workspace of its own, and M7 added `plugins/images/` ([ADR-0030](../adr/0030-split-mannie-into-a-core-and-plugins.md), [ADR-0031](../adr/0031-move-workers-into-the-first-plugin.md), [ADR-0033](../adr/0033-let-plugins-attach-images-to-the-owners-messages.md), [ADR-0035](../adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md)):

```text
mannie/
├── AGENTS.md  CLAUDE.md  README.md  CONTRIBUTING.md  SECURITY.md  CHANGELOG.md  LICENSE
├── Cargo.toml  Cargo.lock       # the root workspace: the core's crates and the desktop app's library, shared dependencies, lints
├── .cargo/config.toml           # target/ for every Cargo workspace (Clippy uses target/clippy), the dev profile, the Windows linker
├── .config/nextest.toml         # cargo-nextest: how the Rust tests of every workspace run
├── .generated/                  # what `task gen` writes and Git ignores: the launcher, its lock, the editor workspace
├── rust-toolchain.toml          # pinned stable toolchain
├── deny.toml                    # cargo-deny: advisories, licenses, bans, sources
├── about.toml  about.hbs        # cargo-about: the third-party notices of a release
├── clippy.toml  tombi.toml      # Clippy settings; TOML formatting and linting
├── eslint.config.js             # ESLint for TypeScript, Svelte and JavaScript
├── tsconfig.json                # references the app's and the end-to-end tests' projects, for the type-aware lint of plugins/
├── .taurignore                  # the plugins' files that `tauri dev` leaves to Vite's reload
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
│   ├── mannie-runtime/          # runs as actors, the event log's hubs, context, compaction, tool calls, tasks, task executors, mention providers, image sources and plugins' contributions; evaluations in tests/evals/
│   ├── mannie-plugin-sdk/       # the plugin SDK: the definition, the manifest and its schema, the services, the points, the specs of commands
│   ├── mannie-plugin-harness/   # what plugins' tests use of the core: one plugin through the real host, the conformance check
│   ├── mannie-host-kit/         # what a plugin's programs, such as the worker, use of the core
│   └── mannie-plugins/          # the plugin host: starts, stops, services and the dispatch of plugin calls
├── plugins/                     # one folder per plugin: plugin.toml, CHANGELOG.md, and a Cargo workspace with its own Cargo.lock
│   ├── images/                  # the Images plugin
│   │   ├── crates/mannie-images/    # preparing the owner's images, the image source and the plugin's definition
│   │   ├── ui/                  # the plugin's UI: index.ts with its definePlugin(...), its button and actions
│   │   ├── tests/               # its Vitest tests in ui/ and its end-to-end tests in e2e/
│   │   └── docs/                # its feature document, README.md
│   └── workers/                 # the Workers plugin
│       ├── crates/mannie-link/  # the link: contracts, TLS with pinned keys and pairing
│       ├── crates/mannie-link-worker/   # the worker's side of the link, which only the worker uses
│       ├── crates/mannie-workers/   # the master's side of workers, its executor and store module, and the plugin's definition
│       ├── crates/mannie-workers-testing/   # the test tools: a worker on loopback, relays, the delegation harness
│       ├── ui/                  # the plugin's UI: the sidebar group, the dialogs, the settings section and their state
│       ├── messages/            # the plugin's message catalogs, one file per locale, every key under workers_
│       ├── tests/               # its Vitest tests in ui/, and its end-to-end tests with a real worker in e2e/
│       ├── worker/              # the mannie-worker crate, the worker service
│       ├── ci/                  # the worker service's smoke tests, which plugin.toml names
│       ├── proto/mannie/link/   # the link's protobuf packages, one folder for each service
│       └── docs/                # its feature document, README.md, its parts and the owner's guide to setting up a worker
├── apps/
│   └── desktop/                 # SvelteKit app (package.json, vite.config.ts, src/)
│       ├── messages/            # message catalogs, one file per locale (ADR-0019)
│       ├── project.inlang/      # the Paraglide project
│       ├── core/                # the mannie-desktop-core crate: the composition root, the IPC commands, the window and tray
│       └── src-tauri/           # the mannie-desktop binary, a member of the launcher: Tauri's configuration, build script, capabilities and icons
├── scripts/                     # checks, the steps of `task ci`, the pseudo-locale, release steps, CI helpers, local ESLint rules
│   └── plugins/                 # the generator of `task gen`, the locks' tasks and the runner of a command in every workspace
├── release/                     # the public key that release signatures are checked against
├── tests/
│   ├── apps/desktop/            # Vitest unit tests of the desktop app
│   ├── e2e/                     # WebdriverIO end-to-end tests (V1, V2, V3, V11, the features of M1 and M2), perf/ for NFR-PERF
│   ├── fixtures/plugins/example/    # the fixture plugin with its UI and tests, which validation builds bundle, the core's tests use and `task plugins:new` copies
│   └── scripts/                 # tests of the repository scripts
└── docs/                        # the documentation of the core; each plugin's is in its folder's docs/
```

Tests live under `tests/` and mirror the folders of the code they test. Rust unit tests are the exception: they sit in a `tests.rs` beside the module, as Rust requires for private items. A plugin keeps its tests in its folder's `tests/`: the Vitest tests of its UI in `tests/ui/`, which mirrors `ui/`, and its end-to-end tests in `tests/e2e/`, which import the shared support of `tests/e2e/support/` as `#e2e/<module>.ts` ([ADR-0035](../adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md), decision 9).

SvelteKit 3 keeps its configuration in the `sveltekit()` call of `apps/desktop/vite.config.ts`, so there is no `svelte.config.js`. The app imports its own modules through the `#lib/*` subpath import of its `package.json`, which replaces the `$lib` alias.

## 3. Later crates

Each crate arrives with the milestone that first needs it, so no crate is a placeholder. M1 brought `mannie-provider` and `mannie-runtime`, M2 brought `mannie-tools`, and M4 brought `mannie-link`, together with the link's contracts and `buf.yaml`. M5 brought `mannie-plugins` and `mannie-workers`, and moved `mannie-link`, the contracts and the worker service into `plugins/workers/` with their names unchanged. M7 brought `mannie-images`.

A plugin's crates, programs and contracts live in `plugins/<id>/`, a Cargo workspace of its own with a committed `Cargo.lock`, and the core's crates in `crates/`. A plugin's commands reach it through the plugin host's dispatch, and its UI, messages and tests live in its folder too ([desktop app](../architecture/desktop-app.md#6-frontend-structure), [plugins](../features/plugins.md#plugin-sdk)).

The desktop app builds from three kinds of workspace ([ADR-0035](../adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md), decision 4). The root's holds the core's crates and `mannie-desktop-core`, so it loads on a fresh clone and its `Cargo.lock` pins the core and Tauri. Each plugin's holds the plugin's crates and programs, so `cargo build --manifest-path plugins/workers/Cargo.toml -p mannie-worker` builds the worker without the desktop app. The launcher, `.generated/desktop/`, holds the binary `mannie-desktop` and the crates that `task gen` generates: `mannie-plugin-list`, which returns the plugins in the order of their starts, and `mannie-plugin-bindings`, which writes each plugin's bindings. Its lock is seeded from the committed locks, normalized offline and checked against them, so every crate of the desktop app is one that a committed lock pins.

## 4. Crates and dependency rules

| Crate                    | Responsibility                                                                                   | May depend on                                                                                                                                     |
| ------------------------ | ------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `mannie-types`           | Shared types and the redacting secret type. No I/O.                                              | serde, specta, thiserror, zeroize                                                                                                                 |
| `mannie-store`           | Persistence                                                                                      | `mannie-types`, rusqlite                                                                                                                          |
| `mannie-logging`         | Logging to rotating files, redaction and crash files                                             | `mannie-types`, tracing and its subscriber and appender, regex                                                                                    |
| `mannie-provider`        | Provider adapters and quirk profiles                                                             | `mannie-types`, reqwest, rustls                                                                                                                   |
| `mannie-tools`           | Tools, operations, the policy ceiling and process containment                                    | `mannie-types`, `mannie-store`, ignore, rustix on Unix, libc in the FreeBSD supervisor, windows-sys on Windows                                    |
| `mannie-runtime`         | The agent runtime                                                                                | `mannie-types`, `mannie-store`, `mannie-provider`, `mannie-tools`                                                                                 |
| `mannie-plugin-sdk`      | What plugins build on: the plugin SDK                                                            | `mannie-types`, serde, specta, schemars, toml for manifests                                                                                       |
| `mannie-plugins`         | The plugin host                                                                                  | `mannie-types`, `mannie-plugin-sdk`, `mannie-store`                                                                                               |
| `mannie-plugin-harness`  | What plugins' tests use of the core: one plugin through the real host, and the conformance check | `mannie-plugin-sdk`, `mannie-plugins`, `mannie-runtime`, `mannie-store`, `mannie-provider`, `mannie-tools`, `mannie-types`, with their test tools |
| `mannie-host-kit`        | What a plugin's programs use of the core                                                         | `mannie-runtime`, `mannie-tools`, `mannie-store`, `mannie-logging`, `mannie-provider`, `mannie-types`                                             |
| `mannie-secrets`         | The operating system credential store                                                            | `mannie-types`, keyring-core and its stores                                                                                                       |
| `mannie-link`            | The Workers plugin's link between master and workers                                             | `mannie-plugin-sdk`, tonic, prost, rustls, tokio-rustls, ring; its tests also `mannie-plugin-harness`                                             |
| `mannie-link-worker`     | The worker's side of the link                                                                    | `mannie-link`, `mannie-host-kit`, `mannie-plugin-sdk`, tonic, rustls                                                                              |
| `mannie-workers`         | The Workers plugin: the master's side of workers and the plugin's definition                     | `mannie-plugin-sdk`, `mannie-link`, tonic, rustls; its tests also `mannie-plugin-harness`, `mannie-link-worker` and `mannie-workers-testing`      |
| `mannie-workers-testing` | The Workers plugin's test tools, which only tests and the smoke test's master use                | The plugin's crates, `mannie-plugin-harness`, `mannie-host-kit`                                                                                   |
| `mannie-images`          | The Images plugin: preparing the owner's images and the plugin's definition                      | `mannie-plugin-sdk`, image; its tests also `mannie-plugin-harness`                                                                                |
| `mannie-desktop-core`    | The desktop app's library: its IPC, its window and tray and the composition root                 | Every library crate of the core, tauri, rfd; never a plugin's crate                                                                               |
| `mannie-desktop`         | The Tauri binary, built in the launcher                                                          | `mannie-desktop-core`, the generated `mannie-plugin-list`, tauri                                                                                  |
| `mannie-worker`          | The Workers plugin's worker service                                                              | `mannie-host-kit`, `mannie-plugin-sdk`, `mannie-link`, `mannie-link-worker`, windows-service, clap, toml, ring                                    |

cargo-deny enforces the rules that matter most in every workspace and in the launcher: only `mannie-desktop-core` and `mannie-desktop` depend on `tauri`, only `mannie-store` on `rusqlite`, only `mannie-secrets` on keyring-core, and only `mannie-desktop-core` on `mannie-secrets` and rfd; ring is the only crypto provider, and `openssl-sys` and `aws-lc-sys` are banned ([ADR-0008](../adr/0008-one-workspace-two-binaries.md)). The workspaces keep the plugins apart: the root's workspace never reaches a plugin's crate, and in the launcher only the generated plugin list and bindings program depend on the plugins' entry crates ([ADR-0035](../adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md), decision 4). `scripts/plugins/check-boundaries.mjs` checks the rest in every graph and for every kind of dependency ([ADR-0035](../adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md), decision 6). No core crate depends on a plugin's crate, and no plugin depends on another plugin. A plugin's crates reach the core only through the plugin SDK. Their tests may also use the plugin harness, as a dev-dependency or from a crate that only tests reach, and the plugin's programs and the crates that its manifest lists under them may also use the host kit; what the plugin's entry crate ships reaches neither. The crates that a manifest lists under `[crates] exclusive`, such as Workers' `tonic` and Images' `image`, are used only by that plugin. The library crates that a plugin's entry crate ships write files, run processes and open sockets only through the SDK's services, unless the plugin's manifest grants the category under `[native]` with its reason, as Workers grants the link its sockets: `scripts/plugins/check-native.mjs` runs Clippy on them with the root's configuration and the calls of every other category refused, and refuses a suppression of that rule in their sources ([ADR-0035](../adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md), decision 6). An installed program, such as git, runs only through the service `processes`, which runs the programs that the manifest names under `[processes.<name>]` ([ADR-0038](../adr/0038-let-plugins-run-installed-programs-in-the-owners-projects.md), decision 7). `task plugins:grants` lists every plugin's grants, with each installed program's reason, fixed arguments and inherited variables. `node scripts/plugins/programs.mjs check` checks every headless program that a manifest declares, such as the worker, so that no crate of Tauri, GTK or WebKitGTK reaches it on any target (V5). `scripts/check-workspace-lints.mjs` checks that every crate of every workspace inherits its workspace's lints, and that every workspace repeats the root's package settings and lints.

## 5. Generated artifacts

| Artifact                               | Generated from                                                                                                   | Location                                                           | When                                                                                                   |
| -------------------------------------- | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------ |
| Launcher and its lock                  | The plugins' manifests, the root's `Cargo.toml` and the committed locks                                          | `.generated/desktop/`                                              | `task gen`                                                                                             |
| Fingerprint                            | The plugins' manifests, workspaces and locks, the root's and the desktop app's manifests, and the interfaces     | `.generated/desktop/fingerprint`                                   | The last step of `task gen`; the desktop binary's build script refuses to build when it is out of date |
| Editor workspace                       | Every Cargo workspace                                                                                            | `.generated/mannie.code-workspace`                                 | `task gen`                                                                                             |
| TypeScript IPC bindings                | Rust types and command registrations                                                                             | `apps/desktop/src/lib/generated/`                                  | `task gen`, and debug builds                                                                           |
| A plugin's bindings and the interfaces | Each plugin's commands and channels                                                                              | `plugins/<id>/ui/generated/`, `.generated/desktop/interfaces.json` | `task gen`                                                                                             |
| The UI's plugin lists                  | The plugins' manifests and whether each has `ui/index.ts`                                                        | `apps/desktop/src/lib/generated/plugin-list*.ts`                   | `task gen`                                                                                             |
| A plugin's message barrel              | The keys of the plugin's catalog                                                                                 | `plugins/<id>/ui/generated/messages.ts`, `message-list.ts`         | `task gen`                                                                                             |
| Message functions                      | `apps/desktop/messages/` and each plugin's `messages/`, through the inlang project in `apps/desktop/.generated/` | `apps/desktop/src/lib/paraglide/`                                  | `task gen`, and every build of the app                                                                 |
| SvelteKit's types                      | The routes and the Vite configuration                                                                            | `apps/desktop/.svelte-kit/`                                        | `task gen`, and every build of the app                                                                 |
| Pseudo-locale project                  | The English catalog                                                                                              | `apps/desktop/.pseudo-locale/`                                     | `task gen:pseudo-locale`, and `task e2e`                                                               |
| Tauri capability schemas               | Tauri's build step                                                                                               | `apps/desktop/src-tauri/gen/`                                      | Every build                                                                                            |
| Tauri command permissions              | The command list in `build.rs` and the plugins' interfaces                                                       | `apps/desktop/src-tauri/permissions/autogenerated/`                | Every build                                                                                            |
| Microsoft Edge WebDriver               | The installed WebView2 version                                                                                   | `target/edgedriver/`                                               | `task e2e` on Windows                                                                                  |
| Performance figures                    | A run of the performance suite                                                                                   | `target/perf/results.json`                                         | `task e2e:perf`                                                                                        |
| Link code                              | `plugins/workers/proto/mannie/link/`                                                                             | Cargo's build output                                               | Every build, from M4                                                                                   |
| A plugin's Clippy configuration        | The root's `clippy.toml` and the plugin's `[native]` grants                                                      | `.generated/clippy/<id>/clippy.toml`                               | `task lint:rust`                                                                                       |
| Third-party notices and SBOM           | The launcher's graph and the Workers plugin's workspace                                                          | Release artifacts                                                  | Release builds                                                                                         |

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
| cargo-deny, cargo-nextest, tauri-driver                  | Exact versions installed by `task setup` and by CI                                |
| cargo-about, cargo-cyclonedx, cross                      | Exact versions installed by CI                                                    |
| GitHub Actions                                           | Commit SHAs, with the release tag in a comment                                    |
| Microsoft Edge WebDriver                                 | The version of the installed WebView2 runtime (`scripts/edgedriver.mjs`)          |
| The shadcn-svelte agent skill                            | `skills-lock.json`, written by the skills CLI 1.7.2                               |

## 7. Tasks

| Task                                   | Does                                                                                                                                                                                                                                                                                                            |
| -------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `task`                                 | Lists the tasks                                                                                                                                                                                                                                                                                                 |
| `task setup`                           | Installs the pinned pnpm packages, cargo-deny, cargo-nextest and tauri-driver, fetches the crates of every Cargo workspace's lock, and runs `task gen`                                                                                                                                                          |
| `task gen`                             | Checks the plugins' manifests and the committed locks, writes the launcher and its lock, generates the IPC bindings of the core and of each plugin, compiles the message catalogs and writes SvelteKit's types                                                                                                  |
| `task gen:pseudo-locale`               | Generates the pseudo-locale project that the end-to-end tests use                                                                                                                                                                                                                                               |
| `task dev`                             | Runs the desktop app in development mode as `io.github.limbon2.mannie.dev`, with a data folder, credentials and logs of its own, beside a release build if one is running                                                                                                                                       |
| `task build`                           | Builds the desktop app's installers and the plugins' programs, such as the worker, in release mode                                                                                                                                                                                                              |
| `task fmt`                             | Formats and sorts all code and documentation: ESLint fixes, Prettier, Tombi, rustfmt and `buf format`                                                                                                                                                                                                           |
| `task fmt:check`                       | Checks that all code and documentation is formatted and sorted, the contracts with `buf format` included                                                                                                                                                                                                        |
| `task fmt:rust`, `task fmt:check:rust` | Formats, or checks, the Rust code of every Cargo workspace and of the launcher with rustfmt                                                                                                                                                                                                                     |
| `task lint`                            | Runs the linters and the dependency and catalog checks below                                                                                                                                                                                                                                                    |
| `task lint:ts`                         | Lints TypeScript, Svelte and JavaScript with ESLint in two threads, including the local rules and the order of imports, types and attributes                                                                                                                                                                    |
| `task lint:types`                      | Type-checks the desktop app and its unit tests with svelte-check, and the end-to-end tests with tsc                                                                                                                                                                                                             |
| `task lint:toml`                       | Lints the TOML files against their schemas with Tombi                                                                                                                                                                                                                                                           |
| `task lint:rust`                       | Checks that every crate inherits its workspace's lints and that the workspaces repeat the root's settings, then runs Clippy with warnings denied in every workspace and the launcher, and on the library crates that each plugin ships with the core's rule against direct native calls, all in `target/clippy` |
| `task lint:deps`                       | Fetches the advisory database once, then runs cargo-deny offline in every workspace and on the launcher's graph, the worker's dependency check (V5), the boundary check between the core and the plugins, and the license check of the desktop app's npm packages                                               |
| `task lint:plugins`                    | Checks that the launcher, its lock, the plugin list and the fingerprint are what `task gen` writes from the plugins as they are                                                                                                                                                                                 |
| `task lint:messages`                   | Checks the message catalogs with the inlang CLI                                                                                                                                                                                                                                                                 |
| `task lint:proto`                      | Lints the link's contracts with `buf lint`, and checks with `buf breaking` that they only grew since the merge base with `origin/main` (or the branch in `MANNIE_CONTRACTS_BASE`) and the newest release that has them                                                                                          |
| `task test`                            | Runs the Rust tests of every workspace and of the desktop binary with cargo-nextest, then their doctests, the desktop app's Vitest tests and the tests of the repository scripts                                                                                                                                |
| `task test:rust:build`                 | Builds the Rust tests of every workspace and of the desktop binary without running them                                                                                                                                                                                                                         |
| `task test:linux`                      | Runs Clippy and the Rust tests of every workspace in a Linux container, without the desktop app's crates, for a workstation that runs Windows                                                                                                                                                                   |
| `task e2e`                             | Builds the validation app and the worker, and runs the end-to-end tests through tauri-driver; the workers' tests pair the app with a real worker                                                                                                                                                                |
| `task e2e:perf`                        | Builds the validation app and measures NFR-PERF-01 to 04; `MANNIE_PERF_REPORT_ONLY=1` reports without enforcing the targets                                                                                                                                                                                     |
| `task bench`                           | Runs the store's benchmarks                                                                                                                                                                                                                                                                                     |
| `task provider:live`                   | Runs the live conformance tests against the server that the `MANNIE_LIVE_*` variables name (V8)                                                                                                                                                                                                                 |
| `task provider:record`                 | Records the fixtures of that server as the live tests run (V8)                                                                                                                                                                                                                                                  |
| `task tools:eval`                      | Runs the system tools' evaluations with the model that the `MANNIE_EVAL_*` variables name, or with the scripted oracle                                                                                                                                                                                          |
| `task release:prepare VERSION=x.y.z`   | Sets the version in every Cargo workspace and turns the changelog's Unreleased section into the release's section ([releasing](releasing.md))                                                                                                                                                                   |
| `task plugins:new ID=<id>`             | Makes a new plugin in `plugins/<id>/` from the fixture plugin, with a lock seeded from the root's                                                                                                                                                                                                               |
| `task plugins:lock ID=<id>`            | Records a plugin's changed `Cargo.toml` in its lock, with the network only for a crate that Cargo's cache lacks                                                                                                                                                                                                 |
| `task plugins:grants`                  | Runs `task gen`, then lists what each plugin may use: its services, secret entries, commands, channels, programs, the crates that only it may use and its direct native calls                                                                                                                                   |
| `task deps:update`, `task deps:align`  | Update the root's crates, or only bring the plugins' locks to the root's versions, so that the committed locks agree                                                                                                                                                                                            |
| `task docs:fmt`                        | Formats the documentation and repository configuration with Prettier                                                                                                                                                                                                                                            |
| `task docs:check`                      | Checks formatting, Markdown rules, spelling, links and the consistency of the documents                                                                                                                                                                                                                         |
| `task docs:lint`                       | Checks the documents' Markdown rules, spelling, links and consistency, without the formatting that `task fmt:check` checks                                                                                                                                                                                      |
| `task ci`                              | Runs every check a change must pass, the steps of `gen`, `docs:check`, `fmt:check`, `lint` and `test`, side by side through `scripts/ci.mjs` within limits that keep the machine usable, and ends with every step's result and time ([ADR-0037](../adr/0037-run-the-checks-of-task-ci-side-by-side.md))         |

Task commands call real executables only, never Unix utilities, so every task runs the same on Windows, Linux and FreeBSD.

## 8. Environment

- The owner's workstation runs Windows 11. Development happens natively on Windows and on Linux.
- FreeBSD builds and tests of the worker run in CI, in a FreeBSD virtual machine.
- Every CI job runs on standard GitHub-hosted runners, with native FreeBSD builds and tests in a virtual machine on a Linux runner ([CI runners](ci-runners.md)).
- Tool caches stay in their default user locations, never in the repository: the Cargo registry, the pnpm store, and the Vite and Vitest caches. Build output (`target/`, `build/`, `.svelte-kit/`) and what `task gen` writes (`.generated/`) stay in their ignored folders.
- No committed `Cargo.lock` changes in `task gen`, `task dev`, `task build`, `task e2e` or a release build, which all pass `--locked`. Only `task deps:update`, `task deps:align`, `task plugins:lock`, `task plugins:new` and `task release:prepare` write committed locks ([ADR-0035](../adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md), decision 5).

## 9. Agent tooling

- The official shadcn-svelte agent skill is vendored in `.claude/skills/shadcn-svelte/` for Claude Code and `.agents/skills/shadcn-svelte/` for Codex ([ADR-0006](../adr/0006-use-shadcn-svelte-through-its-cli.md)). The skills CLI installs and updates it, and `skills-lock.json` records its source and hash.
- `.mcp.json` and `.codex/config.toml` start the Svelte MCP server from the pinned `@sveltejs/mcp` package, for documentation lookups and the `svelte-autofixer`.
- [AGENTS.md](../../AGENTS.md) describes how agents use both.
