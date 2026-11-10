# ADR-0063: Update the desktop app from signed releases

|          |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D55)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Date     | 2026-11-10                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Related  | [ADR-0003](0003-build-the-desktop-app-on-tauri-2.md), [ADR-0004](0004-keep-system-access-in-the-rust-core.md), [ADR-0013](0013-journal-every-side-effect.md), [ADR-0017](0017-license-and-dependency-policy.md), [ADR-0018](0018-release-from-tagged-builds.md), [ADR-0019](0019-keep-visible-text-in-message-catalogs.md), [Updates](../features/updates.md), [Releasing](../engineering/releasing.md), [Security](../architecture/security.md), [M20 validation results](../roadmap/m20-validation.md) |

## Context

- The owner wants a new release to reach the desktop app without a download and an install by hand: "I want to have updates that won't require downloading and reinstalling the app every new version" ([decision D55](../roadmap/milestones.md#5-decisions)).
- Today the owner updates the desktop app by downloading the installer of the new release and running it ([releasing](../engineering/releasing.md#7-updates)). [ADR-0018](0018-release-from-tagged-builds.md), decision 7, left this to "a later feature with its own ADR", and desktop auto-update is part of the gate for 1.0 ([later](../roadmap/milestones.md#later)).
- Earlier decisions that an updater touches:
  - "The app uses two plugins, single-instance and opener. Any other plugin needs an ADR" ([ADR-0003](0003-build-the-desktop-app-on-tauri-2.md), decision 3).
  - "Mannie never phones home" ([vision](../product/vision.md)), and NFR-OPS-04 names every host that Mannie reaches ([goals and scope](../product/goals-and-scope.md#44-operability)). A check for updates is a request that the owner did not configure.
  - The webview is untrusted, so script in it must not be able to choose what Mannie downloads or installs ([ADR-0004](0004-keep-system-access-in-the-rust-core.md)).
  - Replacing the installed program is a side effect, and every side effect is a journaled operation ([ADR-0013](0013-journal-every-side-effect.md)).
- What a release is today:
  - CI builds an NSIS installer for Windows, which installs for the current user without administrator rights, and deb and rpm packages for Linux, all for x86_64. They are files of a draft GitHub release, which the owner publishes after the smoke installs pass (ADR-0018, decisions 3 and 5; D17). The repository is public (D39).
  - The release key is an Ed25519 key that only the `publish` job of the release workflow holds, as a secret of GitHub Actions. It signs the archives of the worker, and nothing signs the desktop installers (ADR-0018, decision 4; D16).
  - Workers update by hand, with `mannie-worker update` on the host (D20).
- Facts of Tauri's updater plugin, verified on 2026-11-10 in its source and its documentation ([crate](https://crates.io/crates/tauri-plugin-updater/2.14.1), [documentation](https://v2.tauri.app/plugin/updater/)):
  - `tauri-plugin-updater` 2.14.1 is Tauri's own updater for Tauri 2, under `Apache-2.0 OR MIT`. It updates installations of NSIS installers and of deb and rpm packages, among others.
  - A check is one GET of a configured address. The answer is a manifest: a version, notes, a date, and for each platform a download address and a signature. The plugin offers the version when it is newer than the running one by SemVer.
  - The signature is a minisign signature, Ed25519 over the BLAKE2b-512 hash of the file. The plugin checks it with the public key of the app's configuration after the download and again before it installs, and the check cannot be turned off.
  - The manifest is not signed. With `requireSignedVersion`, the plugin also requires that the signature's trusted comment, which the key signs too, names the version that the manifest announced.
  - On Windows the plugin runs the downloaded NSIS installer in passive mode with `/P /UPDATE /R` and ends the process, and the installer starts the new version. On Linux it runs `dpkg -i` or `rpm -U` through `pkexec`. When `pkexec` fails, it asks for the password with zenity or kdialog and gives it to `sudo -S` on standard input. The app starts itself again.
  - The plugin reads the kind of an installation from a mark that Tauri's bundler writes into the binary of each package. It treats a Linux binary without a mark as an AppImage, which it updates by replacing the running executable.
  - The plugin keeps a whole download in memory, with no limit of its size.
  - Its default features include `zip`, which unpacks installers that are wrapped in archives.
- Facts of GitHub, verified on 2026-11-10:
  - `https://github.com/limbon2/mannie/releases/latest/download/<file>` redirects to that file of the latest release ([linking to releases](https://docs.github.com/en/repositories/releasing-projects-on-github/linking-to-releases)). "Drafts and prereleases cannot be set as latest" ([releases API](https://docs.github.com/en/rest/releases/releases#create-a-release)).
  - A release file redirects on to the host `release-assets.githubusercontent.com`.
  - GitHub's REST API serves 60 requests an hour to an IP address without a token ([rate limits](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api)).

## Decision

1. The desktop app updates itself with Tauri's updater plugin, `tauri-plugin-updater` 2.14.1. It is the app's third Tauri plugin, beside single-instance and opener.
   - It is built without its default features, with `rustls-tls` and `system-proxy`: TLS comes from rustls with ring, as in the rest of Mannie, and a check works behind the system's proxy.
   - `zip` stays off, since Mannie's updates are the installers themselves.
2. The plugin is registered for its configuration and its state. No capability grants an `updater:` permission, so the webview cannot call the plugin, and the UI reaches updates only through commands of the core, which validate their input (ADR-0004).
3. Mannie reads updates from one address, `https://github.com/limbon2/mannie/releases/latest/download/latest.json`. The address is a constant of the core, and no setting, file or message changes it.
   - `latest.json` is the plugin's static manifest, with the platforms `windows-x86_64-nsis`, `linux-x86_64-deb` and `linux-x86_64-rpm`.
   - The core accepts a download address only when it starts with `https://github.com/limbon2/mannie/releases/download/`, the release files of Mannie's repository. Any other address fails the check.
   - Publishing the draft release (ADR-0018, decision 5) is what offers an update. Drafts and prereleases are never offered.
4. The release key also signs the desktop installers, which refines ADR-0018, decision 4.
   - `scripts/release.mjs` writes each signature in minisign's prehashed format, with a trusted comment that names the file and the version, and writes the manifest. It runs in the `publish` job, the only job that holds the key, and then checks the manifest and every signature again with the public key alone.
   - The app's configuration holds the public key in minisign's form, and a test keeps it equal to `release/mannie-release.pub`.
   - `requireSignedVersion` is on, and downgrades stay off. So Mannie installs only a version that is newer than the running one and that the key signed under that version.
   - Builds need no key: Tauri's bundler signs nothing, and `createUpdaterArtifacts` stays unset.
5. Only an installed release updates itself: a release build whose binary Tauri's bundler marked as an NSIS, deb or rpm package.
   - No other build contacts GitHub. A development build and a binary that no package installed show updates as off with the reason, and a validation build reads a fake source for the tests.
   - This also keeps the plugin away from its AppImage path, which would replace an unmarked Linux binary.
   - The install checks of the release workflow read the kind of the installation in the installed app's log, since a bundler that fails to mark a binary only warns.
6. Mannie checks one minute after it starts and every 24 hours after that, while the setting `updates.check_automatically` is on, which is the default. The owner can also check at any time.
   - An automatic check needs the setting to have been read. When the database did not open, or the settings cannot be read, Mannie makes no automatic check.
   - A newer version downloads at once, in the background. The plugin verifies it, and the verified installer stays in memory until Mannie installs it or exits.
   - Mannie refuses a download of more than 256 MiB, and the release script refuses to publish a larger installer.
   - One check or download runs at a time.
7. A check sends nothing about the owner. It is a GET of the fixed address with the plugin's user agent, and it carries no identifier, no settings and no version. GitHub and its file host learn the IP address and the time of the request.
   - NFR-OPS-04 adds `github.com` and `release-assets.githubusercontent.com`, and the vision names the check as the one request that the owner did not configure.
   - The switch in the settings turns the automatic checks off. Then Mannie contacts GitHub only when the owner checks.
8. Mannie installs an update only when the owner chooses "Restart and update". It never installs by itself, and never at quit.
   - The command names the version that the owner saw, and the core refuses a version that is not the one that is ready.
   - When replies, background commands or terminals are running, the core first answers with their numbers, and installs only after the owner confirms.
9. Every install is a journaled operation (ADR-0013). The core commits a record with both versions to the store's table `updates_operations` before it quits, and installs nothing when it cannot write the record.
   - The next start settles the record by the version that is running: completed when it is the record's target, failed otherwise.
   - A notice tells the owner the result once.
10. The install runs after Mannie quit as it always does: the runs are interrupted and recorded, the store is flushed and the plugins stop.
    - On Windows the installer runs in passive mode for the current user, asks for no administrator rights and starts the new version.
    - On Linux the system asks for the password of an administrator, `dpkg -i` or `rpm -U` installs the package, and Mannie starts again with the arguments that it was started with.
    - When the install cannot start, or fails on Linux, the old version starts again. The failure goes into the log, and into the journal's record as a note for the next start's log.
    - When the Windows installer fails after Mannie ended, the old version stays installed. The next start of it tells the owner that the version was not installed.
11. The log closes last. It stays open through the shutdown and the install, on Windows until right before the installer starts, so the log shows what the updater did.
12. The core reports updates as states and codes, and the texts come from the UI's catalog (ADR-0019). The states are off with a reason, idle, checking, downloading, ready and failed. A failure is one of `unreachable`, `no_update_information`, `no_package_for_this_installation`, `signature_invalid` and `download_failed`.
13. The threats of [security](../architecture/security.md) gain T-100 to T-105: a manifest or a download that the release key did not sign, release files that hide a newer release, a stolen release key, script in the webview that starts a check or an install, the password prompts of the Linux install, and what GitHub learns from a check.
14. Outside this decision:
    - updates of workers, which stay manual (D20);
    - code signing (D16);
    - delta updates, so each update downloads a whole installer;
    - channels and prereleases;
    - macOS;
    - release notes inside the app, since "What's new" opens the release's page in the browser;
    - a return to an earlier version;
    - updates while Mannie shows a startup problem, when the owner installs a release by hand.

The first release with the updater is installed by hand once. Every later release installs from inside the app.

Earlier decisions that this ADR changes:

| Earlier decision                                                 | Change                                                                                                                     |
| ---------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| [ADR-0003](0003-build-the-desktop-app-on-tauri-2.md), decision 3 | Superseded in part: the app uses a third plugin, the updater (decision 1)                                                  |
| [ADR-0018](0018-release-from-tagged-builds.md), decision 4       | Refined: the release key also signs the desktop installers, in minisign's format (decision 4)                              |
| [ADR-0018](0018-release-from-tagged-builds.md), decision 7       | Superseded: the desktop app updates itself from signed releases                                                            |
| [NFR-OPS-04](../product/goals-and-scope.md#44-operability)       | Extended: Mannie also reaches `github.com` and `release-assets.githubusercontent.com` (decision 7)                         |
| The [vision](../product/vision.md)'s "never phones home"         | Changed: the check for updates is the one request that the owner did not configure, and a switch turns it off (decision 7) |

## Consequences

- Positive:
  - A published release reaches the owner's desktop with one click, without a browser, a download or an installer's pages.
  - Only code that the release key signed under its version installs, whatever the network, GitHub's hosts or the release files say.
  - Releases need no new secret and no new step: the `publish` job signs the installers with the key that it already holds.
  - The journal and the log show each install and its result.
- Negative:
  - Mannie makes a request that the owner did not configure, once a day by default, until the owner turns the switch off.
  - The release key now decides what installs on the owner's desktop, where it only decided what a worker's `update` accepts (R54).
  - Each update downloads a whole installer, 16 to 28 MiB today, and Mannie keeps it in memory until it is installed.
  - On Linux each install asks for an administrator's password, and the plugin's second prompt takes the password through Mannie's memory (T-104).
  - A new dependency sits where trust is decided: the plugin, and the HTTP client and the verification code that it uses.
  - A release whose updater is broken cannot repair itself: its installations need one install by hand (R53).
  - Mannie offers no update while it shows a startup problem.
- Risks:
  - Whoever controls the release files or the network path hides a newer release, so Mannie stays on a version with a known fault. They cannot install an older or an unsigned build (T-101).
  - Tauri's bundler fails to mark a binary, so an installed release shows updates as off. The install checks of the release workflow fail on it before the release.
  - The release key is stolen. Together with changed release files, it installs code on the owner's desktop (R54, T-102).
  - A later version of the plugin changes how it verifies or installs. The version is pinned by the lock, and the tests verify a signature of the release script with the plugin's own code.

## Alternatives considered

| Option                                                     | Why not chosen                                                                                                                                                                                                                                                   |
| ---------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Keep updates manual                                        | The owner asked to stop downloading and installing each version by hand (D55)                                                                                                                                                                                    |
| An updater of Mannie's own                                 | It would repeat the plugin's work for each platform: running NSIS in update mode, raising privileges on Linux and telling how the app was installed. The plugin is Tauri's own and passes the license policy ([ADR-0017](0017-license-and-dependency-policy.md)) |
| A separate signing key for desktop updates                 | A second secret to create, store and back up, for the same job, the same workflow and the same owner. A thief of one key of the `publish` job would hold the other too                                                                                           |
| Letting Tauri's bundler sign (`createUpdaterArtifacts`)    | Every build job would need the private key, where only the `publish` job has it today, and local builds would need a key or a switch                                                                                                                             |
| Checks only when the owner asks                            | The owner chose automatic checks with a switch (D55). Without them, an update again waits until the owner thinks of it                                                                                                                                           |
| Installing without a click, or at quit                     | An install interrupts replies, background commands and terminals, and on Linux it asks for a password. The owner chooses the moment                                                                                                                              |
| Installing while the app runs, then restarting             | The programs that Mannie starts could come from a package that is half replaced, and the shutdown that records the interrupted runs would run in a replaced program                                                                                              |
| Distribution repositories, or winget                       | A repository to host and sign for each distribution, and other tools for each platform, where one mechanism serves the three installers that the release already has                                                                                             |
| A manifest server of Mannie's own                          | A service to run and secure, for a file that GitHub's releases already serve from a fixed address                                                                                                                                                                |
| GitHub's API `releases/latest` in place of a manifest file | 60 requests an hour for each IP address without a token, and the answer does not carry the signatures                                                                                                                                                            |
| Keeping the download in a file until the install           | A file that waits on disk for hours or days, where another program of the same user can replace it, and that Mannie must clear up. The plugin hands over the verified bytes and writes the installer to disk only when the install starts                        |

## Validation

M20: the tests of the release script, which sign and verify with a fixed key; a test of the core that verifies a signature of the script with `minisign-verify`, the crate that the plugin verifies with, and runs the plugin's own check, download and verification against a local server; the store's tests of the journal; the tests of the updates module with a fake source; and the end-to-end test on the validation build. Then V12: a published release updates an installed one through the app on Windows 11, Ubuntu and Fedora, recorded in the [M20 validation results](../roadmap/m20-validation.md).
