# Releasing

|              |                                                                                                                                                       |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                      |
| Last updated | 2026-10-13                                                                                                                                            |
| Related      | [ADR-0018](../adr/0018-release-from-tagged-builds.md), [Milestones](../roadmap/milestones.md), [Standards](standards.md), [Repository](repository.md) |

## 1. When releases happen

- M0 builds the release pipeline and proves it with a dry run (V10). Nothing is published.
- M1 ends with v0.1.0, the first build the owner installs and uses.
- M2, M3 and M4 end with v0.2.0, v0.3.0 and v0.4.0. Patch releases happen in between when a fix cannot wait.
- Every release meets the Definition of Done for its scope. The scope grows from release to release, and the quality bar stays the same.
- A later decision declares 1.0, with its own gate: desktop auto-update, a security review, user documentation, and another look at code signing (D16).

## 2. Versions

- One SemVer version covers the desktop app and the worker ([ADR-0018](../adr/0018-release-from-tagged-builds.md)).
- Versions stay at 0.x until the owner declares 1.0.
- A master works with workers one minor version behind.

## 3. Steps

1. Run `task release:prepare VERSION=0.1.0`. It sets the version in the Cargo workspace, `package.json` and `tauri.conf.json`, and turns the changelog's Unreleased section into `## 0.1.0 - <date>`. Commit the result as `chore(release): 0.1.0`.
2. Tag the commit `v0.1.0` and push the tag.
3. CI runs every check, then builds the artifacts below, computes the checksums, signs the worker artifacts, generates the SBOM and third-party notices, and creates a draft GitHub release whose notes are the changelog section (D17).
4. Install the draft's artifacts on clean Windows 11, Ubuntu LTS, Fedora and FreeBSD 14 machines, including an upgrade from the previous release's data.
5. Publish the draft.

## 4. Artifacts

| Artifact             | Format                                                    | Built on                                   |
| -------------------- | --------------------------------------------------------- | ------------------------------------------ |
| Desktop app, Windows | NSIS installer, per user, x86_64                          | Windows                                    |
| Desktop app, Linux   | deb and rpm packages, x86_64                              | The oldest supported Ubuntu LTS release    |
| Worker, Windows      | zip with `mannie-worker.exe` (`x86_64-pc-windows-msvc`)   | Windows                                    |
| Worker, Linux        | tar.gz with a static binary (`x86_64-unknown-linux-musl`) | Linux                                      |
| Worker, FreeBSD      | tar.gz (`x86_64-unknown-freebsd`)                         | FreeBSD 14                                 |
| Checksums            | SHA-256 for every artifact                                | CI                                         |
| Signatures           | Ed25519 for every worker artifact                         | CI, with a key held outside the repository |
| SBOM and notices     | CycloneDX, and generated third-party notices              | CI                                         |

## 5. Data upgrades

- Each release migrates the data of every earlier release. CI tests this with fixture databases from each previous release.
- The store copies the database before it migrates.
- A database with a newer schema than the binary knows is refused and left untouched, with a message that names both versions.

## 6. Signing

- Worker artifacts are signed with Ed25519. The master and the worker embed the public key and check it before an update is installed ([ADR-0016](../adr/0016-link-machines-over-pinned-mtls-grpc.md)).
- Windows installers are not code-signed (D16). Windows SmartScreen warns when an installer first runs, and the installation notes explain how to continue. Signing is considered again before 1.0.

## 7. Updates

- The owner updates the desktop app by installing the new installer. Desktop auto-update is a later feature with its own ADR.
- The master pushes worker updates as part of the [workers](../features/workers/README.md) feature.
