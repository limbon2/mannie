# CI runners

|              |                                                                                                                                            |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Draft for review                                                                                                                           |
| Last updated | 2026-10-21                                                                                                                                 |
| Related      | [Decision record](../roadmap/milestones.md#5-decisions), [Repository](repository.md), [Releasing](releasing.md), [Standards](standards.md) |

Every workflow uses standard GitHub-hosted runners. Mannie's repository is public, so these runners have no minute charge. No job depends on the owner's workstation, a local controller or a registered self-hosted runner.

## 1. Where the jobs run

| Work                                                                          | Runner                                         |
| ----------------------------------------------------------------------------- | ---------------------------------------------- |
| Windows checks, end-to-end tests, service tests, installer and worker archive | `windows-2025`                                 |
| Linux checks, credential stores, service tests and static worker archive      | `ubuntu-24.04`                                 |
| Linux desktop packages and end-to-end tests on the oldest supported WebKitGTK | `ubuntu-22.04`                                 |
| FreeBSD tests, native worker build and install check                          | FreeBSD 14.4 virtual machine on `ubuntu-24.04` |
| Signatures, checksums, SBOMs, notices and draft release                       | `ubuntu-24.04`                                 |

GitHub provides a fresh machine for each job. The FreeBSD jobs start a virtual machine through the pinned [FreeBSD VM action](https://github.com/vmactions/freebsd-vm). The worker is built inside FreeBSD, as [ADR-0018](../adr/0018-release-from-tagged-builds.md) requires. Desktop builds target Windows and Linux; FreeBSD runs the headless worker.

Windows browser automation runs with restricted privileges at medium integrity. This uses the installed WebView2 runtime and its matching EdgeDriver. Verified on 2026-10-21: [WebView2 blocks its debugging endpoint for elevated applications](https://github.com/MicrosoftEdge/WebView2Feedback/issues/5639). The helper gives the runner account write access to the checkout so the debug app can export its generated bindings.

## 2. Running the pipeline

- Pull requests and pushes to `main` run `CI`: `task ci` on Windows and Linux, both webviews, the credential stores, the plugin programs' service tests and cross-builds, and the core's FreeBSD checks. A new push cancels the obsolete run for that branch.
- The `FreeBSD` workflow runs native FreeBSD tests for pull requests that change worker code, and `Nightly` runs them every night.
- The `Release` workflow can be run manually from the Actions page. It builds the Windows installer, Linux packages and worker archives for all three platforms, runs the checks and installs the artifacts on fresh machines. The unsigned artifacts are downloads on that run, retained for one day. A manual branch build creates no release and receives no signing key.
- Pushing a prepared `vX.Y.Z` tag runs the same release pipeline, then signs the worker archives and creates a draft release after all checks and install tests pass. The draft holds the durable downloads. The owner publishes it after the [release checks](releasing.md#3-steps).
- For a repair, run the CI workflow manually with scope Rust or end-to-end tests and select the platform. Rust takes Cargo arguments as a JSON array with a package, an exact test or filter, and the locked dependencies. End-to-end tests accept one repository-relative spec file. These runs skip the unrelated jobs and performance measurements. Develop repairs on a branch without a pull request, then update the pull request once the focused checks pass.
- If code has not changed, use GitHub's Re-run failed jobs instead of starting the whole workflow again.
- The live provider tests and evaluations run separately. Their configured providers can charge for model requests; they do not block a release.

Builds and tests run side by side across jobs. Each job has a timeout. A failed job reports its failing command, and GitHub can rerun that job without repeating successful jobs. Hosted end-to-end tests show their window on the disposable desktop so the webview paints normally. Before the end-to-end suite, one startup check opens a browser session and reaches the app. A driver that cannot start fails that check once; it does not retry the same startup failure for every test file. A cold Rust build can still take tens of minutes; hosted runners remove the local machine queue and maintenance rather than promising a fixed run time.

## 3. Cost and storage

Verified on 2026-10-21 against [GitHub's Actions billing documentation](https://docs.github.com/en/billing/concepts/product-billing/github-actions):

- Standard GitHub-hosted runner minutes are free in public repositories. Larger runners are charged even for public repositories; no workflow uses one.
- Keep the repository's cache limit at the included 10 GB. Old caches may be evicted, which makes a later build colder rather than preventing it from running.
- The pinned sccache compiler wrapper stores completed Rust compilations in GitHub's cache as they finish, including before a later test fails. Results are keyed by compiler inputs, so unchanged libraries can be reused across fresh runner machines. Clippy's metadata stays in its own build cache because sccache does not cache metadata-only compilation. The Cargo registry and pnpm package store are cached too.
- Store debug compiler results through sccache instead of keeping a complete build folder for every job. This leaves more room for reusable results within the cache limit.
- Release intermediates expire after one day. Published release files remain in GitHub Releases. Performance measurements go into the job log and summary, which do not consume artifact storage.
- Making the repository private again restores GitHub's private-repository allowances and billing. Reconsider that cost before changing visibility.

Verified on 2026-10-21 against the [sccache GitHub Actions integration](https://github.com/Mozilla-Actions/sccache-action) and [Rust caching limits](https://github.com/mozilla/sccache/blob/main/docs/Rust.md). Compiler caching speeds compilation; tests still execute. A cold compiler cache and linking still take time.

## 4. Security and maintenance

- Every action is pinned by commit SHA, Rust by `rust-toolchain.toml`, and pnpm dependencies by the committed lock. Keep pins current in dedicated dependency changes.
- Jobs have read-only repository access. Only the job that signs and drafts a tagged release gets write access and the release signing key. Pull-request jobs never receive that key.
- Require approval for workflow runs from every outside contributor. Use `pull_request`, never `pull_request_target`, to test their code. The live providers and evaluations run only on their schedules or through an owner's manual run.
- No self-hosted runner serves this public repository. Stop the old Hyper-V controller and remove its runner registrations when switching to hosted builds. Existing VM images can remain powered off.
- Before publication, scan all Git history for credentials and inspect matches. Do not publish a real credential, an owner's runtime database, a private key or a local environment file.
