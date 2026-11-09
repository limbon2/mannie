# CI runners

|              |                                                                                                                                            |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Draft for review                                                                                                                           |
| Last updated | 2026-11-10                                                                                                                                 |
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

- Pull requests and pushes to `main` run `CI`: the checks of `task ci` on Windows and Linux, the end-to-end specs on both webviews, the performance measurements, the credential stores, the plugin programs' service tests and cross-builds, and the core's FreeBSD checks. A failed job stops no other, so one run reports every failure. A new push cancels the obsolete run for that branch.
- The checks of `task ci` run as the groups of `scripts/ci.mjs`, each in a job of its own, and a job's name is the command that repeats it on a workstation, such as `task ci -- core`:

  | Group     | Checks                                                                                                          | Builds                                        |
  | --------- | --------------------------------------------------------------------------------------------------------------- | --------------------------------------------- |
  | `static`  | The documentation, the tests of the scripts, the contracts, the TOML files, the formatting and the dependencies | Nothing                                       |
  | `desktop` | ESLint, svelte-check, the message catalogs, Vitest and the Rust tests of the desktop binary, after `task gen`   | The bindings' programs and the desktop binary |
  | `clippy`  | Clippy in every workspace and the launcher, after `task gen`                                                    | The bindings' programs                        |
  | `core`    | The Rust tests and doctests of the root workspace                                                               | Its tests                                     |
  | `plugins` | The Rust tests and doctests of each plugin's workspace                                                          | Their tests                                   |

  Windows runs one job for each group. Linux builds about twice as fast, so `desktop` and `clippy` share one job and `core` and `plugins` another. A test of the script fails when a step of `task ci` belongs to no group, or when a platform's jobs miss a group.

