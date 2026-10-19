# Contributing

Read [AGENTS.md](AGENTS.md) and the [engineering standards](docs/engineering/standards.md) first. They apply to people as much as to coding agents.

## Prerequisites

| Tool                         | Version                 | Used for                                                                                            |
| ---------------------------- | ----------------------- | --------------------------------------------------------------------------------------------------- |
| Node.js                      | 24 or newer             | The desktop UI, the repository scripts and the documentation tooling                                |
| pnpm                         | 11.8 (`packageManager`) | JavaScript dependencies                                                                             |
| [Task](https://taskfile.dev) | 3.40 or newer           | Every workflow in `Taskfile.yml`                                                                    |
| Git                          | Current                 | Version control                                                                                     |
| Rust, through rustup         | `rust-toolchain.toml`   | The core and the worker. rustup installs the pinned toolchain on the first `cargo` command          |
| A C compiler                 | Current                 | SQLite, which rusqlite builds from source: Visual Studio's C++ build tools on Windows, gcc on Linux |

The desktop app needs its platform webview: the WebView2 runtime on Windows, which Windows 11 includes, and on Linux the WebKitGTK 4.1 development packages with the libraries Tauri uses. On Ubuntu and Debian:

```bash
sudo apt-get install libwebkit2gtk-4.1-dev librsvg2-dev libayatana-appindicator3-dev libxdo-dev build-essential
```

The tray loads the AppIndicator library when the app starts, and the app stops without it, so the packages depend on it too. Provider profiles keep their credentials in a Secret Service, such as GNOME Keyring or KWallet; without one, profiles without a credential still work.

The end-to-end tests (`task e2e`) drive the webview through tauri-driver, which `task setup` installs, and the webview's own WebDriver. On Windows, `task e2e` downloads the Microsoft Edge WebDriver that matches the installed WebView2 runtime into `target/edgedriver/`; behind a proxy, set `NODE_USE_ENV_PROXY=1` so that Node.js uses `HTTPS_PROXY`. On Linux, install the WebKitWebDriver of your distribution's WebKitGTK 4.1, the `webkit2gtk-driver` package on Ubuntu and Debian, and GNOME Keyring, and run the tests in a session with a display, a D-Bus session bus and a Secret Service, as CI does: `dbus-run-session -- sh scripts/ci/with-secret-service.sh gnome-keyring xvfb-run task e2e`. On Windows, the app under test keeps out of the way: its window opens off every screen, without a taskbar button, and never takes the focus, so you can keep working while the tests run, and only its tray icon appears. To watch a test, set `MANNIE_E2E_SHOW_WINDOW=1`. `task dev` keeps its build of the app open in `target/debug/`, so to run the tests at the same time, set `CARGO_TARGET_DIR` to another absolute folder, such as the repository's `target/e2e`.

The performance suite (`task e2e:perf`) measures the targets NFR-PERF-01 to 04 and enforces them; run it on the reference machine, or set `MANNIE_PERF_REPORT_ONLY=1` to only record the figures. The live provider tests (`task provider:live`) call a real server that the `MANNIE_LIVE_*` variables name, listed in `crates/mannie-provider/tests/live.rs`, and `task provider:record` records that server's fixtures.

Mannie builds from several Cargo workspaces ([ADR-0035](docs/adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md)): the root's, with the core's crates and the desktop app's library, one in each plugin's folder, and the launcher in `.generated/desktop/`, which `task gen` writes with the desktop binary and the list of plugins. All of them build into the root's `target/`. On a fresh clone, cargo works at the root and in each plugin's folder, so `cargo build --manifest-path plugins/workers/Cargo.toml -p mannie-worker` builds the worker, but a cargo command for the desktop binary in `apps/desktop/src-tauri/` fails with "failed to read `.generated/desktop/Cargo.toml`" until `task gen` has run. `task setup` fetches the crates that every committed lock pins and then runs `task gen`, which resolves the launcher offline after that. When `task gen` says that Cargo's cache lacks a crate, run `task setup` again. A plugin whose `Cargo.toml` changed needs `task plugins:lock ID=<id>`, and `task deps:update` and `task deps:align` keep the plugins' locks on the root's versions.

The [repository](docs/engineering/repository.md) document lists the tasks.

## Workflow

1. Run `task setup` once after cloning. It installs the tools, fetches the crates and runs `task gen`.
2. For anything new, follow the [feature protocol](docs/features/README.md). An architectural change needs an [ADR](docs/adr/0000-template.md) before any code.
3. Make the change with its tests and documentation, meeting the [Definition of Done](docs/engineering/standards.md#1-definition-of-done).
4. Run `task fmt`, then `task ci`.
5. Commit with a [Conventional Commits](https://www.conventionalcommits.org/) message that has a scope, for example `feat(chat): stream reasoning summaries` or `docs(adr): accept ADR-0009`.
6. Add one line to `CHANGELOG.md` under Unreleased for every change the owner would notice, or to the plugin's `plugins/<id>/CHANGELOG.md` for a change to a plugin.

## Adding a plugin

A plugin is one folder, `plugins/<id>/`, and adding it changes no committed file outside it ([ADR-0035](docs/adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md)). `task plugins:new ID=<id>` makes the folder from the fixture plugin: its manifest, its Cargo workspace with a lock seeded from the root's, a crate that defines the plugin, a UI with its catalog, its tests and its changelog. `task gen` then bundles it into the desktop app. The plugin builds only on the plugin SDK, `mannie-plugin-sdk` in Rust and `@mannie/plugin-sdk` in its UI ([ADR-0034](docs/adr/0034-build-plugins-on-a-plugin-sdk.md)). When it needs something that the SDK does not provide, the SDK changes first. Its feature document is `plugins/<id>/docs/README.md`, and `task plugins:grants` lists what each plugin may use for the review of its change.

While the owner is the only contributor, commits need no sign-off. This changes before the repository accepts outside contributions ([ADR-0017](docs/adr/0017-license-and-dependency-policy.md)).
