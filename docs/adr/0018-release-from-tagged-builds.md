# ADR-0018: Release the desktop app and the worker together from tagged builds

|          |                                                                                                                                                      |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted                                                                                                                                             |
| Date     | 2026-10-13                                                                                                                                           |
| Deciders | Owner                                                                                                                                                |
| Related  | [Releasing](../engineering/releasing.md), [ADR-0017](0017-license-and-dependency-policy.md), [ADR-0016](0016-link-machines-over-pinned-mtls-grpc.md) |

## Context

- Every milestone from M1 on ends with a 0.x release ([decision D15](../roadmap/milestones.md#5-decisions)).
- The desktop app runs on Windows and Linux, and the worker on Windows, Linux and FreeBSD (D6).
- Tauri's bundler builds NSIS and MSI installers on Windows, and deb, rpm and AppImage packages on Linux. The license policy rules out AppImage ([ADR-0017](0017-license-and-dependency-policy.md)).
- The master pushes updates to workers, so a worker must be able to verify who built a binary as well as whether it arrived intact.

## Decision

1. The desktop app and the worker share one SemVer version, which stays below 1.0 until the owner declares 1.0 (D15).
2. Only CI builds releases, from tags `vX.Y.Z` on `main`.
3. Each release has these artifacts:
   - for the Windows desktop, an NSIS installer for x86_64 that installs per user, without administrator rights;
   - for the Linux desktop, deb and rpm packages for x86_64, built on the oldest supported Ubuntu LTS release;
   - for the worker, builds for `x86_64-pc-windows-msvc` (zip), `x86_64-unknown-linux-musl` (tar.gz, static) and `x86_64-unknown-freebsd` (tar.gz, built on FreeBSD 14).
4. Every artifact has a SHA-256 checksum, and worker artifacts are signed with Ed25519. The signing key is kept outside the repository, and its public key is embedded in the worker, whose `update` command checks it ([decision D20](../roadmap/milestones.md#5-decisions)). Releases include a CycloneDX SBOM and third-party notices.
5. CI creates a draft GitHub release (D17) whose notes are the version's changelog section. The owner publishes it after the smoke installs pass.
6. Code signing of the Windows installer follows decision D16.
7. Desktop auto-update is a later feature with its own ADR.

## Consequences

- Positive: every release is built the same way, and workers can verify what they install.
- Negative: the owner has to manage a release signing key, and unsigned Windows installers trigger SmartScreen warnings (risk R14).
- Refinement in M4: the owner applies worker updates on the host, and the master pushes no builds (D20), so only the worker embeds the release key.

## Alternatives considered

| Option                                   | Why not chosen                                                             |
| ---------------------------------------- | -------------------------------------------------------------------------- |
| MSI installers                           | Need the WiX toolset and are made for machine-wide installs                |
| AppImage                                 | Bundles LGPL libraries ([ADR-0017](0017-license-and-dependency-policy.md)) |
| Separate versions for desktop and worker | Multiplies the version combinations to test                                |

## Validation

V10 runs the whole pipeline in M0, and every release runs it again.