- The end-to-end specs run in two parts on each webview, each in a job that builds the validation app, so the suite takes as long as its longer part. `MANNIE_E2E_SHARD=1/2` names a part (`tests/e2e/support/shard.ts`); WebdriverIO sorts the spec files and gives each part the next share of them. The performance measurements, which CI only records, run in a job of their own.
- The `FreeBSD` workflow runs native FreeBSD tests for pull requests that change worker code, and `Nightly` runs them every night.
- The `Release` workflow can be run manually from the Actions page. It builds the Windows installer, Linux packages and worker archives for all three platforms, runs the checks and installs the artifacts on fresh machines. The unsigned artifacts are downloads on that run, retained for one day. A manual branch build creates no release and receives no signing key.
- Pushing a prepared `vX.Y.Z` tag runs the same release pipeline, then signs the worker archives and creates a draft release after all checks and install tests pass. The draft holds the durable downloads. The owner publishes it after the [release checks](releasing.md#3-steps).
- A release takes its checks from the `CI` run that the push of its commit to `main` started, and waits for that run while it builds. A tag that is pushed with its commit would otherwise run every check twice at the same time. The release runs the checks itself when the commit has no such run, as in a manual build of a branch, and when the run was cancelled, as a later push to `main` does to it (`scripts/ci/commit-checks.mjs`). It always checks the dependencies itself, because a tag can follow its commit by days and advisories appear in between.
- For a repair, run the CI workflow manually with scope Rust or end-to-end tests and select the platform. Rust takes Cargo arguments as a JSON array with a package, an exact test or filter, and the locked dependencies. End-to-end tests accept one repository-relative spec file. These runs skip the unrelated jobs and performance measurements. Develop repairs on a branch without a pull request, then update the pull request once the focused checks pass.
- If code has not changed, use GitHub's Re-run failed jobs instead of starting the whole workflow again. When that repairs the `CI` run of a release's commit, re-run the release's job "Checks of the commit's push to main" afterwards, which then reads the new result.
- The live provider tests and evaluations run separately. Their configured providers can charge for model requests; they do not block a release.

Builds and tests run side by side across jobs. Each job has a timeout. A failed job reports its failing command, and GitHub can rerun that job without repeating successful jobs. Hosted end-to-end tests show their window on the disposable desktop so the webview paints normally. Before the end-to-end suite, one startup check opens a browser session and reaches the app. A driver that cannot start fails that check once; it does not retry the same startup failure for every test file. A cold Rust build can still take tens of minutes; hosted runners remove the local machine queue and maintenance rather than promising a fixed run time.

The groups exist because a runner has four processors, so the steps of `task ci` that build run one after another on it. Measured on 2026-11-09, after a dependency update that left the compiler cache without most of the workspace's crates, the one Windows job that ran every step took 46 min: `task gen` built for 6 min, the Rust tests for 16 min and Clippy for 8 min, and the tests then ran for 11 min. The same job took 18 min on Linux, and each end-to-end job 25 min, 5 min of them for the performance measurements. A dependency problem that cargo-deny found in the first minute showed when the job ended. A release ran all of this twice, once for the push to `main` and once for the tag.

Verified on 2026-11-10 against [GitHub's limits for Actions](https://docs.github.com/en/actions/reference/limits): GitHub Free runs 20 jobs of an account at a time. Further jobs wait: on 2026-11-09, when a release ran every check beside the `CI` run of the same push, some of its jobs started 100 s late. A full `CI` run has 23 jobs, 19 of which start at once and 4 after the plan of the programs' jobs, and Linux shares jobs between groups to stay near that limit. A release adds its build jobs beside them, and the jobs over the limit start as the first short ones end.

## 3. Cost and storage

Verified on 2026-10-21 against [GitHub's Actions billing documentation](https://docs.github.com/en/billing/concepts/product-billing/github-actions):

- Standard GitHub-hosted runner minutes are free in public repositories. Larger runners are charged even for public repositories; no workflow uses one.
- Keep the repository's cache limit at the included 10 GB. Old caches may be evicted, which makes a later build colder rather than preventing it from running.
- The pinned sccache compiler wrapper stores completed Rust compilations in GitHub's cache as they finish, including before a later test fails. Results are keyed by compiler inputs, so unchanged libraries can be reused across fresh runner machines. Clippy's metadata stays in its own build cache because sccache does not cache metadata-only compilation. The Cargo registry and pnpm package store are cached too.
- Store debug compiler results through sccache instead of keeping a complete build folder for every job. This leaves more room for reusable results within the cache limit.
- Release intermediates expire after one day. Published release files remain in GitHub Releases. Performance measurements go into the job log and summary, which do not consume artifact storage.
- Making the repository private again restores GitHub's private-repository allowances and billing. Reconsider that cost before changing visibility.

Verified on 2026-10-21 against the [sccache GitHub Actions integration](https://github.com/Mozilla-Actions/sccache-action) and [Rust caching limits](https://github.com/mozilla/sccache/blob/main/docs/Rust.md). Compiler caching speeds compilation; tests still execute. A cold compiler cache and linking still take time.

The caches stay useful only while they fit the limit and the runs that need them can read them. Verified on 2026-11-10 against [GitHub's dependency caching reference](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching): a run restores the caches of its own branch or tag and of the default branch, GitHub removes the caches that were read longest ago once the repository holds more than its limit, and a repository can add 200 cache entries a minute. On 2026-11-09 the caches that the run of the tag `v0.6.1` had saved, which only that tag can read, held 5.4 GB of the repository's 9.8 GB, and 3,200 of the 3,620 compiler results in the cache were of that day. The workflows therefore keep these rules:

- Only a run on `main` saves the caches of [rust-cache](https://github.com/Swatinem/rust-cache) (`save-if`, verified on 2026-11-10 for v2.9.2). Tags and pull requests restore `main`'s.
- Every job of a platform restores one cache of Cargo's registry, which the `static` job saves, and the job with Clippy restores the registry with Clippy's build folder and saves that. A cache of the registry for each job would hold the same crates many times.
- A release does not repeat the checks of its commit's push, so a tag's run stores only the results of its release builds, which a retry of the same tag reads.
- `ci.yml` and `release.yml` set the same variables whose names start with `CARGO_`, and the debug information of dev builds is set in `.cargo/config.toml`. sccache's key of a compilation holds every such variable (`src/compiler/rust.rs` of v0.18.0, verified on 2026-11-10), so a variable that one workflow sets alone keeps it from reading what the other stored.
- A job that builds what another job stores only reads the compiler cache (`SCCACHE_GHA_RW_MODE=READ_ONLY`, verified on 2026-11-10 in [sccache's notes on GitHub Actions](https://github.com/mozilla/sccache/blob/v0.18.0/docs/GHA.md)): the second end-to-end job of a platform, the performance measurements and two of the three credential jobs. Jobs that start together with an empty cache build the same crates at once, and sccache drops what GitHub's limit of 200 new entries a minute refuses.

## 4. Security and maintenance

- Every action is pinned by commit SHA, Rust by `rust-toolchain.toml`, and pnpm dependencies by the committed lock. Keep pins current in dedicated dependency changes.
- Jobs have read-only repository access. Only the job that signs and drafts a tagged release gets write access and the release signing key. Pull-request jobs never receive that key.
- Require approval for workflow runs from every outside contributor. Use `pull_request`, never `pull_request_target`, to test their code. The live providers and evaluations run only on their schedules or through an owner's manual run.
- No self-hosted runner serves this public repository. Stop the old Hyper-V controller and remove its runner registrations when switching to hosted builds. Existing VM images can remain powered off.
- Before publication, scan all Git history for credentials and inspect matches. Do not publish a real credential, an owner's runtime database, a private key or a local environment file.
