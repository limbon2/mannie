# Technology stack

|              |                                                                                                                                                          |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                         |
| Last updated | 2026-10-14                                                                                                                                               |
| Related      | [ADR index](../adr/README.md), [Overview](overview.md), [Engineering standards](../engineering/standards.md), [Repository](../engineering/repository.md) |

For each building block, this document says what Mannie uses, how, and what it leaves out. The decisions themselves are in the ADRs.

## 1. Version baseline

Verified 2026-10-13; the rows for the tools M0 added, from mode-watcher on, were verified 2026-10-14. The lockfiles (`Cargo.lock`, `pnpm-lock.yaml`) and `rust-toolchain.toml` hold the exact pins. This table mirrors them and changes with every upgrade.

| Component                                                               | Baseline                                                                           | License                | Notes                                                                                                                                                              |
| ----------------------------------------------------------------------- | ---------------------------------------------------------------------------------- | ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Rust                                                                    | 1.99 stable, 2024 edition                                                          | MIT or Apache-2.0      | The next stable release, 1.100, is planned for 2026-11-12                                                                                                          |
| Tauri (`tauri`, `@tauri-apps/api`)                                      | 2.12.3                                                                             | MIT or Apache-2.0      | 3.0.0-alpha.4 exists (2026-10-01) and is not used ([ADR-0003](../adr/0003-build-the-desktop-app-on-tauri-2.md))                                                    |
| `@tauri-apps/cli`                                                       | 2.12.2                                                                             | MIT or Apache-2.0      |                                                                                                                                                                    |
| `tauri-plugin-single-instance`                                          | 2.5.3                                                                              | MIT or Apache-2.0      |                                                                                                                                                                    |
| specta, tauri-specta                                                    | 2.0.0-rc.25                                                                        | MIT                    | Release candidates since 2024, pinned exactly ([ADR-0007](../adr/0007-generate-ipc-bindings-from-rust.md))                                                         |
| Svelte                                                                  | 5.57.2                                                                             | MIT                    | Runes mode forced                                                                                                                                                  |
| SvelteKit, adapter-static                                               | 3.0.1, 4.0.0                                                                       | MIT                    | Needs Vite 8, TypeScript 6 and Node 22 or newer                                                                                                                    |
| Vite                                                                    | 8.3.4                                                                              | MIT                    |                                                                                                                                                                    |
| TypeScript                                                              | 6.0.3                                                                              | Apache-2.0             | 7.0.2 is released, but SvelteKit 3 declares `^6.0.0`                                                                                                               |
| shadcn-svelte CLI                                                       | 1.7.0                                                                              | MIT                    | No update command ([ADR-0006](../adr/0006-use-shadcn-svelte-through-its-cli.md))                                                                                   |
| bits-ui                                                                 | 2.19.5                                                                             | MIT                    | Primitives under shadcn-svelte                                                                                                                                     |
| Tailwind CSS                                                            | 4.3.3                                                                              | MIT                    |                                                                                                                                                                    |
| `@lucide/svelte`                                                        | 1.55.0                                                                             | ISC                    | Icons. 1.56.0 was younger than pnpm's minimum release age when M0 pinned the icons                                                                                 |
| Paraglide JS (`@inlang/paraglide-js`)                                   | 2.27.0                                                                             | MIT                    | Message catalogs compiled to typed message functions ([ADR-0019](../adr/0019-keep-visible-text-in-message-catalogs.md))                                            |
| `@sveltejs/mcp`                                                         | 0.1.26                                                                             | MIT                    | Svelte documentation and autofixer for coding agents                                                                                                               |
| tokio                                                                   | 1.53.2                                                                             | MIT                    |                                                                                                                                                                    |
| reqwest                                                                 | 0.13.5                                                                             | MIT or Apache-2.0      | rustls with ring                                                                                                                                                   |
| rustls                                                                  | 0.23.45                                                                            | Apache-2.0, ISC or MIT | ring is the only crypto provider ([ADR-0008](../adr/0008-one-workspace-two-binaries.md))                                                                           |
| rustls-platform-verifier                                                | 0.7.1                                                                              | MIT or Apache-2.0      | The operating system's trust store for provider connections, through reqwest                                                                                       |
| bytes, futures-util                                                     | 1.12.1, 0.3.34                                                                     | MIT; MIT or Apache-2.0 | Streamed response bodies of providers                                                                                                                              |
| fastrand                                                                | 2.5.0                                                                              | Apache-2.0 or MIT      | The jitter of retries, with a fixed seed in tests                                                                                                                  |
| sha2                                                                    | 0.10.9                                                                             | MIT or Apache-2.0      | The content hash of checkpoints                                                                                                                                    |
| url                                                                     | 2.5.8                                                                              | MIT or Apache-2.0      | Base URLs of providers and links from model output                                                                                                                 |
| `tauri-plugin-opener`                                                   | 2.7.1                                                                              | Apache-2.0 or MIT      | Its Rust API opens a confirmed link in the default browser; the webview gets no permission of it                                                                   |
| `mdast-util-from-markdown`, `micromark-extension-gfm`, `mdast-util-gfm` | 2.1.0, 3.0.0, 3.1.0                                                                | MIT                    | Model output parsed as Markdown with micromark and rendered by Mannie's own components ([ADR-0021](../adr/0021-render-model-output-as-markdown-with-micromark.md)) |
| `decode-named-character-reference`                                      | 1.3.0                                                                              | MIT                    | Resolved to its build without the DOM, which never assigns `innerHTML`                                                                                             |
| proptest                                                                | 1.12.0                                                                             | MIT or Apache-2.0      | Property tests of the event parser and the event log; never shipped                                                                                                |
| tonic, prost                                                            | 0.14.6, 0.14.4                                                                     | MIT, Apache-2.0        | The `tls-ring` feature of tonic ([ADR-0016](../adr/0016-link-machines-over-pinned-mtls-grpc.md))                                                                   |
| rusqlite                                                                | 0.40.2                                                                             | MIT                    | `bundled` feature ([ADR-0009](../adr/0009-persist-state-in-sqlite.md))                                                                                             |
| keyring-core and stores                                                 | 1.0.1; windows-native-keyring-store 1.1.0; zbus-secret-service-keyring-store 1.0.1 | MIT or Apache-2.0      | ([ADR-0010](../adr/0010-keep-provider-credentials-on-the-master.md))                                                                                               |
| tracing                                                                 | 0.1.44                                                                             | MIT                    | With tracing-subscriber and tracing-appender                                                                                                                       |
| schemars                                                                | 1.2.3                                                                              | MIT                    | Tool input schemas                                                                                                                                                 |
| Node.js, pnpm, Task                                                     | 24 LTS, 11.8.0, 3.40 or newer                                                      |                        | Development tooling                                                                                                                                                |
| FreeBSD (worker)                                                        | 14.4 until 2026-12-31; 14.5 until 2027-06-30; 15.1 until 2027-03-31                |                        | FreeBSD 13 reached its end of life on 2026-04-30                                                                                                                   |
| mode-watcher                                                            | 1.1.0                                                                              | MIT                    | Light, dark and system themes, with the inline head script turned off for the Content Security Policy                                                              |
| clap                                                                    | 4.6.7                                                                              | MIT or Apache-2.0      | The worker's command line                                                                                                                                          |
| windows-service                                                         | 0.8.1                                                                              | MIT or Apache-2.0      | The worker as a Windows service                                                                                                                                    |
| WebdriverIO                                                             | 10.0.2                                                                             | MIT                    | End-to-end tests, through tauri-driver                                                                                                                             |
| axe-core                                                                | 4.14.0                                                                             | MPL-2.0                | Accessibility checks in the end-to-end tests; never shipped                                                                                                        |
| cargo-deny                                                              | 0.20.2                                                                             | MIT or Apache-2.0      | `task lint`                                                                                                                                                        |
| tauri-driver                                                            | 2.1.0                                                                              | MIT or Apache-2.0      | `task e2e`                                                                                                                                                         |
| cargo-about, cargo-cyclonedx                                            | 0.9.2, 0.5.9                                                                       | MIT or Apache-2.0      | Third-party notices and SBOMs of a release                                                                                                                         |
| cross                                                                   | 0.2.5                                                                              | MIT or Apache-2.0      | The worker's FreeBSD build in CI                                                                                                                                   |
| skills CLI (`skills`)                                                   | 1.7.2                                                                              | MIT                    | Installs the shadcn-svelte agent skill ([ADR-0006](../adr/0006-use-shadcn-svelte-through-its-cli.md))                                                              |

