# Technology stack

|              |                                                                                                                                                          |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                         |
| Last updated | 2026-10-13                                                                                                                                               |
| Related      | [ADR index](../adr/README.md), [Overview](overview.md), [Engineering standards](../engineering/standards.md), [Repository](../engineering/repository.md) |

For each building block, this document says what Mannie uses, how, and what it leaves out. The decisions themselves are in the ADRs.

## 1. Version baseline

Verified 2026-10-13. The lockfiles (`Cargo.lock`, `pnpm-lock.yaml`) and `rust-toolchain.toml` hold the exact pins from M0 on. This table mirrors them and changes with every upgrade.

| Component                             | Baseline                                                                           | License                | Notes                                                                                                                   |
| ------------------------------------- | ---------------------------------------------------------------------------------- | ---------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| Rust                                  | 1.99 stable, 2024 edition                                                          | MIT or Apache-2.0      | The next stable release, 1.100, is planned for 2026-11-12                                                               |
| Tauri (`tauri`, `@tauri-apps/api`)    | 2.12.3                                                                             | MIT or Apache-2.0      | 3.0.0-alpha.4 exists (2026-10-01) and is not used ([ADR-0003](../adr/0003-build-the-desktop-app-on-tauri-2.md))         |
| `@tauri-apps/cli`                     | 2.12.2                                                                             | MIT or Apache-2.0      |                                                                                                                         |
| `tauri-plugin-single-instance`        | 2.5.3                                                                              | MIT or Apache-2.0      |                                                                                                                         |
| specta, tauri-specta                  | 2.0.0-rc.25                                                                        | MIT                    | Release candidates since 2024, pinned exactly ([ADR-0007](../adr/0007-generate-ipc-bindings-from-rust.md))              |
| Svelte                                | 5.57.2                                                                             | MIT                    | Runes mode forced                                                                                                       |
| SvelteKit, adapter-static             | 3.0.1, 4.0.0                                                                       | MIT                    | Needs Vite 8, TypeScript 6 and Node 22 or newer                                                                         |
| Vite                                  | 8.3.4                                                                              | MIT                    |                                                                                                                         |
| TypeScript                            | 6.x                                                                                | Apache-2.0             | 7.0.2 is released, but SvelteKit 3 declares `^6.0.0`                                                                    |
| shadcn-svelte CLI                     | 1.7.0                                                                              | MIT                    | No update command ([ADR-0006](../adr/0006-use-shadcn-svelte-through-its-cli.md))                                        |
| bits-ui                               | 2.19.5                                                                             | MIT                    | Primitives under shadcn-svelte                                                                                          |
| Tailwind CSS                          | 4.3.3                                                                              | MIT                    |                                                                                                                         |
| `@lucide/svelte`                      | 1.56.0                                                                             | ISC                    | Icons                                                                                                                   |
| Paraglide JS (`@inlang/paraglide-js`) | 2.27.0                                                                             | MIT                    | Message catalogs compiled to typed message functions ([ADR-0019](../adr/0019-keep-visible-text-in-message-catalogs.md)) |
| `@sveltejs/mcp`                       | 0.1.26                                                                             | MIT                    | Svelte documentation and autofixer for coding agents                                                                    |
| tokio                                 | 1.53.2                                                                             | MIT                    |                                                                                                                         |
| reqwest                               | 0.13.5                                                                             | MIT or Apache-2.0      | rustls with ring                                                                                                        |
| rustls                                | 0.23.45                                                                            | Apache-2.0, ISC or MIT | ring is the only crypto provider ([ADR-0008](../adr/0008-one-workspace-two-binaries.md))                                |
| tonic, prost                          | 0.14.6, 0.14.4                                                                     | MIT, Apache-2.0        | The `tls-ring` feature of tonic ([ADR-0016](../adr/0016-link-machines-over-pinned-mtls-grpc.md))                        |
| rusqlite                              | 0.40.2                                                                             | MIT                    | `bundled` feature ([ADR-0009](../adr/0009-persist-state-in-sqlite.md))                                                  |
| keyring-core and stores               | 1.0.1; windows-native-keyring-store 1.1.0; zbus-secret-service-keyring-store 1.0.1 | MIT or Apache-2.0      | ([ADR-0010](../adr/0010-keep-provider-credentials-on-the-master.md))                                                    |
| tracing                               | 0.1.44                                                                             | MIT                    | With tracing-subscriber and tracing-appender                                                                            |
| schemars                              | 1.2.3                                                                              | MIT                    | Tool input schemas                                                                                                      |
| Node.js, pnpm, Task                   | 24 LTS, 11.8.0, 3.40 or newer                                                      |                        | Development tooling                                                                                                     |
| FreeBSD (worker)                      | 14.4 until 2026-12-31; 14.5 until 2027-06-30; 15.1 until 2027-03-31                |                        | FreeBSD 13 reached its end of life on 2026-04-30                                                                        |

## 2. Tauri 2

