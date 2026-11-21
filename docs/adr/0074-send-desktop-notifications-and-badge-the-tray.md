# ADR-0074: Send desktop notifications and badge the tray for conversation events

|          |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D69)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| Date     | 2026-11-21                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Related  | [ADR-0002](0002-grow-through-extension-points.md), [ADR-0003](0003-build-the-desktop-app-on-tauri-2.md), [ADR-0004](0004-keep-system-access-in-the-rust-core.md), [ADR-0009](0009-persist-state-in-sqlite.md), [ADR-0012](0012-run-agents-as-durable-actors.md), [ADR-0017](0017-license-and-dependency-policy.md), [ADR-0019](0019-keep-visible-text-in-message-catalogs.md), [ADR-0058](0058-let-agents-ask-the-owner-questions.md), [ADR-0063](0063-update-the-desktop-app-from-signed-releases.md), [Notifications](../features/notifications.md), [Window and tray](../features/window-and-tray.md), [Questions to the owner](../features/questions.md), [Security](../architecture/security.md) |

## Context

- The owner asked for desktop notifications when Mannie asks questions or finishes its work, and a badge on the tray icon when a job is done. Both [window and tray](../features/window-and-tray.md) and [questions to the owner](../features/questions.md) left notifications and tray badges to a feature of their own.
- On 2026-11-21 the owner chose the behavior of this feature:
  - desktop notifications fire for questions (`ask_owner`), tool approval requests, completed runs and failed runs;
  - notifications fire only when Mannie's main window is hidden in the tray or not focused;
  - the tray icon badge shows a dot when a run finished while the owner was not viewing that conversation, or while any conversation waits on a question or approval;
  - the taskbar or dock icon shows the numeric count (`1` to `99`, and `99+`) of distinct conversations that need attention (unread finished runs plus pending questions or approvals);
  - both badges clear only after the owner opens each unread finished conversation (which also carries an unread mark in the sidebar) and no question or approval remains pending, and each badge has its own toggle in Settings.
- Earlier decisions that this change touches:
  - [ADR-0003](0003-build-the-desktop-app-on-tauri-2.md), decision 3, limits Tauri plugins to `single-instance`, `opener` and (via [ADR-0063](0063-update-the-desktop-app-from-signed-releases.md)) `updater`, and requires an ADR for any additional Tauri plugin.
  - [ADR-0004](0004-keep-system-access-in-the-rust-core.md) treats the webview as untrusted: script in the webview must not be able to send arbitrary desktop notifications.
  - [ADR-0019](0019-keep-visible-text-in-message-catalogs.md) keeps all visible text in message catalogs, including text that the Rust core passes to operating system surfaces such as the tray menu and desktop notifications.
