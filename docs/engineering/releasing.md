# Releasing

|              |                                                                                                                                                                                                                                                            |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                                                                                           |
| Last updated | 2026-11-10                                                                                                                                                                                                                                                 |
| Related      | [ADR-0018](../adr/0018-release-from-tagged-builds.md), [ADR-0063](../adr/0063-update-the-desktop-app-from-signed-releases.md), [Milestones](../roadmap/milestones.md), [Standards](standards.md), [Repository](repository.md), [CI runners](ci-runners.md) |

## 1. When releases happen

- M0 builds the release pipeline and proves it with a dry run (V10). Nothing is published.
- M1 ends with v0.1.0, the first build the owner installs and uses.
- M2, M3 and M4 end together with v0.2.0 ([D23](../roadmap/milestones.md#5-decisions)).
- M5, M6 and M7 end together with v0.3.0 ([D29](../roadmap/milestones.md#5-decisions), [D31](../roadmap/milestones.md#5-decisions)). Patch releases happen in between when a fix cannot wait.
- Every release meets the Definition of Done for its scope. The scope grows from release to release, and the quality bar stays the same.
- A later decision declares 1.0, with its own gate: a security review, user documentation, and another look at code signing (D16). Desktop updates were part of that gate, and M20 builds them ([ADR-0063](../adr/0063-update-the-desktop-app-from-signed-releases.md), [D55](../roadmap/milestones.md#5-decisions)).

## 2. Versions

- One SemVer version covers the desktop app and the worker ([ADR-0018](../adr/0018-release-from-tagged-builds.md)).
- Versions stay at 0.x until the owner declares 1.0.
- A master works with workers one minor version behind.

## 3. Steps

1. Run `task release:prepare VERSION=0.1.0`. It sets the version of the root's Cargo workspace, which Tauri reads for the installers, of every plugin's workspace and of the desktop app's `package.json`, records it in each workspace's `Cargo.lock`, moves the Unreleased lines of each plugin's `plugins/<id>/CHANGELOG.md` into the changelog's Unreleased section under their kinds and the plugin's name ([ADR-0035](../adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md), decision 11), and turns that section into `## 0.1.0 - <date>`. Commit the result as `chore(release): 0.1.0`.
2. Tag the commit `v0.1.0`, and push the commit to `main` with the tag or before it, as `git push origin main v0.1.0` does. The push to `main` starts the checks, and the release waits up to 90 seconds for that run to appear before it runs the checks itself.
3. The release workflow (`.github/workflows/release.yml`) checks that the tag matches the workspace version and the changelog, takes the result of every check from the `CI` run that the push of the commit started, checks the dependencies again, because advisories appear from day to day, runs the FreeBSD tests, builds the artifacts below, installs each one on a clean runner and starts it, signs the archives of the signed programs, such as the worker, generates the SBOMs and third-party notices, signs the desktop installers and writes the update manifest `latest.json` ([section 7](#7-updates)), computes the checksums, and creates or refreshes an unpublished draft GitHub release whose notes are the changelog section (D17). The install checks of the desktop app also read the kind of the installation (`nsis`, `deb` or `rpm`) in the start line of its log, because an app whose binary lacks that mark does not update itself ([ADR-0063](../adr/0063-update-the-desktop-app-from-signed-releases.md), decision 5). A commit without a `CI` run of a push, or whose run was cancelled, gets the checks in the release's own run ([CI runners](ci-runners.md#2-running-the-pipeline)). A retry replaces the draft's files and notes; a published release is refused. Every job uses a standard GitHub-hosted runner, so the artifacts are built, installed and signed on fresh machines ([CI runners](ci-runners.md)).
4. Install the draft's artifacts on clean Windows 11, Ubuntu LTS, Fedora and FreeBSD 14 machines, including an upgrade from the previous release's data.
5. Publish the draft. Publishing offers the release to the installed desktop apps, which read `latest.json` of the latest published release. A draft or a prerelease is never the latest ([section 7](#7-updates)).

When a check fails in CI while the build works and the tests pass on the owner's machines, the workflow drafts nothing, and the owner creates the release from the run's artifacts by hand, as for v0.2.0 ([D24](../roadmap/milestones.md#5-decisions)):

- The owner checks the worker archives' signatures with `node scripts/release.mjs verify release/mannie-release.pub <archives>`.
- The owner writes the update manifest with `node scripts/release.mjs manifest <folder> <version> https://github.com/limbon2/mannie/releases/download/v<version>`, which reads the release key from the environment variable `MANNIE_RELEASE_SIGNING_KEY`, and checks it with `node scripts/release.mjs verify-manifest release/mannie-release.pub <folder>`. `latest.json` goes into the release with the other files.
- A release without `latest.json` installs by hand only. Until a later release has one, the installed desktop apps say that they cannot install the latest release from here.

For a build without publishing a release, run `Release` manually on a branch from the Actions page. It runs the same builds, checks and install tests and keeps the unsigned artifacts on the run for one day. A manual branch build does not sign them or create a draft. A manual run on a version tag signs the artifacts and creates or refreshes its draft, as a tag push does.

The `artifacts` input defaults to `all`. When the owner requests `windows-freebsd`, the build omits the Linux desktop packages and their install checks, then removes the static Linux worker archive and its signature after checking all worker signatures. The draft keeps the Windows installer, the Windows and FreeBSD worker archives, their signatures, checksums, SBOMs, notices and the desktop lock. Its update manifest names only the Windows installer, so once the draft is published, a desktop app on Linux finds no package for its installation and keeps its version until a later release. All checks and the FreeBSD tests still run.

For a limited draft, include `[skip ci]` in the release commit to prevent the tag push from starting an automatic full build, then run `gh workflow run release.yml --ref v0.5.0 -f artifacts=windows-freebsd`. GitHub's [skip instructions](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/skip-workflow-runs) apply to pushes and leave manual runs available (verified on 2026-10-21). The skipped push starts no `CI` run either, so the manual run does the checks itself.

## 4. Artifacts

| Artifact             | File                                                                                                               | Built on                                   |
| -------------------- | ------------------------------------------------------------------------------------------------------------------ | ------------------------------------------ |
| Desktop app, Windows | `mannie-<version>-windows-x86_64-setup.exe`, an NSIS installer per user                                            | Windows                                    |
| Desktop app, Linux   | `mannie-<version>-linux-amd64.deb` and `mannie-<version>-linux-x86_64.rpm`                                         | The oldest supported Ubuntu LTS release    |
| Worker, Windows      | `mannie-worker-<version>-windows-x86_64.zip` (`x86_64-pc-windows-msvc`)                                            | Windows                                    |
| Worker, Linux        | `mannie-worker-<version>-linux-x86_64.tar.gz`, a static `x86_64-unknown-linux-musl` binary                         | Linux                                      |
| Worker, FreeBSD      | `mannie-worker-<version>-freebsd-x86_64.tar.gz` (`x86_64-unknown-freebsd`)                                         | FreeBSD 14                                 |
| Checksums            | `SHA256SUMS` for every artifact, in the format `sha256sum --check` reads                                           | CI                                         |
| Signatures           | `<archive>.sig`, an Ed25519 signature for every worker artifact                                                    | CI, with a key held outside the repository |
| Update manifest      | `latest.json`, which names each desktop installer of the release with its URL and its signature by the release key | CI, with the same key                      |
| SBOMs and notices    | CycloneDX SBOMs of the Rust crates and npm packages, and third-party notices                                       | CI                                         |
| Desktop lock         | `mannie-desktop-<version>.Cargo.lock`, the lock of the launcher that built the desktop app                         | CI                                         |

The programs come from the plugins' manifests, whose `[[programs]]` tables name each program's package, targets, signing, self-check and smoke test ([ADR-0035](../adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md), decision 10), and `scripts/plugins/programs.mjs` gives the workflows their builds, archives, signatures, SBOMs and notices, so a plugin's program changes no workflow. Today the worker is the one program. Each archive is `<package>-<version>-<platform>`, holds the binary and `LICENSE`, and its program answers `--version` with its package's name and the version, which the install checks compare. The desktop app builds in the launcher that `task gen` writes, whose lock `task gen` seeds from the committed locks ([ADR-0035](../adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md), decision 5), so the release keeps that lock beside the SBOMs. The desktop app's SBOM and Rust notices come from the launcher's graph, which holds the plugins' crates, and each program's from its plugin's workspace. `node scripts/release.mjs` runs the steps that are the same on every runner: the version check, the release notes, the checksums, signing and verifying, and the update manifest. It writes no manifest with an installer above 256 MiB, the most that the desktop app downloads.

## 5. Data upgrades

- Each release migrates the data of every earlier release. CI tests this with fixture databases from each previous release.
- The store copies the database before it migrates.
- A database with a newer schema than the binary knows is refused and left untouched, with a message that names both versions.

## 6. Signing

- Worker artifacts are signed with Ed25519. The worker embeds the public key, and `mannie-worker update` checks an archive's signature with it before it installs the build ([ADR-0016](../adr/0016-link-machines-over-pinned-mtls-grpc.md), [decision D20](../roadmap/milestones.md#5-decisions)). The release workflow runs `mannie-worker update --check` on every signed archive, which proves that the built-in key is the release key.
- Before the first release, the owner creates the key pair with `node scripts/release.mjs keygen <folder>`, stores the private key as the repository secret `MANNIE_RELEASE_SIGNING_KEY`, keeps an offline copy, deletes the file, and commits the public key as `release/mannie-release.pub`. The release workflow signs with the secret and checks every signature against the committed public key, so a release fails until both exist.
- The release key also signs the desktop installers for the updater ([ADR-0063](../adr/0063-update-the-desktop-app-from-signed-releases.md), decision 4). These are minisign signatures of the same Ed25519 key, whose trusted comment names the file and the version, and `latest.json` holds them. The desktop app holds the public key in `plugins.updater.pubkey` of `tauri.conf.json`, in the form that `node scripts/release.mjs updater-key release/mannie-release.pub` prints, and a test keeps the two equal. A build needs no key.
- Windows installers are not code-signed (D16). Windows SmartScreen warns when an installer first runs, and the installation notes explain how to continue. Signing is considered again before 1.0.

## 7. Updates

- An installed desktop app updates itself from the latest published release ([updates](../features/updates.md), [ADR-0063](../adr/0063-update-the-desktop-app-from-signed-releases.md)). It downloads the installer that `latest.json` names for its kind of installation, keeps it only with a valid signature by the release key, and installs it when the owner chooses "Restart and update".
- The owner installs the first release with the updater by hand, as every release on an installation that does not update itself, such as a binary built from the source.
- The owner updates a worker on its host: they download the archive of the new release and its signature, and run `mannie-worker update` with administrator rights ([compatibility and updates](../../plugins/workers/docs/compatibility-and-updates.md#updating-a-worker)). The master pushes no builds (D20), and it says when a worker needs an update.
