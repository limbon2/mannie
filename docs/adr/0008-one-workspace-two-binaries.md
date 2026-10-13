# ADR-0008: Build a desktop binary and a headless worker from one Cargo workspace

|          |                                                                                                                                                             |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Proposed                                                                                                                                                    |
| Date     | 2026-10-13                                                                                                                                                  |
| Deciders | Owner                                                                                                                                                       |
| Related  | [ADR-0003](0003-build-the-desktop-app-on-tauri-2.md), [ADR-0015](0015-run-an-agent-runtime-on-every-machine.md), [Repository](../engineering/repository.md) |

## Context

Verified 2026-10-13:

- The desktop app and the worker share the agent runtime, the tools, the store and the link ([ADR-0015](0015-run-an-agent-runtime-on-every-machine.md)).
- On Linux, a Tauri binary links GTK and WebKitGTK when it starts. One binary with a role flag would need GUI libraries on headless servers, and Tauri does not target FreeBSD.
- rustls uses the aws-lc-rs crypto provider by default, which needs a C toolchain and CMake to build. The ring provider needs less build tooling, which matters on FreeBSD.
- FreeBSD 13 reached its end of life on 2026-04-30. FreeBSD 14.4 is supported until 2026-12-31, 14.5 until 2027-06-30 and 15.1 until 2027-03-31.
- Windows 10 reached its end of support on 2025-10-14.
- The current stable Rust release is 1.99.0.

## Decision

1. One Cargo workspace holds the library crates in `crates/` and the binaries in `apps/`, as [repository](../engineering/repository.md) describes.
2. `mannie-desktop` (Tauri) and `mannie-worker` (headless) are thin composition roots, and the logic lives in library crates.
3. cargo-deny bans enforce the boundaries. Only `mannie-desktop` may depend on `tauri`, and only `mannie-store` on `rusqlite`. ring is the only crypto provider, so `aws-lc-sys` is banned. `openssl-sys` is banned because rustls is used everywhere.
4. Platform baselines:
   - Desktop: Windows 11 x86_64 with the WebView2 Evergreen runtime, and Linux x86_64 with WebKitGTK 4.1 (Ubuntu 22.04 LTS and newer, Debian 12 and newer, current Fedora releases).
   - Worker: Windows 11, Windows Server 2022 and Windows Server 2025 (`x86_64-pc-windows-msvc`); Linux x86_64 as a static binary (`x86_64-unknown-linux-musl`); FreeBSD 14 and 15 on amd64 (`x86_64-unknown-freebsd`), built on the oldest supported 14.x release.
5. `rust-toolchain.toml` pins a stable toolchain, 1.99 at the time of writing, and the code uses the 2024 edition.
6. Every pull request cross-builds the worker for FreeBSD. The worker's tests run in a FreeBSD virtual machine every night and for every release tag.

## Consequences

- Positive: the worker has no GUI dependencies, and tooling enforces the crate boundaries.
- Negative: there are two composition roots, and the musl and FreeBSD builds need care in CI (risk R10).

## Alternatives considered

| Option                                       | Why not chosen                                    |
| -------------------------------------------- | ------------------------------------------------- |
| One binary with a role flag                  | Puts GUI libraries on servers                     |
| Separate repositories for desktop and worker | Duplicates the runtime and invites version skew   |
| aws-lc-rs as the crypto provider             | Heavier build requirements, especially on FreeBSD |

## Validation

V5.
