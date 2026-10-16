# M0 validation results

|              |                                                                                                                                                                                                            |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                                           |
| Last updated | 2026-10-16                                                                                                                                                                                                 |
| Related      | [Milestones](milestones.md#4-validations), [ADR index](../adr/README.md), [Desktop app](../architecture/desktop-app.md), [Security](../architecture/security.md), [Releasing](../engineering/releasing.md) |

This document records what the validations of [M0](milestones.md#m0-foundation) showed, so that the owner can accept, change or supersede the Proposed ADRs they settle. The checks themselves stay in the repository and run in CI. Each section names the checks, where they ran, the result, and the findings that need a decision or a follow-up.

The runs below were made on 2026-10-14: on the owner's Windows 11 workstation (WebView2 154.0.4258.62), and for Linux in Ubuntu 24.04 containers (WebKitGTK 2.52.6, GNOME Keyring 46.1, KWallet 5.115, systemd 255) that ran the same commands as the CI jobs. The GitHub workflows have not run yet, because the repository has no GitHub remote; what only CI or the owner can run is marked as not run yet.

## 1. Summary

| V   | Result                                                                                                              | ADRs               | Open                                                                    |
| --- | ------------------------------------------------------------------------------------------------------------------- | ------------------ | ----------------------------------------------------------------------- |
| V1  | Passed on WebView2 and WebKitGTK, with findings                                                                     | ADR-0004, ADR-0005 | The WebKitGTK of Ubuntu 22.04                                           |
| V2  | Passed                                                                                                              | ADR-0007           | Events are not exercised until a feature needs one                      |
| V3  | Passed for the store, and for channels on WebView2 and WebKitGTK                                                    | ADR-0009, ADR-0012 | None                                                                    |
| V5  | Passed for the worker's graph, the crate bans and the FreeBSD cross-build                                           | ADR-0008           | FreeBSD 14; the worker's tests and service passed on FreeBSD 15.1 in M4 |
| V7  | Passed with Windows Credential Manager, GNOME Keyring and KWallet, and without a Secret Service                     | ADR-0010           | None                                                                    |
| V9  | Passed                                                                                                              | ADR-0017           | None                                                                    |
| V10 | The pipeline's steps pass locally, and the tag of v0.2.0 built, signed and installed the artifacts on clean runners | ADR-0018           | None                                                                    |
| V11 | Passed on WebView2 and WebKitGTK                                                                                    | ADR-0019           | None                                                                    |

## 2. V1: the webview under the security baseline

Checks: `tests/e2e/specs/security.test.ts` and `tests/e2e/specs/shell.test.ts`, run with `task e2e`.

- The shell, the settings view and the command palette render under the policy of [ADR-0004](../adr/0004-keep-system-access-in-the-rust-core.md), and the only violations are the inline styles of finding 1.
- The global `__TAURI__` object does not exist, and a command without a capability grant is refused.
- eval, the Function constructor, an inline script and an inline event handler are all refused when the app's own code tries them.
- A corpus of 29 hostile payloads runs no script and opens no connection to the local server that the test points every remote reference at. Three more payloads try to navigate, and each navigation is refused (finding 3).
- A script cannot navigate the window away from the app's origin, and `window.open` opens no window and no connection.
- axe finds no WCAG 2.1 AA violation in the light and dark themes, with the command palette open, and in the settings view; the shell works from the keyboard and shows the focus.

Result: all 30 end-to-end tests pass on WebView2 and on WebKitGTK 2.52.6. V1 names the WebKitGTK of Ubuntu 22.04, where they have not passed yet.

Findings:

1. SvelteKit's route announcer and bits-ui's command wrappers have static inline `style` attributes, which `style-src 'self'` blocks. `src/app.css` gives these elements the same presentation through selectors, so the stack works under the policy as decided, but the webview reports a violation for each blocked attribute. The test accepts exactly these values; the validation build adds `'report-sample'` to the policy so that it can tell them apart. The owner can keep this, or allow exactly these values with `style-src-attr 'unsafe-hashes'` and their hashes, which is an exception to ADR-0004 and needs a new ADR.
2. SvelteKit 3 fetches `_app/version.json` when the window gains focus, to notice a new deployment. `connect-src` refuses it and nothing depends on it, so the test ignores it.
3. The engine opens connections to the host of a navigation before the policy or the navigation guard refuses it: for a framed page, a meta refresh and a followed link, WebView2 opened 9 to 10 connections across runs, and WebKitGTK 1. The policy cannot prevent this, so the Markdown renderer of M1 must never produce such markup, and its test runs the same corpus expecting no connection. The [security](../architecture/security.md#11-assumptions-and-residual-risks) document records the residual risk.
4. The shadcn-svelte components carry English text in places, such as the narrow-screen sidebar sheet and the default label of the command list. The app avoids each one from the outside, and the window is at least 800 pixels wide ([patches](../../apps/desktop/src/lib/components/ui/PATCHES.md)).
5. The preset's muted text color gave small text a contrast of 4.3:1, below WCAG 2.1 AA. The theme token is darker now (5.5:1).
6. Scripts that WebDriver injects run outside the page's policy, and the developer tools allow eval while they run. The probes therefore live in the app's validation harness and run in a later task, as the app's own code does.
7. WebKitWebDriver reports no visible text for text in a box that truncates it, such as the header's heading. The tests read the DOM text of elements that WebDriver reports as displayed.

## 3. V2: generated IPC bindings

Checks: `task gen`, `tests/e2e/specs/ipc.test.ts`, and the Rust and TypeScript type checks.

- tauri-specta 2.0.0-rc.25 generates the bindings from the types in `mannie-types`, a crate without Tauri, into the ignored folder `apps/desktop/src/lib/generated/`. Debug builds keep them current.
- Three commands with typed errors (the interface settings and the startup status) and one `Channel<T>` stream work end to end through the settings view and the validation harness. Errors arrive as a code with arguments, and the UI shows the catalog's message for the code.
- Events are not exercised: M0 has no event, so the first feature that broadcasts one proves them.

## 4. V3: the store and the streaming load

Checks: `cargo bench -p mannie-store --features validation` (the writer benchmark), `crates/mannie-store/tests/durability.rs`, and the channel load of `tests/e2e/specs/ipc.test.ts`.

- One writer keeps up with 8 runs at 200 tokens per second with batched deltas (on the owner's workstation). With a batch every 250 ms, all 16,000 deltas were written and read back; batch commits took 0.7 ms at the median, 1.5 ms at the 95th percentile and 18.7 ms at most, and the state changes, committed with full durability as journal entries are, took 10.8 ms at the median and 17.8 ms at most. With a batch every 33 ms, batch commits took 0.4 ms at the median and 9.1 ms at most, so the writer has room for far more load.
- Killing the writer process at a different point in each of five rounds loses no acknowledged event, leaves no gap, and passes SQLite's integrity check.
- On WebView2, 8 runs of 200 deltas per second for 10 seconds arrived complete (15,992 of 15,992 deltas), with no gap and none out of order. Batches arrived every 31 ms at the median and 32.1 ms at the 95th percentile (at most 33.4 ms), and a delta reached the webview in 17 ms at the median and 33 ms at the 95th percentile.
- On WebKitGTK, the same load arrived complete (16,008 of 16,008 deltas), with no gap and none out of order. Batches arrived every 16 ms at the median and 17 ms at the 95th percentile (at most 18 ms), and a delta reached the webview in 9 ms at the median and 16 ms at the 95th percentile.

## 5. V5: the worker stays headless and builds for FreeBSD

Checks: `scripts/check-worker-dependencies.mjs` and cargo-deny in `task lint`, `cargo build --target x86_64-unknown-freebsd` in the toolchain image of cross 0.2.5, and the nightly workflow.

- None of the worker's 112 dependencies, on any target, belongs to Tauri, GTK or WebKitGTK. The same check finds 32 such crates in the desktop app's graph.
- cargo-deny allows rusqlite only through `mannie-store`, Tauri only through `mannie-desktop` and its plugins, and bans OpenSSL and aws-lc. No crypto provider is in the graph yet; ring arrives with the link.
- The worker cross-builds for `x86_64-unknown-freebsd` (a 7.2 MB ELF binary for FreeBSD).
- The worker installs as a systemd unit, runs under its account with a data folder that only the account can open, stops cleanly and uninstalls (`scripts/ci/worker-service-unix.sh`, in a container booted with systemd). The Windows service check, `scripts/ci/worker-service-windows.ps1`, needs an elevated session; it passed later on GitHub's Windows runners.
- Not run yet: the worker's tests and its rc.d service in a FreeBSD 14.4 virtual machine (`.github/workflows/nightly.yml`).

## 6. V7: credentials in the operating system's store

Checks: `crates/mannie-secrets/tests/credential_store.rs` and `crates/mannie-secrets/tests/planted_secret.rs`.

- Windows Credential Manager stores, reads, replaces and deletes a credential, including non-ASCII text.
- A planted test secret appears in no log, database or crash file of the scenario.
- On Linux, the round trip passes against GNOME Keyring and against KWallet, and so does the planted-secret scenario with the credential going through each of them. Without a Secret Service, on a session bus that cannot start one or without a session bus, the store refuses to save.
- Findings: KWallet's Secret Service needs the QCA OpenSSL plugin (`libqca-qt5-2-plugins`) for its encrypted sessions, and it asks the user once before Secret Service clients may use the open wallet; `scripts/ci/kwallet-login.py` opens the wallet as a KDE login does and records that answer. On Linux the store's calls block on their own runtime, so callers must run them on a blocking thread, as the crate documents; the planted-secret test broke that rule until V7 found it.

## 7. V9: the license policy

Checks: `cargo deny check` and `scripts/check-licenses.mjs` in `task lint`.

- cargo-deny passes advisories, bans, licenses and sources. Two advisories are accepted with their reasons in `deny.toml`: `paste` and `proc-macro-error` are unmaintained, have no known vulnerability, and run only at compile time, inside specta's macros and Tauri's GTK bindings.
- The MPL-2.0 crates in Tauri's tree (cssparser, cssparser-macros, dtoa-short and selectors) and `option-ext` are used unmodified, as ADR-0017 allows. The only GPL-family license in the graph is the LGPL alternative of r-efi, which is also available under MIT or Apache-2.0.
- WebKitGTK and GTK are linked dynamically from the distribution's packages, which the deb and rpm packages depend on; no Rust crate bundles them.
- The 141 npm packages of the desktop app all carry licenses that the policy allows.

## 8. V10: the release pipeline

Checks: `.github/workflows/release.yml`, `scripts/release.mjs` and its tests, and local runs of the build steps.

- `task release:prepare` and the release notes, checksums, signatures and their verification are covered by the tests in `tests/scripts/`.
- The NSIS installer builds on Windows (3.3 MiB). It was not installed on the owner's workstation; the release workflow installs it on a clean runner.
- The deb and rpm packages (6.5 MB each, depending on the distribution's WebKitGTK 4.1 and GTK 3) and the static worker archive (3.0 MB) build in an Ubuntu 24.04 container. With the release workflow's install check, the deb installs on a clean Ubuntu 24.04 and the rpm on a clean Fedora 44; in both the desktop app starts and keeps running, and the worker reports its version. The release builds the packages on Ubuntu 22.04 instead, the oldest supported LTS.
- cargo-about writes the Rust notices, cargo-cyclonedx and `pnpm sbom` write CycloneDX SBOMs, and `scripts/third-party-notices.mjs` writes the npm notices.
- Not run yet: the tagged run on GitHub, which builds every artifact on its platform, installs and starts each one on a clean runner, signs, and drafts the release. It needs the GitHub remote and the signing key ([releasing](../engineering/releasing.md#6-signing)), and the owner's installs on clean Windows 11, Ubuntu, Fedora and FreeBSD 14 machines complete V10.

## 9. V11: message catalogs and the pseudo-locale

Checks: `tests/e2e/specs/locale.test.ts`, `scripts/pseudo-locale.mjs` and its tests, and `inlang check`.

- Paraglide JS 2.27 compiles the catalogs inside the SvelteKit 3 app, and the `custom-core` strategy reads the locale from the core's settings.
- Changing the language reloads the window into the pseudo-locale, the choice survives a restart, and the system language can be chosen again.
- In the pseudo-locale, every visible string and every announced label in the settings view, the main view and the command palette comes from the catalog, the layout does not overflow, and axe finds no violation. A planted hard-coded string is flagged.
- The pseudo-locale is added through the project settings alone, so adding a locale needs no code change.
- Findings: language names from `Intl` now pass through a catalog message, and the command list's English default label is replaced from the catalog (V1, finding 4).

## 10. Decisions for the owner

- The ADRs that these validations settle, ADR-0004, ADR-0005, ADR-0007, ADR-0008, ADR-0009, ADR-0010, ADR-0012, ADR-0017, ADR-0018 and ADR-0019: the owner accepted them on 2026-10-16 ([D25](milestones.md#5-decisions)).
- V1, finding 1: ADR-0004 is accepted unchanged, so the blocked inline styles keep their stylesheet equivalents; allowing exactly those values would need a new ADR.
- V10: done. The signing key exists, and the tag of v0.2.0 ran the release pipeline's builds and installs.
