# M20 validation results

|              |                                                                                                                                                                                                                                               |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                                                                                                              |
| Last updated | 2026-11-10                                                                                                                                                                                                                                    |
| Related      | [Milestones](milestones.md#m20-desktop-updates), [ADR-0063](../adr/0063-update-the-desktop-app-from-signed-releases.md), [Updates](../features/updates.md), [Releasing](../engineering/releasing.md), [Security](../architecture/security.md) |

This document records what the checks of [M20](milestones.md#m20-desktop-updates) showed, so that the owner can review desktop updates and ADR-0063, and see what remains before the release. The owner accepted ADR-0063 with the plan (D55). Each section names the checks, where they run, the result, and what is open. V12 needs two published releases, the first with the updater and a later one, so it stays pending until both exist.

## 1. Summary

| Check                                             | Result                                           | Open                                                                                                    |
| ------------------------------------------------- | ------------------------------------------------ | ------------------------------------------------------------------------------------------------------- |
| `task ci`                                         | 15 or 16 of 17 steps passed in each of four runs | No run passed all steps: four tests of other features fail in some runs on this workstation (section 2) |
| The end-to-end test on the validation build       | Passed                                           | None on WebView2; WebKitGTK is open (section 3)                                                         |
| The whole end-to-end run on the same build        | 34 of 35 spec files passed                       | One test of the shell's spec fails on this workstation, with and without this work (section 3)          |
| The new steps of the release workflow             | Pending                                          | They run for the first time in the release that carries M20 (section 4)                                 |
| An installed build on Windows before the release  | Started                                          | The checks of section 5 that are pending                                                                |
| V12: a published release updates an installed one | Pending                                          | Every check of section 6                                                                                |

## 2. The automated checks

The [acceptance table](../features/updates.md#acceptance) of the feature names each check and its test. `task ci` runs them, except the end-to-end test:

- the tests of `scripts/release.mjs`, which sign and verify with a fixed key, write and verify a manifest, and keep the app's public key and the limit of an installer's size equal to the release's;
- three tests of `mannie-desktop-core`: one verifies a signature of the script with `minisign-verify`, the crate that the plugin verifies with, one runs the plugin's own check, download and verification against a local server, and one finds no permission of the plugin in the capability files;
- the tests of the journal in `crates/mannie-store`;
- the unit tests of the module `updates` with a fake source, and the tests of the UI's state and messages.

On Windows, on 2026-11-10, `task ci` ran four times on this work's final code, and no run passed all 17 steps: 16, 16, 15 and 16 steps passed. Two runs before them found a Clippy finding in this work's tests and a difference of formatting, and both were fixed. In the four runs, the 15 steps other than `test:rust:plugins` and `test:rust:core` passed every time. `test:rust:plugins` failed in three of the runs and `test:rust:core` in two, and the only tests that failed were three tests of other features. No other test failed in any of the four runs, the tests of this work passed in each, and each of the 17 steps passed in at least one run. A fourth test of another feature failed once in a further run of the plugins' tests:

| Test                                                                             | Failed in                                            | What failed                                                                                                                                                                |
| -------------------------------------------------------------------------------- | ---------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `a_delegated_task_uses_every_tool_of_the_worker_agent` of the plugin Workers     | Three runs of `task ci`                              | The call of `edit_file` ended with `permission_denied` on a file that the same task had written two calls before                                                           |
| `stop_ends_a_running_command_and_its_reply_within_a_second` of `mannie-runtime`  | Two runs of `task ci`                                | The stop took 1.35 and 1.47 seconds, and the test allows one second                                                                                                        |
| `a_planted_secret_reaches_no_log_database_or_crash_file` of `mannie-secrets`     | One run of `task ci`                                 | The failure printed no message. In a later run that prints the messages, the test saved a credential in Windows Credential Manager, and its read a moment later found none |
| `workers_reports_its_running_tasks_as_work_until_they_end` of the plugin Workers | One run of the plugins' tests in a copy of this work | After the stop of the reply, the plugin still reported one running task                                                                                                    |

This work changes none of these tests. It changes no file of `mannie-tools`, of `mannie-secrets` or of the plugin Workers. In `mannie-runtime` it only adds the identifier of an install, and in `mannie-store` the journal with one migration. Further runs on the same day show how far each failure depends on this work:

- The first test failed in 5 of 18 runs of the plugins' tests in this checkout, and in 1 of 18 runs on the files of `d9c2313`, the commit that this work starts from, in the same call with the same error. So it fails without this work. The runs show neither the cause of the error nor whether this work changes how often the test fails.
- The second test failed in 2 of 11 runs of the root workspace's tests in this checkout, both inside `task ci`. Run 50 times at below-normal priority, as in `task ci`, with the other tests of its file and the tests of `mannie-store`, it failed 8 times in this checkout and 3 times on the files of `d9c2313`. So it fails without this work, and the counts do not show whether this work changes how often. Each of these 11 failures fell in a run in which at least 13 other tests took a second or more longer than their median over the 50 runs. At least ten other tests were delayed like this in 36 of the 50 runs in this checkout and in 46 of the 50 on the files of `d9c2313`, so the delays occur without this work. With no check running, the workstation had 3.5 GB of its 31 GB of memory available, 22 days after its last restart. This may explain the delays, and no run shows that it does.
- The third test failed in 10 of 1000 runs of the crate's tests in this checkout, and in 2 of 1000 runs on the files of `d9c2313`. So it fails without this work, and the counts do not show whether this work changes how often. In the run that prints the messages, a second test of the crate once read the value from before its latest save. Both tests use Windows Credential Manager, and cargo-nextest runs them at the same time, each in a process of its own.
- The fourth test failed once, in a run with this work. It passed in the 60 other runs of the plugin's tests whose output was kept, 18 of them on the files of `d9c2313`, and in 60 further runs of the test alone. No run shows whether it fails without this work.

## 3. The end-to-end test

`tests/e2e/specs/updates.test.ts` drives the validation build, whose fake source the test sets. The validation build sends no request, makes no automatic check, and records an install in place of quitting. On Windows with WebView2 157.0.4341.42, on 2026-11-10, its eight tests passed. The steps:

- the settings section shows the running version and that no check ran yet, and the switch for automatic checks, turned off from the keyboard, keeps its position across a restart of the app;
- a version that is ready shows in the section and in a notice, in both themes;
- "Restart and update" from the command palette, while a reply is being written, asks first with the number of replies, and the confirmation from the keyboard records one install of that version;
- the notice of an install's result shows after a start, for an install that worked and for one that failed, in both themes, and closes with a click and from the keyboard;
- in the pseudo-locale, every text of the section, of the notices and of the question comes from the catalog, and Escape closes the question without an install;
- axe finds no WCAG 2.1 AA violation in any of these states, which show the section, both notices and the question in each theme.

`task e2e:run` on the same build ran 35 spec files, one in the startup configuration and 34 in the main one, and 34 of them passed. `tests/e2e/specs/shell.test.ts` failed in one test, "reaches the skip link first, with a visible focus indicator". The test presses Tab, after the test before it opened and closed the command palette, and expects the focus on the skip link. The focus stayed on the page's `body`. This work does not cause the failure:

- a build without the overlay of updates failed in the same test, in the same way;
- on a page where the palette never opened, the first Tab reaches the skip link, and after the palette opened and closed, the second Tab reaches it;
- during a run of the spec, the app's window never was the foreground window, because an agent's session started the run while the owner used another program;
- in CI, the test passed on `d9c2313`, the commit that this work starts from, with WebView2 156.0.4314.58 ([run 42717503900](https://github.com/limbon2/mannie/actions/runs/42717503900)).

Two things differ from CI, the window that is not in the foreground and the newer WebView2, and these runs do not show which of them decides. A run that the owner starts from a terminal, where the app's window takes the foreground, shows it: `task e2e` with `MANNIE_E2E_SPEC=tests/e2e/specs/shell.test.ts` in the environment.

## 4. The release workflow

The workflow's new steps cannot run before a release, and a failure in them keeps the draft from being made:

| Step                                                                             | What it does                                                                        | Checked before the release                                                             |
| -------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| "Sign the desktop installers and write the update manifest" in the job `publish` | Runs `manifest` and `verify-manifest` of `scripts/release.mjs` with the release key | The script's tests run both commands with a test key                                   |
| The install check on Windows                                                     | Reads `nsis` in the start line of the installed app's log                           | The lines read the kind from a sample log on the workstation, and fail on another kind |
| The install checks on Ubuntu and Fedora, in `scripts/ci/check-linux-install.sh`  | Read `deb` and `rpm` in the same line                                               | The lines read the kind from a sample log with `sh`, and the script parses             |

## 5. An installed build on Windows before the release

A local build of the NSIS installer, installed on the owner's workstation, checks the parts that need a real installation and the real address, while no release has a manifest yet:

| Check                                                                                                                                                 | Result                                                            |
| ----------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| `task build` needs no signing key                                                                                                                     | Passed on 2026-11-10, with no signing variable in the environment |
| The start line of the log names the installation `nsis`                                                                                               | Pending                                                           |
| "Check for updates" asks GitHub and says that Mannie cannot install the latest release from here, because the latest release has no `latest.json` yet | Pending                                                           |
| A development build shows updates as off and sends no request                                                                                         | Pending                                                           |

The three checks that are pending wait for the owner, because the installer replaces the installed Mannie and a development build opens the owner's own data.

## 6. V12: a published release updates an installed one

The owner installs the first release with the updater by hand on Windows 11, Ubuntu and Fedora, and publishes a later release. Each check then runs in the installed app:

| Check                                                                                                       | Windows 11     | Ubuntu (deb)   | Fedora (rpm)   |
| ----------------------------------------------------------------------------------------------------------- | -------------- | -------------- | -------------- |
| The check after the start finds the release, downloads it, and the notice shows the version                 | Pending        | Pending        | Pending        |
| "What's new" opens the release's page in the browser                                                        | Pending        | Pending        | Pending        |
| "Restart and update" installs the version, and the new version starts as the one running instance           | Pending        | Pending        | Pending        |
| The notice says that Mannie was updated, and the journal holds a completed record                           | Pending        | Pending        | Pending        |
| With a reply being written, the question shows the number, and "Cancel" keeps Mannie running                | Pending        | Pending        | Pending        |
| With the switch off, Mannie sends no request after the start, and "Check for updates" still checks          | Pending        | Pending        | Pending        |
| The installer asks nothing, and neither User Account Control nor SmartScreen shows a prompt                 | Pending        | Not applicable | Not applicable |
| The system's prompt asks for an administrator's password                                                    | Not applicable | Pending        | Pending        |
| After a cancelled password prompt, the old version starts again and says that the version was not installed | Not applicable | Pending        | Pending        |
| The log of the old version ends with the install's lines, and holds no secret and no content of the owner   | Pending        | Pending        | Pending        |

## 7. Open

- Every check above that is pending.
- The end-to-end test on WebKitGTK.
- What makes the test of the shell's spec fail on this workstation (section 3).
- A run of `task ci` in which all 17 steps pass. Four tests of other features fail in some runs on this workstation, and what makes them fail is open (section 2).
- A draft is never the latest release, so an installed app finds a release only after the owner publishes it. V12 therefore runs on published releases, and a release whose updater is broken needs one install by hand (R53).
