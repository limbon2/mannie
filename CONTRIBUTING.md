# Contributing

Read [AGENTS.md](AGENTS.md) and the [engineering standards](docs/engineering/standards.md) first. They apply to people as much as to coding agents.

## Prerequisites

| Tool                         | Version                 | Used for                                                        |
| ---------------------------- | ----------------------- | --------------------------------------------------------------- |
| Node.js                      | 24 or newer             | Documentation tooling now, and the desktop UI from M0           |
| pnpm                         | 11.8 (`packageManager`) | JavaScript dependencies                                         |
| [Task](https://taskfile.dev) | 3.40 or newer           | Every workflow in `Taskfile.yml`                                |
| Git                          | Current                 | Version control                                                 |
| Rust, through rustup         | From M0                 | The core and the worker; `rust-toolchain.toml` pins the version |

From M0 on, the desktop app also needs its platform webview: the WebView2 runtime on Windows, which Windows 11 includes, and the WebKitGTK 4.1 development packages on Linux. The [repository](docs/engineering/repository.md) document lists the tasks.

## Workflow

1. Run `task setup` once after cloning.
2. For anything new, follow the [feature protocol](docs/features/README.md). An architectural change needs an [ADR](docs/adr/0000-template.md) before any code.
3. Make the change with its tests and documentation, meeting the [Definition of Done](docs/engineering/standards.md#1-definition-of-done).
4. Run `task fmt`, then `task ci`.
5. Commit with a [Conventional Commits](https://www.conventionalcommits.org/) message that has a scope, for example `feat(chat): stream reasoning summaries` or `docs(adr): accept ADR-0009`.
6. Add one line to `CHANGELOG.md` under Unreleased for every change the owner would notice.

While the owner is the only contributor, commits need no sign-off. This changes before the repository accepts outside contributions ([ADR-0017](docs/adr/0017-license-and-dependency-policy.md)).