## 2. Tauri 2

- Used: commands and channels for IPC, capabilities with `AppManifest::commands`, the single-instance plugin, the opener crate's Rust function for confirmed links (the plugin itself is not registered, so the webview cannot call it), the system tray through Tauri's `tray-icon` feature, and the bundler for NSIS on Windows and deb and rpm on Linux. On Linux the tray needs the distribution's AppIndicator library.
- Not used: the fs, shell and http plugins, the global `__TAURI__` object, the isolation pattern, AppImage, and remote URLs in capabilities ([ADR-0004](../adr/0004-keep-system-access-in-the-rust-core.md)).
- Tauri APIs appear only in `mannie-desktop` ([ADR-0003](../adr/0003-build-the-desktop-app-on-tauri-2.md)).

## 3. SvelteKit and Svelte 5

- SvelteKit runs as a static single-page app with adapter-static and an `index.html` fallback, without server-side rendering or prerendering ([ADR-0005](../adr/0005-build-the-ui-with-sveltekit-as-a-static-spa.md)).
- Svelte 5 runes are forced for project files. `$app/state` replaces `$app/stores`.
- `svelte-check` runs in CI with zero warnings allowed.
- SvelteKit 3 takes its configuration in the `sveltekit()` call of `vite.config.ts`, and the app imports its own modules through the `#lib/*` subpath import, which replaces `$lib`.
- Paraglide JS compiles the message catalogs in `apps/desktop/messages/` into typed message functions: its Vite plugin on every build, and `task gen` for the type checks. A custom strategy reads the locale from the core's settings ([ADR-0019](../adr/0019-keep-visible-text-in-message-catalogs.md)).

