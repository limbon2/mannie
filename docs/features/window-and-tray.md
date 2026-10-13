# Window and tray

|              |                                                                                                                                                                                                             |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Accepted                                                                                                                                                                                                    |
| Milestone    | M1                                                                                                                                                                                                          |
| Last updated | 2026-10-13                                                                                                                                                                                                  |
| Related      | [Desktop app](../architecture/desktop-app.md#7-window-and-lifecycle), [Chat](chat.md), [ADR-0003](../adr/0003-build-the-desktop-app-on-tauri-2.md), [ADR-0012](../adr/0012-run-agents-as-durable-actors.md) |

## Intent

When the owner closes the window, Mannie asks once whether to keep running in the system tray or to close entirely. It records the answer, and the owner can change it later. If the owner chose to close entirely, every later close while runs are active warns first.

## Behavior

- The first time the owner closes the main window, Mannie asks whether to keep running in the system tray or to quit when the window closes. The answer is saved as a setting.
- The settings show the choice, and the owner can change it at any time.
- Tray mode: closing the window hides it, and runs and worker links continue. The tray icon's menu has "Open Mannie" and "Quit". On Windows, clicking the tray icon also opens the window.
- Quit mode: closing the window quits Mannie. When runs are active, Mannie first warns how many runs are active and that quitting interrupts them, and that they can be retried later. The owner can quit anyway or cancel.
- Quitting from the tray menu while runs are active shows the same warning.
- Tasks on workers are not affected by either choice. They keep running on their workers.
- Starting Mannie again while it runs in the tray brings the window back.
- On Linux, clicking the tray icon does nothing, and the menu is used instead. On desktops without tray support, such as GNOME without the AppIndicator extension, the icon may not appear at all, and starting Mannie again brings the window back.
- Not included: notifications or badges for finished runs, and starting Mannie when the owner logs in. Each would be a feature of its own.

## Fit

- Uses Tauri's tray icon, a core feature of Tauri and not a plugin, and the single-instance plugin to bring the window back ([ADR-0003](../adr/0003-build-the-desktop-app-on-tauri-2.md)).
- Extension points: a settings section for the window behavior, IPC commands to answer the first-close question and to change the setting, UI shell slots for the close dialog and the warning, and messages under its own key prefix ([ADR-0019](../adr/0019-keep-visible-text-in-message-catalogs.md)).
- Must not change: run durability ([ADR-0012](../adr/0012-run-agents-as-durable-actors.md)). Quitting marks active runs as interrupted, exactly as any other exit of the process does.

## Open questions

None.

## Decisions

- 2026-10-13: the owner chooses between tray and quit on the first close, and can change it later (D13).
- 2026-10-13: quitting from the tray menu while runs are active warns in the same way as closing in quit mode, so that no path quits active runs without a warning.

## Design

Written when M1 starts.

## Acceptance

- The first close asks once. The answer survives restarts, and the setting changes it.
- In tray mode, closing the window while a reply streams keeps the run going, and reopening the window shows the whole reply without gaps.
- In quit mode, closing with active runs warns. Cancel keeps everything running. Quitting leaves the runs interrupted and retryable.
- Quitting from the tray menu with active runs warns.
- Starting Mannie again while it is in the tray shows the window.
- The behavior is checked on Windows 11, on a Linux desktop with tray support, and on GNOME without the AppIndicator extension.