- Facts verified on 2026-11-21:
  - `tauri-plugin-notification` 2.8.1 ([crates.io](https://crates.io/crates/tauri-plugin-notification/2.8.1), `Apache-2.0 OR MIT`, Rust 1.90 or newer, built on `tauri` 2.12) sends native desktop notifications through `notify-rust` 4.19.0 (`MIT OR Apache-2.0`), which uses `tauri-winrt-notification` 0.8.1 (`MIT OR Apache-2.0`, depending on `windows` 0.62 and `windows-version` 0.1, both already in `Cargo.lock`) on Windows and `zbus` on Linux over `org.freedesktop.Notifications`.
  - `tauri-plugin-notification` 2.8.1 exposes `Notification::on_action` in Rust on Windows and Linux so the core handles clicks while the app runs.
  - On Windows, WinRT toast notifications require a registered App User Model ID on a Start Menu shortcut, which Tauri's NSIS installer writes for `io.github.limbon2.mannie`. Uninstalled development binaries lack that shortcut, so `tauri-plugin-notification` omits Mannie's identifier and `notify-rust` falls back to PowerShell's App User Model ID.
  - Neither Windows nor Linux AppIndicator places a separate icon next to a system tray icon. Tauri's `TrayIcon::set_icon` swaps the tray icon image in memory from RGBA pixels.
  - Tauri 2.12 exposes `WebviewWindow::set_overlay_icon` on Windows (calling `ITaskbarList3::SetOverlayIcon` with an in-memory RGBA icon) and `WebviewWindow::set_badge_count` on Linux (sending the launcher count over Unity LauncherEntry when supported by the desktop) with no additional dependency.

## Decision

1. Use `tauri-plugin-notification` 2.8.1 as the desktop app's fourth Tauri plugin, beside `single-instance`, `opener` and `updater`. This supersedes [ADR-0003](0003-build-the-desktop-app-on-tauri-2.md), decision 3, in part. Do not add `@tauri-apps/plugin-notification` to the frontend package.
2. Register `tauri-plugin-notification` in `mannie-desktop-core` without granting any `notification:*` capability permission to the webview ([ADR-0004](0004-keep-system-access-in-the-rust-core.md)). The webview cannot post operating system notifications directly.
3. Expose an owner-conversation signal stream (`subscribe_signals`) from `mannie-runtime`'s event hub. The hub emits signals only for owner conversations, never for task conversations, when a run completes, fails, enters `awaiting_answer` or `awaiting_approval`, starts, or when a conversation is deleted.
   - Child agents and worker tasks do not trigger separate notifications; their relayed tool approvals already move the parent conversation into `awaiting_approval`.
   - Stopped runs and interrupted runs never send notifications and never mark a conversation unread.
4. Send a desktop notification from the Rust core only when Mannie's main window is hidden in the tray or unfocused, and the owner has the feature and that event category enabled in the `notifications` settings section (all enabled by default: master switch, questions, tool approvals, completed runs, failed runs, taskbar count badge and tray icon badge).
   - Deduplicate waiting notifications by `part_id`, so a child approval that temporarily displays over a pending question does not trigger a second notification for the same question when the approval finishes.
5. Keep all visible notification and tray-attention text in the message catalog ([ADR-0019](0019-keep-visible-text-in-message-catalogs.md)).
   - On startup and after every reload, the window sends the localized notification titles and tray attention tooltip through `set_notification_labels`. The core validates each label under the same bounds as tray labels (1 to 100 non-blank characters, no control characters).
   - Each notification uses the fixed catalog title for its event kind and the conversation's cleaned, length-capped title as its body. Model output, question text, reply text, tool arguments, error text and secrets never appear in desktop notifications.
6. Handle clicks on desktop notifications in Rust through `Notification::on_action`. Clicking a notification shows and focuses the main window and sends an open-conversation directive over the `notifications` IPC channel if the conversation still exists.
7. Persist unread finished conversations in a dedicated store module `notifications` with table `notifications_unread` (forward-only migration v1, [ADR-0009](0009-persist-state-in-sqlite.md)).
   - When an owner conversation completes or fails while that conversation is not currently open in a focused Mannie window, record its ID in `notifications_unread`.
   - Clear a conversation's unread record only when the owner views it in a focused window (opening it while focused, or focusing the window while it is already open), when a new run starts in that conversation, or when the conversation is deleted.
   - Existing conversations in upgraded databases start read.
   - Push the unread set to the window over the `notifications` channel so the Chat sidebar and collapsed projects render an unread dot using the theme's primary color token.
8. Render both the tray icon badge and the Windows taskbar count overlay in pure Rust with no extra crate:
   - For the system tray icon, composite a high-contrast dot onto a copy of the app's default RGBA icon and swap it through `TrayIcon::set_icon`, together with the localized attention tooltip, while `tray_badge` is enabled and at least one owner conversation is unread or waiting on a question or approval. Restore the plain icon and default tooltip once none remain or when `tray_badge` is turned off.
   - For the taskbar or dock icon, track the count of distinct owner conversations that are unread or waiting on a question or approval. While `taskbar_badge` is enabled and the count is greater than zero, render a 32x32 RGBA numbered badge (`1` to `99`, and `99+` for 100 or more) and apply it through `WebviewWindow::set_overlay_icon` on Windows, or pass the numeric count to `WebviewWindow::set_badge_count` on Linux. Clear the overlay or badge count when the count returns to zero or `taskbar_badge` is turned off, and reapply it when the main window is created or shown again from the tray.
9. Log notification daemon, taskbar or tray errors at `warn` level and continue without blocking runs. In validation builds, record dispatched notifications, click actions, tray badge state and taskbar badge count in memory instead of calling the operating system notification service or taskbar overlay API, so end-to-end tests stay isolated from the desktop session.
10. Outside this decision:
    - custom notification sounds, quiet hours and per-conversation mute rules;
    - inline toast action buttons for approving tools or answering questions outside Mannie;
    - unread indicators inside plugin-owned sidebars (such as Notebooks), which would require a plugin SDK change; plugin home conversations still notify, open on click and clear when opened;
    - numeric digits inside the 16px system tray icon itself (the tray keeps the plain dot for clarity while the taskbar or dock shows the number);
    - macOS (D6).

Earlier decisions that this ADR changes:

| Earlier decision                                                 | Change                                                                                                                            |
| ---------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| [ADR-0003](0003-build-the-desktop-app-on-tauri-2.md), decision 3 | Superseded in part: the desktop app adds `tauri-plugin-notification` as its fourth Tauri plugin, registered without a capability. |

## Consequences

- Positive:
  - The owner learns immediately when a background or tray run finishes, fails, asks a question or needs tool approval, and can jump straight to that conversation by clicking the notification.
  - Unread finished conversations remain visible in the sidebar and keep the tray badge lit until each one is opened, including across webview reloads and restarts.
  - Model output never reaches operating system notification banners, and script in the webview cannot post spoofed notifications.
  - All new Rust dependencies use `Apache-2.0 OR MIT` and reuse the `windows` 0.62 and `zbus` 5 crates already in `Cargo.lock`.
- Negative:
  - Adds one Tauri plugin and a `notifications` store module with one table and migration.
  - Uninstalled Windows development builds cannot rely on WinRT toast delivery without a registered Start Menu shortcut, so manual verification on Windows runs against an installed build.
  - Unread marks inside plugin-owned sidebars wait for a later plugin SDK addition.
- Risks:
  - Operating system notification daemons on Linux vary in how they deliver click callbacks, or may not be running at all. Failures log cleanly and never interrupt runs, and the tray badge and sidebar unread marks still show pending work.
  - A conversation title from a first message may contain sensitive text the owner typed. The core strips control characters and caps length, and the owner can disable notifications in settings.

## Alternatives considered

| Option                                                       | Why not chosen                                                                                                                                       |
| ------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| Grant `notification:default` to the webview                  | Lets untrusted webview script post spoofed system notifications, breaking [ADR-0004](0004-keep-system-access-in-the-rust-core.md).                   |
| Call Windows WinRT and Linux D-Bus directly without a plugin | Duplicates platform boilerplate that `tauri-plugin-notification` 2.8.1 maintains on the exact `windows` and `zbus` versions already in `Cargo.lock`. |
| Include question or reply text in notification bodies        | Exposes untrusted model output and private transcript text on lock screens and shared displays.                                                      |
| Keep unread finished state only in memory                    | Restarting or updating Mannie would erase unread marks for finished jobs the owner had not opened yet.                                               |
| Clear the tray badge as soon as the window gains focus       | The owner chose to keep the badge until every unread finished conversation has been opened.                                                          |
| Add an unread field to `ConversationSummary`                 | Touches the Workers link compatibility fixtures and runtime queries for a desktop-only concern that belongs to the notifications module.             |

## Validation

V14: unit and integration tests cover runtime signal emission (completed, failed, stopped, interrupted, questions, approvals, child tasks and deduplication), `notifications_unread` persistence and upgrade from an earlier database, tray icon RGBA badge compositing and numbered taskbar overlay icon rendering (`1` to `99` and `99+`), focus and viewed-conversation state transitions, label validation and capability checks. End-to-end tests on the validation build verify recorded notifications, simulated click navigation, sidebar and collapsed project unread marks, taskbar badge count and tray badge state, settings switches, keyboard operability, axe checks in both themes and the pseudo-locale. Manual checks on Windows 11 and a Linux desktop with tray and notification daemon verify live OS toasts, click-to-open, the numbered taskbar badge and the tray icon badge.