## 4. shadcn-svelte, bits-ui and Tailwind CSS

- Components are added only with the pinned shadcn-svelte CLI, after reading their documentation ([ADR-0006](../adr/0006-use-shadcn-svelte-through-its-cli.md)).
- The official shadcn-svelte agent skill is vendored in the repository, and the Svelte MCP server is configured for documentation lookups and the autofixer ([repository](../engineering/repository.md)).
- Theme tokens are CSS variables. Tailwind CSS 4 utilities are the only styling mechanism.
- Model output is Markdown, parsed by micromark into a syntax tree and rendered by Mannie's own components, which never insert markup: raw HTML shows as text, images are never loaded, and links open only after the owner confirms them ([ADR-0021](../adr/0021-render-model-output-as-markdown-with-micromark.md)).

## 5. IPC bindings

- specta and tauri-specta generate typed TypeScript functions for commands, events and channels from the Rust types in `mannie-types` ([ADR-0007](../adr/0007-generate-ipc-bindings-from-rust.md)).
- The bindings are generated into a git-ignored folder before the frontend builds.

## 6. Async runtime and HTTP

- tokio is the async runtime of the core and the worker.
- reqwest with rustls and ring carries provider traffic, checks certificates against the operating system's trust store through rustls-platform-verifier, and follows no redirects. The adapters parse server-sent events with a small parser of Mannie's own, which property tests with proptest check against every way a stream can be split ([ADR-0011](../adr/0011-own-the-provider-adapters.md)).

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
| WebdriverIO with tauri-driver          | End-to-end tests of the desktop app on WebView2 and WebKitGTK                                     |
| inlang CLI                             | Checks of the message catalogs                                                                    |
| cargo-about, cargo-cyclonedx, pnpm     | Third-party notices and CycloneDX SBOMs of a release                                              |
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
