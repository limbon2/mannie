# Updates

|              |                                                                                                                                                                                                                                                                                                                         |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                |
| Milestone    | M20                                                                                                                                                                                                                                                                                                                     |
| Last updated | 2026-11-10                                                                                                                                                                                                                                                                                                              |
| Related      | [ADR-0063](../adr/0063-update-the-desktop-app-from-signed-releases.md), [ADR-0013](../adr/0013-journal-every-side-effect.md), [ADR-0018](../adr/0018-release-from-tagged-builds.md), [Releasing](../engineering/releasing.md#7-updates), [Security](../architecture/security.md), [Window and tray](window-and-tray.md) |

## Intent

The owner wants a new release to reach the desktop app without a download and an install by hand: "I want to have updates that won't require downloading and reinstalling the app every new version". Mannie looks for a newer release by itself, downloads it in the background and keeps it only when the release key signed it. The owner chooses when it installs, because an install restarts Mannie.

## Behavior

### Where updates show

- The settings have a section "Updates". It shows the version that is running, what Mannie knows about a newer one, the button "Check for updates" and the switch "Check for updates automatically".
- When a version is ready to install, a notice appears in the lower corner of the window, with "Restart and update" and "What's new". The notice stays while Mannie checks again. The owner can close it, and it stays closed until Mannie starts again or another version is ready. The settings section keeps showing the version.
- The command palette has "Check for updates", which opens the settings section and checks, and "Restart and update" while a version is ready.

### Checks and downloads

- Mannie checks one minute after it starts and every 24 hours after that, while the switch is on. The switch is on by default, and turning it on checks at once.
- "Check for updates" checks at any time, also while the switch is off.
- A check asks GitHub for the latest release of Mannie. The request says nothing about the owner or this installation, and it does not name the version that is running. GitHub learns the IP address and the time of the request. With the switch off, Mannie contacts GitHub only when the owner checks.
- When the latest release is newer than the running version, Mannie downloads its installer at once, in the background. The section shows how much arrived, as a percent when GitHub names the size of the installer and in megabytes when it does not. Mannie keeps the download only when the release key signed it under that version.
- The download waits in memory. When Mannie quits without installing it, a later check downloads it again.
- When no newer release exists, the section says that Mannie is up to date, with the time of the check.

### Installing

- "Restart and update" installs the version that is ready. Nothing else does: Mannie never installs by itself, and never when it quits.
- When replies are being written, background commands run or terminals run commands, Mannie first asks, with their numbers, because the restart interrupts them as quitting does ([window and tray](window-and-tray.md)).
- Mannie then quits as it always does, installs the version and starts again.
  - On Windows a small window of the installer shows the progress and asks nothing.
  - On Linux the system asks for an administrator's password, because the package manager installs the package. The section and the question say so before the owner chooses.
- After the start, a notice says that Mannie was updated to the version. Its "What's new" opens the release's page in the browser.
- When the restart does not begin, for example because Mannie could not record the install, the notice of the ready version shows again and says what failed.

### Failures

Each failure shows in the settings section with what the owner can do, and "Check for updates" tries again.

| What happened                                                          | What the section shows                                                                                        |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| GitHub could not be reached                                            | That Mannie could not reach GitHub, and that the owner should check the network connection                    |
| The latest release has no update information, or Mannie cannot read it | That Mannie cannot install the latest release from here, with "Open the releases page" for an install by hand |
| The latest release has no package for this kind of installation        | The same                                                                                                      |
| The download failed, or the installer is larger than 256 MiB           | That the download failed                                                                                      |
| The release key did not sign the download under the announced version  | That Mannie discarded the download because its signature is wrong                                             |

- A failed check or download leaves a version that is ready as it is.
- A failed automatic check is not repeated before the check of the next day.
- When an install did not happen, the version that ran before starts again where it can, and a notice says that the version was not installed and which version is running. Its "Check for updates" opens the settings section and checks, and the check downloads the version again.
- When a check, a restart or the page of a release does not start, the settings section says why, and so does the notice that offered it.

### Where Mannie does not update itself

- A development build shows updates as off.
- A copy of Mannie that no installer and no package installed, such as a binary that was built from the source, shows updates as off, with "Open the releases page".
- While updates are off, the section shows the running version and the reason, without the switch and without "Check for updates".
- While Mannie shows a startup problem, such as a database that did not open, it offers no update. The owner installs a release by hand.
- When Mannie cannot read its settings, it makes no automatic check, since it cannot tell whether the switch is off. The owner can still check.

### What this version does not do

- It installs no older version, no prerelease and no draft.
- It shows no release notes inside the app.
- It does not update workers. `mannie-worker update` stays as it is ([D20](../roadmap/milestones.md#5-decisions)).
- It downloads a whole installer for each update.
- The installers stay without the code signing of Windows ([D16](../roadmap/milestones.md#5-decisions)).

## Fit

- Follows [ADR-0063](../adr/0063-update-the-desktop-app-from-signed-releases.md), which decides the updater plugin, the address, the signatures, the journal and the install after the quit.
- [UI shell slots](../architecture/extension-points.md#37-ui-shell-slots): a settings section, an overlay for the notice and the question, and two actions of the command palette.
- [IPC commands and channels](../architecture/extension-points.md#36-ipc-commands-and-channels): the commands below and the channel of the status.
- [Settings sections](../architecture/extension-points.md#38-settings-sections): the section `updates`.
- [Store tables and migrations](../architecture/extension-points.md#39-store-tables-and-migrations): the module `updates` with the table `updates_operations`.
- [Message catalogs](../architecture/extension-points.md#311-message-catalogs): the prefix `updates_`.
- It needs no new extension point.
- Must not change:
  - Quitting. The restart quits through the path of "Quit", so active runs are interrupted and recorded and background commands end as they do today ([window and tray](window-and-tray.md)).
  - Releases for an install by hand. The installers, their names and `SHA256SUMS` stay, and a release gains the file `latest.json` ([releasing](../engineering/releasing.md#4-artifacts)).
  - Workers, to which the master pushes no builds (D20).

## Open questions

None.

## Decisions

- 2026-11-10: a failed automatic check waits for the check of the next day. Mannie does not try again sooner by itself, so a host that is offline makes one request a day.
- 2026-11-10: the downloaded installer stays in memory and is not kept across starts, so no file waits on disk where another program could replace it.
- 2026-11-10: Mannie installs nothing without the journal's record of the install. When the record cannot be written, the version stays ready and the command fails.
- 2026-11-10: closing the notice of a ready version hides it until the next start or the next version. The settings section and the command palette keep offering the install.
- 2026-11-10: the failures that Mannie cannot repair offer "Open the releases page", so the owner can always install a release by hand.
- 2026-11-10: the notice of a ready version stays while Mannie checks again, so it does not leave and return with each check.
- 2026-11-10: "Check for updates" of the command palette and of the notice about a failed install opens the settings section before it checks, so the owner sees what the check finds.
- 2026-11-10: a restart that does not begin shows the notice of the ready version again, also after the owner closed it, because the notice is where the owner reads why.
- 2026-11-10: the section hides the switch and "Check for updates" while updates are off, since neither would do anything.
- 2026-11-10: the section shows the amount of a download as a percent when the server names the size, and in megabytes otherwise.

## Design

### Checks and downloads

The module `updates` of `mannie-desktop-core` holds the state of updates and reaches the updater plugin through a port, so its tests run against a fake source. The source of a build follows from how it was built and installed:

| Build                                                                              | Updates                                                                             |
| ---------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| A release build whose binary Tauri's bundler marked as an NSIS, deb or rpm package | On, from the releases of Mannie's repository                                        |
| A release build without the mark                                                   | Off, with the reason `not_installed_from_package`                                   |
| A development build                                                                | Off, with the reason `development_build`                                            |
| A validation build                                                                 | A fake source that the end-to-end tests set, with no request and no automatic check |

- The start line of the log names the mark in its field `installation`: the bundler's kind of package, such as `nsis`, or `unmarked`. The install checks of the release workflow read it there (ADR-0063, decision 5).
- The real source builds the plugin's updater with the address `https://github.com/limbon2/mannie/releases/latest/download/latest.json`, which is a constant of the core, and with the app's configuration: the public key, `requireSignedVersion` and the passive mode of the Windows installer. The configuration holds no `endpoints`, so the address has one place.
- One task checks at a time. It reads the manifest, and when the manifest offers a version that is not ready yet, it downloads the installer and the plugin verifies it. A request for a check while the task runs starts nothing.
- A check decides what is ready. The ready version offered again keeps its installer, another version replaces it, and a check that offers nothing drops it. A failed check or download changes nothing.
- The core accepts a download address only under `https://github.com/limbon2/mannie/releases/download/`, and ends a download that passes 256 MiB or that the server announces as larger. The release script refuses to write a manifest with an installer above the same size, and a test keeps the two limits equal, so a release offers nothing that the app would not download.
- A check waits 30 seconds for the manifest. A download ends after 30 minutes, or when no data arrived for 60 seconds.
- The schedule waits one minute, checks when the setting was read and is on, and repeats every 24 hours, by the system's monotonic clock ([platform differences](#platform-differences)). The time of the last check is kept in memory only.
- The status reaches the window through a channel, and the progress of a download changes it every 256 KiB.

### The install

1. `restart_to_update` takes the version that the owner saw. It refuses a version that is not the one that is ready.
2. Unless the owner confirmed, it counts the replies, the background commands and the terminals that run, and answers with the numbers when any is above zero.
3. It writes the journal's record with the running version and the ready one, with full durability, and then quits Mannie through the path of "Quit". When the record cannot be written, the installer stays ready and the command fails.
4. After the event loop has ended and the core has shut down, the app hands the installer to the plugin, which verifies it again and installs it. The log is still open (ADR-0063, decision 11).
5. On Windows the plugin starts the installer and ends the process, and the log closes right before. On Linux the plugin returns after the package manager did, Mannie starts its binary again with the arguments that it was started with, and the log closes after that.
6. When the install fails, Mannie logs the failure, notes its text in the journal's record and starts itself again in the same way. The text is only logged, never shown.
   - Mannie starts the binary itself, because Tauri's restart would end the process before the log is closed. A start that fails is logged.
7. The next start settles every record that is still `executing`: `completed` when its target is the running version, `failed` otherwise. The status carries the newest result until the owner closes its notice.

### Contracts

| Command                   | What it does                                                                                                                                                                                                                                                      |
| ------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `get_updates_settings`    | Reads the settings section `updates`                                                                                                                                                                                                                              |
| `update_updates_settings` | Changes `check_automatically`. Turning it on starts a check                                                                                                                                                                                                       |
| `subscribe_updates`       | Opens the channel of the status: the running version, the state, the result of the last install and whether an install asks for the system's password. The channel sends the status at once and after each change                                                 |
| `check_for_updates`       | Starts a check and answers at once. It starts nothing while a check or a download runs, and it fails while the database is not open                                                                                                                               |
| `restart_to_update`       | Takes the version and whether the owner confirmed. It answers `confirm_first` with the numbers of what runs, or `restarting`                                                                                                                                      |
| `dismiss_update_outcome`  | Drops the result of the last install from the status, when the owner closes its notice                                                                                                                                                                            |
| `open_release_page`       | Opens the page of the ready version's release, of the running version's or of the latest release in the browser. The core builds the address from the versions that it holds, and the page of the ready version fails with `version_not_ready` when none is ready |

The states of the status:

| State         | What it carries                                                            |
| ------------- | -------------------------------------------------------------------------- |
| `off`         | The reason, `development_build` or `not_installed_from_package`            |
| `idle`        | The time of the last check, or none before the first                       |
| `checking`    | Nothing                                                                    |
| `downloading` | The version, the bytes that arrived, and the size when the server names it |
| `ready`       | The version                                                                |
| `failed`      | The code of the failure and the time of the check                          |

The codes of a failed check:

| Code                               | When                                                                                                                                                |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `unreachable`                      | The request for the manifest got no answer                                                                                                          |
| `no_update_information`            | The address answered without a manifest that the plugin can read, or the manifest names a download outside the release files of Mannie's repository |
| `no_package_for_this_installation` | The manifest has no entry for this platform and kind of installation                                                                                |
| `download_failed`                  | The download ended with an error, or passed the limit of its size                                                                                   |
| `signature_invalid`                | The signature does not verify, or its trusted comment names another version or none                                                                 |

A command fails with a code of its own:

| Code                          | When                                                     |
| ----------------------------- | -------------------------------------------------------- |
| `version_not_ready`           | The named version is not the one that is ready           |
| `not_available_in_this_build` | This build does not update itself                        |
| `store_unavailable`           | The database is not open                                 |
| `storage_failed`              | The settings or the journal could not be read or written |
| `written_by_newer_version`    | A newer version of Mannie wrote the settings section     |
| `browser_unavailable`         | The system's browser did not open the page of a release  |

Validation builds add commands that set what the fake source offers and how it fails, set the result of a last install, and read the installs that were asked for. In a validation build an install is recorded and Mannie keeps running.

### Data

- The settings section `updates`, version 1, holds `check_automatically`, which is true by default.
- The table `updates_operations` is the journal of installs ([ADR-0013](../adr/0013-journal-every-side-effect.md)), made by migration 0001 of the store's module `updates`. A row holds an ID, `from_version`, `to_version`, the state (`executing`, `completed` or `failed`), `failure` with the noted text of a failed install, and the times of its creation and its last change.

### Messages

`updates_` prefix: the settings section, the states and the failures, the notices, the question before a restart and the actions of the command palette.

### Threat notes

The threats are T-100 to T-105 of [security](../architecture/security.md#5-threats-and-mitigations).

- The manifest and the download come from the network, and neither is trusted. Only an installer that the release key signed under a version that is newer than the running one installs (T-100). Whoever changes the release files can hide a newer release and nothing more (T-101), unless they also hold the release key (T-102).
- Script in the webview can start a check and can restart Mannie into the version that is ready. It can name no address, no file and no other version, and a check that runs keeps a second one from starting (T-103).
- On Linux the password prompts belong to the system and to the plugin. Mannie's own code never sees the password and logs nothing of the prompts (T-104).
- A check tells GitHub the IP address and the time, and the switch turns the automatic checks off (T-105).

### Platform differences

| Topic               | Windows                                                                                                                                                                         | Linux                                                                                            |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| What updates itself | An installation of the NSIS installer                                                                                                                                           | An installation of the deb or the rpm package                                                    |
| The schedule        | The 24 hours also pass while the computer sleeps                                                                                                                                | The 24 hours pass only while the computer is awake, so a computer that sleeps checks less often  |
| The install         | The installer runs in passive mode for the current user, with a window of its progress and no questions                                                                         | The system asks for an administrator's password, then `dpkg -i` or `rpm -U` installs the package |
| The start after it  | The installer starts the new version                                                                                                                                            | Mannie starts itself again                                                                       |
| A failed install    | When the installer cannot start, the old version starts again. When it fails later, the old version stays installed, and its next start says that the version was not installed | The old version starts again and says that the version was not installed                         |

## Acceptance

| Check                                                                                                                                                                                      | How it is tested                                                                                                                     |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------ |
| A signature of the release script verifies with the crate that the plugin verifies with, and a changed file, a changed trusted comment and the legacy format do not                        | The tests of `scripts/release.mjs`, and a test of `mannie-desktop-core` that reads the fixture which the script's test keeps current |
| The manifest names the platforms by the installers' names, every signature names the version, the script refuses an installer over the limit, and the app's public key is the release key  | The tests of `scripts/release.mjs`                                                                                                   |
| The plugin's own check, download and verification accept a release of the script and refuse a changed installer and another version                                                        | A test of `mannie-desktop-core` against a local server                                                                               |
| No capability grants the webview a permission of the plugin                                                                                                                                | A test of `mannie-desktop-core` that reads the capability files                                                                      |
| The journal: an install that reached its version, one that did not, a resolution that runs twice, and a noted failure                                                                      | The tests of `crates/mannie-store`                                                                                                   |
| Checks: nothing new, a new version, the same version again, another version, each failure, one check at a time, the limit of the size, and the schedule with the switch on, off and unread | The unit tests of the module `updates` with a fake source                                                                            |
| The restart: a version that is not ready, the question with the numbers, an install without its record, and the result after the next start                                                | The same                                                                                                                             |
| The section, the notice, the question and the notice of a result, with the keyboard, in both themes and in the pseudo-locale                                                               | The end-to-end spec `tests/e2e/specs/updates.test.ts` on the validation build                                                        |
| An installed release names its kind of installation in its log                                                                                                                             | The install checks of the release workflow                                                                                           |
| A published release updates an installed one on Windows 11, Ubuntu and Fedora (V12)                                                                                                        | By hand, recorded in the [M20 validation results](../roadmap/m20-validation.md)                                                      |
