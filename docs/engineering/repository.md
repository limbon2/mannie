# Repository layout

|              |                                                                                                                                                                      |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                     |
| Last updated | 2026-10-13                                                                                                                                                           |
| Related      | [ADR-0008](../adr/0008-one-workspace-two-binaries.md), [Extension points](../architecture/extension-points.md), [Standards](standards.md), [Releasing](releasing.md) |

## 1. Principles

1. One repository holds the desktop app, the worker, their shared crates and the documentation, and one version covers them all.
2. Contracts are the sources of truth: Rust types for IPC, protobuf files for the link, and SQL migrations for stored data. Everything generated from them is generated during the build and never committed.
3. Toolchains and tools are pinned, so a given commit always builds the same way.
4. Every workflow is one task in `Taskfile.yml`.

## 2. Layout today

The repository holds the documentation, the formatting and lint configuration that the code follows from its first line, and the checks for both:

```text
mannie/
├── AGENTS.md  CLAUDE.md  README.md  CONTRIBUTING.md  SECURITY.md  CHANGELOG.md  LICENSE
├── docs/                          # product, roadmap, architecture, engineering, ADRs, features
├── scripts/
│   ├── check-docs.mjs             # documentation consistency checks
│   ├── check-workspace-lints.mjs  # checks that every crate inherits the workspace lints
│   └── eslint/                    # local ESLint rules
├── tests/scripts/                 # tests of the repository scripts
├── Cargo.toml                     # workspace lint policy; the members arrive in M0
├── clippy.toml                    # Clippy settings
├── tombi.toml                     # TOML formatting and linting
├── eslint.config.js               # ESLint for TypeScript, Svelte and JavaScript
├── Taskfile.yml                   # every workflow
├── package.json                   # formatting, lint and documentation tooling, pinned
└── .editorconfig  .gitattributes  .gitignore  .prettierrc.json  .prettierignore
    .markdownlint-cli2.jsonc  .remarkrc.mjs  cspell.config.yaml
```

## 3. Target layout

M0 builds this layout:

```text
mannie/
├── Cargo.toml                   # workspace: members, shared dependencies, lints
├── rust-toolchain.toml          # pinned stable toolchain
├── deny.toml                    # cargo-deny: advisories, licenses, bans, sources
├── clippy.toml  tombi.toml      # Clippy settings; TOML formatting and linting
├── eslint.config.js             # ESLint for TypeScript, Svelte and JavaScript
├── buf.yaml                     # buf module for proto/
├── proto/mannie/link/           # link contracts, one package per service, versioned
├── crates/
│   ├── mannie-types/            # shared domain, IPC and settings types; the secret type
│   ├── mannie-store/            # SQLite, migrations, repositories (the only rusqlite user)
│   ├── mannie-provider/         # provider adapters and quirk profiles
│   ├── mannie-tools/            # tool registry, built-in tools, operations, process supervisor
│   ├── mannie-runtime/          # runs, agent loop, context, agent definitions, tasks
│   ├── mannie-link/             # link client and server, pairing, key pinning
│   └── mannie-secrets/          # operating system credential store (master only)
├── apps/
│   ├── desktop/                 # SvelteKit app (package.json, svelte.config.js, src/)
│   │   ├── messages/            # message catalogs, one file per locale (ADR-0019)
│   │   └── src-tauri/           # the mannie-desktop crate (the only tauri user)
│   └── worker/                  # the mannie-worker crate
├── scripts/                     # repository scripts, including the local ESLint rules
├── tests/scripts/               # tests of the repository scripts
├── docs/
├── Taskfile.yml
└── package.json  pnpm-workspace.yaml
```

## 4. Crates and dependency rules

| Crate             | Responsibility                                      | May depend on                                                     |
| ----------------- | --------------------------------------------------- | ----------------------------------------------------------------- |
| `mannie-types`    | Shared types and the redacting secret type. No I/O. | serde, specta, schemars                                           |
| `mannie-store`    | Persistence                                         | `mannie-types`, rusqlite                                          |
| `mannie-provider` | Provider adapters                                   | `mannie-types`, reqwest                                           |
| `mannie-tools`    | Tools, operations and process containment           | `mannie-types`, `mannie-store`                                    |
| `mannie-runtime`  | The agent runtime                                   | `mannie-types`, `mannie-store`, `mannie-provider`, `mannie-tools` |
| `mannie-link`     | The link between master and workers                 | `mannie-types`, tonic, prost, rustls                              |
| `mannie-secrets`  | The operating system credential store               | `mannie-types`, keyring-core and its stores                       |
| `mannie-desktop`  | The Tauri application and its IPC                   | Every library crate, tauri                                        |
| `mannie-worker`   | The worker service                                  | Every library crate except `mannie-secrets`                       |

