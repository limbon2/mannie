# Notifications and tray badge

|              |                                                                                                                                                                                                                                                                                 |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                        |
| Milestone    | Unscheduled                                                                                                                                                                                                                                                                     |
| Last updated | 2026-11-21                                                                                                                                                                                                                                                                      |
| Related      | [ADR-0074](../adr/0074-send-desktop-notifications-and-badge-the-tray.md), [Window and tray](window-and-tray.md), [Questions to the owner](questions.md), [Conversation view](chat/conversation-view.md), [Security](../architecture/security.md), [Feature protocol](README.md) |

## Intent

The owner asked for Mannie to "send notifications when it asks questions or finishes its work" and to "see a little badge next to the icon in the tray" when a job is done. Today a run that finishes, fails or waits for input while Mannie is hidden in the tray or behind another window gives no signal outside the app. This feature sends desktop notifications when the owner is away, marks unread finished conversations in the sidebar, and badges the system tray icon until the owner has opened every finished conversation and no question or approval waits.

## Behavior

### Desktop notifications

- Mannie sends an operating system notification when an owner conversation needs attention or finishes while Mannie's main window is hidden in the tray or not focused:
  - the agent asks a question through `ask_owner`;
  - the agent (or a child agent or worker task reporting into that conversation) asks for tool approval;
  - a run completes;
  - a run fails.
- Runs stopped by the owner and runs interrupted by quitting or a crash never send notifications.
- Child agent and worker task conversations do not send notifications on their own. Their tool approval requests already surface on the parent conversation, which notifies once per approval part.
- A notification shows a fixed status title from the message catalog ("Waiting for your answer", "Approval needed", "Reply finished", "Reply failed") and the conversation's title as the body. Model output, question text, reply text, tool arguments and error details never appear in the notification.
- Clicking a desktop notification brings Mannie's main window to the front and opens that conversation, if it still exists.
- When Mannie's main window is visible and focused, no desktop notification is sent.
- If the operating system notification service is unavailable or refuses a notification, Mannie logs a warning and continues without interrupting runs.

### Unread finished conversations, taskbar count and tray badge

- When a run completes or fails in a conversation that is not currently open in a focused Mannie window, Mannie marks that conversation as unread.
- The Chat sidebar shows an unread dot on each unread finished conversation (below higher-priority marks for pending approvals, pending questions and active replies) and on a collapsed project that contains one.
- A conversation's unread mark clears when:
  - the owner opens that conversation while Mannie's window is focused;
  - Mannie's window gains focus while that conversation is already open;
  - a new run starts in that conversation;
  - the conversation is deleted.
- Unread marks persist across webview reloads and app restarts. Existing conversations when upgrading to this version start read.
- Mannie counts the distinct owner conversations that need attention: every unread finished conversation plus every conversation currently waiting on a question or tool approval.
- While the taskbar badge setting is on and that count is greater than zero, Mannie shows the number on its taskbar or dock icon (`1` to `99`, and `99+` for 100 or more). When the count returns to zero or the setting is turned off, Mannie clears the taskbar badge. Reopening Mannie from the tray reapplies the current count.
- While the tray badge setting is on and the count is greater than zero, Mannie adds a high-contrast badge dot to the system tray icon and updates its tooltip to show that attention is waiting.
- Once no unread finished conversation and no pending question or approval remain, Mannie restores the plain tray icon and default tooltip.

### Settings

- Settings gains a "Notifications" section with switches, all on by default:
  - Enable desktop notifications (master switch);
  - Questions from agents;
  - Tool approval requests;
  - Completed replies;
  - Failed replies;
  - Show count on taskbar icon;
  - Show badge on tray icon.
- Turning off the master switch disables all desktop notifications without changing either badge switch. Turning off the taskbar badge switch or tray badge switch immediately clears only that badge while keeping unread marks in the sidebar.
- The section has loading, saving and error states, works fully from the keyboard, and meets WCAG 2.1 AA in both themes.

### What this version does not do

- Custom notification sounds, quiet hours or per-conversation mute settings.
- Inline buttons on desktop notifications to approve tools or answer questions outside Mannie.
- Unread marks inside plugin-owned sidebars (such as Notebooks); plugin home conversations still send notifications, open on click and clear when opened.
- Numeric digits drawn inside the 16px system tray icon itself (the taskbar or dock icon carries the count while the tray icon keeps a plain dot for legibility).
- macOS support (D6).