- Used: commands and channels for IPC, capabilities with `AppManifest::commands`, the single-instance and opener plugins, the system tray through Tauri's `tray-icon` feature, and the bundler for NSIS on Windows and deb and rpm on Linux. On Linux the tray needs the distribution's AppIndicator library.
- Not used: the fs, shell and http plugins, the global `__TAURI__` object, the isolation pattern, AppImage, and remote URLs in capabilities ([ADR-0004](../adr/0004-keep-system-access-in-the-rust-core.md)).
- Tauri APIs appear only in `mannie-desktop` ([ADR-0003](../adr/0003-build-the-desktop-app-on-tauri-2.md)).

## 3. SvelteKit and Svelte 5

- SvelteKit runs as a static single-page app with adapter-static and an `index.html` fallback, without server-side rendering or prerendering ([ADR-0005](../adr/0005-build-the-ui-with-sveltekit-as-a-static-spa.md)).
- Svelte 5 runes are forced for project files. `$app/state` replaces `$app/stores`.
- `svelte-check` runs in CI with zero warnings allowed.
- Paraglide JS compiles the message catalogs in `apps/desktop/messages/` into typed message functions. `sv add paraglide` sets it up, and a custom strategy reads the locale from the core's settings ([ADR-0019](../adr/0019-keep-visible-text-in-message-catalogs.md)).

## 4. shadcn-svelte, bits-ui and Tailwind CSS

- Components are added only with the pinned shadcn-svelte CLI, after reading their documentation ([ADR-0006](../adr/0006-use-shadcn-svelte-through-its-cli.md)).
- The official shadcn-svelte agent skill is installed in the repository in M0, together with the Svelte MCP server for documentation lookups and the autofixer.
- Theme tokens are CSS variables. Tailwind CSS 4 utilities are the only styling mechanism.

## 5. IPC bindings

- specta and tauri-specta generate typed TypeScript functions for commands, events and channels from the Rust types in `mannie-types` ([ADR-0007](../adr/0007-generate-ipc-bindings-from-rust.md)).
- The bindings are generated into a git-ignored folder before the frontend builds.

## 6. Async runtime and HTTP

- tokio is the async runtime of the core and the worker.
- reqwest with rustls and ring carries provider traffic. Each adapter parses server-sent events with a small parser of its own, which is fuzz-tested ([ADR-0011](../adr/0011-own-the-provider-adapters.md)).

## 7. Storage

- SQLite through rusqlite with the `bundled` feature, in WAL mode, with one writer thread and a small pool of readers ([ADR-0009](../adr/0009-persist-state-in-sqlite.md)).
- Migrations are embedded in the binary and run at startup after a copy of the database file.

## 8. Credentials

- `keyring-core` with `windows-native-keyring-store` on Windows and `zbus-secret-service-keyring-store` on Linux, on the master only ([ADR-0010](../adr/0010-keep-provider-credentials-on-the-master.md)).
- No plaintext fallback.

## 9. The link

- tonic and prost for gRPC, protox to compile the protobuf files without a system `protoc`, rustls with ring for mutual TLS, and custom verifiers that accept only pinned keys ([ADR-0016](../adr/0016-link-machines-over-pinned-mtls-grpc.md)).
- The buf CLI runs `buf lint` and `buf breaking` in CI.

## 10. Observability

- tracing with tracing-subscriber, and tracing-appender for daily rotating JSON log files. A redaction layer removes secrets.
- No telemetry, metrics export or crash reporting service.

## 11. Testing and quality tools

| Tool                                   | Use                                                                                               |
| -------------------------------------- | ------------------------------------------------------------------------------------------------- |
| cargo test                             | Rust unit and integration tests                                                                   |
| cargo-deny                             | Advisories, licenses, bans and sources ([ADR-0017](../adr/0017-license-and-dependency-policy.md)) |
| rustfmt, clippy                        | Rust formatting and lints                                                                         |
| Vitest                                 | Frontend unit tests                                                                               |
| tauri-driver (WebDriver)               | End-to-end tests of the desktop app on Windows and Linux                                          |
| axe-core                               | Accessibility checks in end-to-end tests                                                          |
| Prettier, ESLint, svelte-check         | Frontend formatting, lints and type checks                                                        |
| Prettier, markdownlint, cspell, remark | Documentation checks (`task docs:check`)                                                          |

## 12. Deliberately not used

| Not used                           | Reason                                                                                                       |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| LLM frameworks and agent libraries | The loop and the adapters are Mannie's own ([ADR-0011](../adr/0011-own-the-provider-adapters.md))            |
| Electron, or Node.js at runtime    | Tauri is the shell, and the core is Rust ([ADR-0003](../adr/0003-build-the-desktop-app-on-tauri-2.md))       |
| OpenSSL and aws-lc                 | rustls with ring builds the same way on every target ([ADR-0008](../adr/0008-one-workspace-two-binaries.md)) |
| AppImage                           | It would bundle LGPL libraries ([ADR-0017](../adr/0017-license-and-dependency-policy.md))                    |
| Tauri's fs, shell and http plugins | They give the webview direct system access ([ADR-0004](../adr/0004-keep-system-access-in-the-rust-core.md))  |