cargo-deny enforces the rules that matter most: only `mannie-desktop` depends on `tauri`, only `mannie-store` on `rusqlite`, ring is the only crypto provider, and `openssl-sys` and `aws-lc-sys` are banned ([ADR-0008](../adr/0008-one-workspace-two-binaries.md)).

## 5. Generated artifacts

| Artifact                     | Generated from                       | Location                          | When                           |
| ---------------------------- | ------------------------------------ | --------------------------------- | ------------------------------ |
| TypeScript IPC bindings      | Rust types and command registrations | `apps/desktop/src/lib/generated/` | `task gen`, and debug builds   |
| Link code                    | `proto/mannie/link/`                 | Cargo's build output              | Every build                    |
| Tauri capability schemas     | Tauri's build step                   | `apps/desktop/src-tauri/gen/`     | Every build                    |
| Message functions            | `apps/desktop/messages/`             | `apps/desktop/src/lib/paraglide/` | Every build of the desktop app |
| Third-party notices and SBOM | Lockfiles                            | Release artifacts                 | Release builds                 |

Generated files are listed in `.gitignore` and never committed. To change one, change its source and run the generator.

## 6. Tool pinning

| Tool                                                | Pinned by                                             |
| --------------------------------------------------- | ----------------------------------------------------- |
| Rust                                                | `rust-toolchain.toml`                                 |
| Node.js                                             | `engines` in `package.json` (24 or newer)             |
| pnpm                                                | `packageManager` in `package.json`                    |
| Task                                                | 3.40 or newer                                         |
| Prettier, ESLint, Tombi and the documentation tools | Exact versions in `package.json` and `pnpm-lock.yaml` |
| shadcn-svelte CLI                                   | An exact devDependency of the desktop app             |
| cargo-deny, buf                                     | Exact versions installed by CI and by `task setup`    |

## 7. Tasks

| Task                                   | Does                                                                                                                          | Available                                           |
| -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------- |
| `task`                                 | Lists the tasks                                                                                                               | Now                                                 |
| `task setup`                           | Installs the pinned tooling from the lockfile                                                                                 | Now                                                 |
| `task fmt`                             | Formats and sorts all code and documentation: ESLint fixes, then Prettier, then Tombi                                         | Now; rustfmt joins in M0                            |
| `task fmt:check`                       | Checks that all code and documentation is formatted and sorted (Prettier, Tombi)                                              | Now; rustfmt joins in M0                            |
| `task fmt:rust`, `task fmt:check:rust` | Formats, or checks, the Rust code with rustfmt                                                                                | Now, for the first crate in M0                      |
| `task lint`                            | Runs the linters: `lint:ts` and `lint:toml`                                                                                   | Now; Clippy, svelte-check and cargo-deny join in M0 |
| `task lint:ts`                         | Lints TypeScript, Svelte and JavaScript with ESLint, including the local rules and the order of imports, types and attributes | Now                                                 |
| `task lint:toml`                       | Lints the TOML files against their schemas with Tombi                                                                         | Now                                                 |
| `task lint:rust`                       | Checks that every crate inherits the workspace lints, then runs Clippy with warnings denied                                   | Now, for the first crate in M0                      |
| `task test:scripts`                    | Runs the tests of the repository scripts, such as the local ESLint rules                                                      | Now                                                 |
| `task docs:fmt`                        | Formats the documentation and repository configuration with Prettier                                                          | Now                                                 |
| `task docs:check`                      | Checks formatting, Markdown rules, spelling, links and the consistency of the documents                                       | Now                                                 |
| `task ci`                              | Runs every check a change must pass: `docs:check`, `fmt:check`, `lint` and `test:scripts`                                     | Now                                                 |
| `task gen`                             | Generates the IPC bindings                                                                                                    | From M0                                             |
| `task dev`                             | Runs the desktop app in development mode                                                                                      | From M0                                             |
| `task build`                           | Builds the desktop app and the worker                                                                                         | From M0                                             |
| `task test`                            | Runs the unit and integration tests                                                                                           | From M0                                             |
| `task e2e`                             | Runs the end-to-end tests                                                                                                     | From M0                                             |
| `task release:prepare`                 | Bumps the version and turns the changelog's Unreleased section into the release's notes                                       | From M0                                             |

Task commands call real executables only, never Unix utilities, so every task runs the same on Windows, Linux and FreeBSD.

## 8. Environment

- The owner's workstation runs Windows 11. Development happens natively on Windows and on Linux.
- FreeBSD builds and tests of the worker run in CI, in a FreeBSD virtual machine.
- Tool caches stay in their default user locations, never in the repository: the Cargo registry, the pnpm store, and the Vite and Vitest caches. Build output (`target/`, `build/`, `.svelte-kit/`) stays in its ignored folders.

## 9. Agent tooling

M0 adds the shadcn-svelte agent skill (in `.claude/skills/` and `.agents/skills/`) and the Svelte MCP server configuration to the repository, and AGENTS.md describes how agents use them.