## Fit

- Follows [ADR-0074](../adr/0074-send-desktop-notifications-and-badge-the-tray.md), which adds `tauri-plugin-notification` 2.8.1 without a webview capability, defines the runtime signal stream, keeps notification text in the message catalog and persists unread state.
- [Settings sections](../architecture/extension-points.md#38-settings-sections): adds the `notifications` section with defaults and validation.
- [Store tables and migrations](../architecture/extension-points.md#39-store-tables-and-migrations): adds the `notifications` module with `notifications_unread` (forward-only migration v1).
- [IPC commands and channels](../architecture/extension-points.md#36-ipc-commands-and-channels): adds `get_notification_settings`, `update_notification_settings`, `set_notification_labels`, `set_viewed_conversation` and `subscribe_notifications`, plus validation commands for end-to-end tests.
- [UI shell slots](../architecture/extension-points.md#37-ui-shell-slots): registers the Notifications settings section and a root overlay that sends localized labels, tracks the viewed conversation and applies click navigation.
- [Message catalogs](../architecture/extension-points.md#311-message-catalogs): adds keys under the `notifications_` prefix and sidebar unread labels under `chat_`.
- Must not change:
  - [Window and tray](window-and-tray.md) close and quit behavior, including the close question and quit warnings.
  - [Questions to the owner](questions.md) and tool approvals: clicking a notification only navigates to the conversation and never approves or answers anything.
  - Workers link contracts and fixtures.

## Open questions

None.

## Decisions

- 2026-11-21: the owner chose desktop notifications for questions, tool approvals, completed runs and failed runs, sent only when Mannie's window is hidden in the tray or unfocused.
- 2026-11-21: the owner chose to show the tray badge for finished runs while away and for waiting questions or approvals, and to clear it only after every unread finished conversation has been opened, with an unread mark in the sidebar.
- 2026-11-21: the owner accepted [ADR-0074](../adr/0074-send-desktop-notifications-and-badge-the-tray.md) (D69) with its defaults: fixed catalog titles with cleaned conversation titles and no model output, click-to-open navigation, SQLite persistence for unread finished conversations, and a Settings -> Notifications section with all switches on by default.
- 2026-11-21: the owner extended the badge design to show the numeric attention count (`1` to `99`, `99+`) on the taskbar or dock icon while keeping a plain red dot on the tray icon, with two separate switches under Settings -> Notifications (`Show count on taskbar icon` and `Show badge on tray icon`, both on by default).

## Design

### Contracts

- Settings section `notifications` (version 1): `NotificationSettings` with `enabled`, `onQuestion`, `onApproval`, `onCompletion`, `onFailure`, `taskbarBadge` and `trayBadge` (all `true` by default).
- Commands:
  - `get_notification_settings() -> NotificationSettings`
  - `update_notification_settings(UpdateNotificationSettings) -> NotificationSettings`
  - `set_notification_labels(NotificationLabels) -> ()` (validates each string: 1 to 100 non-blank characters, no control characters)
  - `set_viewed_conversation(SetViewedConversation) -> ()` (updates which conversation the main window currently shows)
  - `subscribe_notifications(Channel<ChannelBatch<NotificationUpdate>>) -> ()`
- Channel `NotificationUpdate`:
  - `unread_changed { conversationIds }` (current unread finished conversation IDs, replayed on subscribe)
  - `open_conversation { conversationId }` (sent on notification click; buffered across reloads)
- Validation commands (validation builds only):
  - `validation_set_window_focused(bool)`
  - `validation_notifications_state() -> ValidationNotificationsState` (recorded notifications, unread IDs, tray badge active flag, taskbar badge count, window focus flag)
  - `validation_click_notification(id: i32) -> bool`

### Data

- Store module `notifications` (migration v1) owns table `notifications_unread (conversation_id TEXT PRIMARY KEY, marked_at INTEGER NOT NULL) STRICT, WITHOUT ROWID`.
- On startup, the core loads `notifications_unread` and prunes rows whose conversation no longer exists in `runtime_conversations`.

### Runtime signals and badge rendering

- `mannie-runtime`'s event hub broadcasts `OwnerSignal` (`Active`, `Completed`, `Failed`, `AwaitingAnswer`, `AwaitingApproval`, `EndedWithoutAlert`, `Deleted`, `Shutdown`) for owner conversations only (never task conversations).
- `mannie-desktop-core`'s notifications module runs a background task subscribed to `runtime.subscribe_signals()`, drives the pure state machine (`notifications::state`), persists unread marks to `Store`, dispatches OS toasts through `tauri-plugin-notification` (or records them in validation builds), updates the tray icon through `window::tray::apply_badge`, and updates the main window's taskbar or dock count badge (`set_overlay_icon` on Windows, `set_badge_count` on Linux).
- `notifications::badge` composites an anti-aliased badge dot with a contrasting border ring onto a copy of `app.default_window_icon()`'s RGBA pixels for the tray, and renders a 32x32 RGBA numbered overlay icon (`1` to `99` and `99+`) for the Windows taskbar in pure Rust.

### Message keys

- `notifications_` prefix: settings section title, description, switches, loading/saving/error messages, notification toast titles, attention tray tooltip and error codes.
- `chat_` prefix: `chat_conversation_finished_unread` and `chat_project_finished_unread`.

### Threat notes

- No `notification:*` capability permission is granted to any webview (T-111).
- Notification titles come strictly from validated catalog labels and bodies carry only the cleaned, 120-character-capped conversation title, never model output, question text, reply text, tool arguments or secrets (T-112).
- Clicking a notification only shows the main window and navigates to the conversation; it never approves an operation or answers a question.

### Platform differences

| Topic         | Windows                                                                                          | Linux                                                                                  |
| ------------- | ------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------- |
| Notifications | WinRT toast through `tauri-winrt-notification`; installed builds use `io.github.limbon2.mannie`  | D-Bus `org.freedesktop.Notifications` through `notify-rust` and `zbus`                 |
| Click action  | `Notification::on_action` reports `"tap"` while the toast is on screen                           | `Notification::on_action` reports `"tap"` through the server's `ActionInvoked` signal  |
| Taskbar badge | `WebviewWindow::set_overlay_icon` draws a 32x32 RGBA count badge (`1` to `99`, `99+`) on taskbar | `WebviewWindow::set_badge_count` updates the Unity LauncherEntry count where supported |
| Tray badge    | `TrayIcon::set_icon` swaps the icon RGBA buffer in the notification area and updates the tooltip | `TrayIcon::set_icon` swaps the AppIndicator icon RGBA buffer and updates the tooltip   |

## Acceptance

| Check                                                                                              | How it is tested                                                              |
| -------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| Settings, labels, validation bounds and channel message contracts serialize and validate           | `crates/mannie-types/src/notifications.rs` unit tests                         |
| `notifications_unread` persists, clears, prunes deleted rows and upgrades older databases          | `crates/mannie-store/src/notifications.rs` and store upgrade tests            |
| Runtime signals fire for completed, failed, questions, approvals and delete; skip tasks and stops  | `crates/mannie-runtime/tests/notifications.rs`                                |
| Focus/view gate, deduplication, settings toggles, unread counts, transitions and click lookup work | `apps/desktop/core/src/notifications/state.rs` unit tests                     |
| Tray dot compositing and numbered taskbar overlay rendering (`1` to `99`, `99+`) draw cleanly      | `apps/desktop/core/src/notifications/badge.rs` unit tests                     |
| No capability file grants any `notification:*` permission                                          | `apps/desktop/core/src/notifications.rs` capability test                      |
| Sidebar and collapsed project unread marks, settings switches and messages render accurately       | Vitest (`project-groups.test.ts`, `notifications-messages.test.ts`, UI)       |
| Unfocused alerts, taskbar count, tray badge, notification click, unread clearing and settings work | End to end: `tests/e2e/specs/notifications.test.ts`                           |
| Keyboard operation, axe WCAG 2.1 AA in both themes and pseudo-locale hold                          | End to end: `tests/e2e/specs/notifications.test.ts` and `chat-locale.test.ts` |
