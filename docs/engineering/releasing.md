# Releasing

|              |                                                                                                                                                       |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                      |
| Last updated | 2026-10-16                                                                                                                                            |
| Related      | [ADR-0018](../adr/0018-release-from-tagged-builds.md), [Milestones](../roadmap/milestones.md), [Standards](standards.md), [Repository](repository.md) |

## 1. When releases happen

- M0 builds the release pipeline and proves it with a dry run (V10). Nothing is published.
- M1 ends with v0.1.0, the first build the owner installs and uses.
- M2, M3 and M4 end together with v0.2.0 ([D23](../roadmap/milestones.md#5-decisions)). Patch releases happen in between when a fix cannot wait.
- Every release meets the Definition of Done for its scope. The scope grows from release to release, and the quality bar stays the same.
- A later decision declares 1.0, with its own gate: desktop auto-update, a security review, user documentation, and another look at code signing (D16).

## 2. Versions

- One SemVer version covers the desktop app and the worker ([ADR-0018](../adr/0018-release-from-tagged-builds.md)).
- Versions stay at 0.x until the owner declares 1.0.
- A master works with workers one minor version behind.

## 3. Steps

1. Run `task release:prepare VERSION=0.1.0`. It sets the version of the Cargo workspace, which Tauri reads for the installers, and of the desktop app's `package.json`, records it in `Cargo.lock`, and turns the changelog's Unreleased section into `## 0.1.0 - <date>`. Commit the result as `chore(release): 0.1.0`.
2. Tag the commit `v0.1.0` and push the tag.
3. The release workflow (`.github/workflows/release.yml`) checks that the tag matches the workspace version and the changelog, runs every check and the FreeBSD tests, builds the artifacts below, installs each one on a clean runner and starts it, signs the worker artifacts, generates the SBOMs and third-party notices, computes the checksums, and creates a draft GitHub release whose notes are the changelog section (D17).
4. Install the draft's artifacts on clean Windows 11, Ubuntu LTS, Fedora and FreeBSD 14 machines, including an upgrade from the previous release's data.
5. Publish the draft. When a check fails in CI while the build works and the tests pass on the owner's machines, the workflow drafts nothing. The owner then checks the worker archives' signatures with `node scripts/release.mjs verify release/mannie-release.pub <archives>` and creates the release from the run's artifacts by hand, as for v0.2.0 ([D24](../roadmap/milestones.md#5-decisions)).

## 4. Artifacts

| Artifact             | File                                                                                       | Built on                                   |
| -------------------- | ------------------------------------------------------------------------------------------ | ------------------------------------------ |
| Desktop app, Windows | `mannie-<version>-windows-x86_64-setup.exe`, an NSIS installer per user                    | Windows                                    |
| Desktop app, Linux   | `mannie-<version>-linux-amd64.deb` and `mannie-<version>-linux-x86_64.rpm`                 | The oldest supported Ubuntu LTS release    |
| Worker, Windows      | `mannie-worker-<version>-windows-x86_64.zip` (`x86_64-pc-windows-msvc`)                    | Windows                                    |
| Worker, Linux        | `mannie-worker-<version>-linux-x86_64.tar.gz`, a static `x86_64-unknown-linux-musl` binary | Linux                                      |
| Worker, FreeBSD      | `mannie-worker-<version>-freebsd-x86_64.tar.gz` (`x86_64-unknown-freebsd`)                 | FreeBSD 14                                 |
| Checksums            | `SHA256SUMS` for every artifact, in the format `sha256sum --check` reads                   | CI                                         |
| Signatures           | `<archive>.sig`, an Ed25519 signature for every worker artifact                            | CI, with a key held outside the repository |
| SBOMs and notices    | CycloneDX SBOMs of the Rust crates and npm packages, and third-party notices               | CI                                         |

Each worker archive holds the binary and `LICENSE`. `node scripts/release.mjs` runs the steps that are the same on every runner: the version check, the release notes, the checksums, and signing and verifying.

## 5. Data upgrades

- Each release migrates the data of every earlier release. CI tests this with fixture databases from each previous release.
- The store copies the database before it migrates.
- A database with a newer schema than the binary knows is refused and left untouched, with a message that names both versions.

## 6. Signing

- Worker artifacts are signed with Ed25519. The worker embeds the public key, and `mannie-worker update` checks an archive's signature with it before it installs the build ([ADR-0016](../adr/0016-link-machines-over-pinned-mtls-grpc.md), [decision D20](../roadmap/milestones.md#5-decisions)). The release workflow runs `mannie-worker update --check` on every signed archive, which proves that the built-in key is the release key.
- Before the first release, the owner creates the key pair with `node scripts/release.mjs keygen <folder>`, stores the private key as the repository secret `MANNIE_RELEASE_SIGNING_KEY`, keeps an offline copy, deletes the file, and commits the public key as `release/mannie-release.pub`. The release workflow signs with the secret and checks every signature against the committed public key, so a release fails until both exist.
- Windows installers are not code-signed (D16). Windows SmartScreen warns when an installer first runs, and the installation notes explain how to continue. Signing is considered again before 1.0.

## 7. Updates

- The owner updates the desktop app by installing the new installer. Desktop auto-update is a later feature with its own ADR.
- The owner updates a worker on its host: they download the archive of the new release and its signature, and run `mannie-worker update` with administrator rights ([compatibility and updates](../features/workers/compatibility-and-updates.md#updating-a-worker)). The master pushes no builds (D20), and it says when a worker needs an update.
