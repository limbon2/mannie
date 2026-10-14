# Window and tray

|              |                                                                                                                                                                                                                                                                            |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                   |
| Milestone    | M1                                                                                                                                                                                                                                                                         |
| Last updated | 2026-10-14                                                                                                                                                                                                                                                                 |
| Related      | [Desktop app](../architecture/desktop-app.md#7-window-and-lifecycle), [Chat](chat/README.md), [ADR-0003](../adr/0003-build-the-desktop-app-on-tauri-2.md), [ADR-0012](../adr/0012-run-agents-as-durable-actors.md), [ADR-0020](../adr/0020-complete-the-ui-shell-slots.md) |

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
- Extension points: a settings section for the window behavior, IPC commands to answer the first-close question and to change the setting, an overlay in the UI shell for the close dialog and the warning ([ADR-0020](../adr/0020-complete-the-ui-shell-slots.md)), and messages under the `window_` key prefix ([ADR-0019](../adr/0019-keep-visible-text-in-message-catalogs.md)).
- Must not change: run durability ([ADR-0012](../adr/0012-run-agents-as-durable-actors.md)). Quitting marks active runs as interrupted, exactly as any other exit of the process does.

## Open questions

None.

## Decisions

- 2026-10-13: the owner chooses between tray and quit on the first close, and can change it later (D13).
- 2026-10-13: quitting from the tray menu while runs are active warns in the same way as closing in quit mode, so that no path quits active runs without a warning.
- 2026-10-14: the defaults below were chosen when building started, and each one waits for the owner's review.
  - The tray icon is present in every mode, once the window has given it its labels.
  - When runs are active, the first-close question says so, and choosing Quit there needs no second warning.
  - The setting has three values: ask on the next close, keep running in the tray, and quit. Settings offer all three.
  - When the window cannot answer a question about closing within five seconds, for example because its page does not load, Mannie quits. Active runs become interrupted and can be retried.

## Design

### Setting

The settings section `window` holds `close_action`: `ask` (the default), `tray` or `quit`. The core keeps a copy in memory, so a close is decided without waiting for the database.

### Closing and quitting

The core decides what a close request does, in a function with unit tests for every combination:

| Close action | Active runs | What happens                                                                                               |
| ------------ | ----------- | ---------------------------------------------------------------------------------------------------------- |
| Ask          | Any         | The window stays and asks. With active runs, the question says how many and that quitting interrupts them. |
| Tray         | Any         | The window hides                                                                                           |
| Quit         | None        | Mannie quits                                                                                               |
| Quit         | Some        | The window stays and warns with the number of runs                                                         |

- "Quit" in the tray menu shows the window and follows the last two rows. "Open Mannie" shows the window and brings it to the front.
- The window learns about a question through a channel that it opens at startup (`subscribe_window_requests`), so no event permission is granted to the webview. A question that is pending when the window subscribes, for example after a reload, is sent again.
- The answers are commands: `answer_close_question` (tray or quit, saved before it takes effect), `confirm_quit`, and `dismiss_window_request`. `get_window_settings` and `update_window_settings` serve the settings section.
- Quitting stops the runtime, which marks active runs as interrupted and waits up to three seconds for them, then closes the store and the logs, and then ends the process.

### Tray

- The tray icon uses Tauri's `tray-icon` feature and the app's icon. The menu's labels are visible text, so they come from the catalog: the window sends them with `set_tray_labels` when it starts, and again after each reload, which also follows a change of language. Until then the tray has no icon.
- Windows: a left click opens the window, and a right click opens the menu. Linux: the menu opens on click, and the click itself is not delivered.
- Creating the tray can fail, for example without a tray host. Mannie logs it and continues; closing in tray mode still hides the window, and starting Mannie again shows it.
- On Linux, Tauri's tray loads the AppIndicator library when Mannie starts and panics without it. The deb package depends on `libayatana-appindicator3-1` and the rpm package on `libappindicator-gtk3`.

### Message keys

`window_` prefix: the close question, the quit warning, the settings section and the tray's labels.

### Threat notes

The webview can ask the core to hide the window, to quit, and to change the tray's labels. None of these reach files, processes or the network. The core accepts labels of at most 100 characters without control characters.

### Platform differences

| Topic        | Windows                                              | Linux                                                                                                            |
| ------------ | ---------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Tray         | The notification area; a left click opens the window | AppIndicator through the desktop's tray host; the menu opens the window. GNOME needs the AppIndicator extension. |
| Second start | The single-instance plugin shows the window          | The same, through D-Bus                                                                                          |

### Tests

WebDriver cannot press the window's close button or reach the tray, so validation builds have commands that call the same code: one closes the window as the close button does, one runs a tray menu item, and one reports whether the window is visible. Unit tests cover the decision table and the label checks. The tray itself is checked by hand.

## Acceptance

- The first close asks once. The answer survives restarts, and the setting changes it (end-to-end).
- In tray mode, closing the window while a reply streams keeps the run going, and reopening the window shows the whole reply without gaps (end-to-end).
- In quit mode, closing with active runs warns. Cancel keeps everything running. Quitting leaves the runs interrupted and retryable (end-to-end).
- Quitting from the tray menu with active runs warns (end-to-end through the validation command, and by hand).
- Starting Mannie again while it is in the tray shows the window (end-to-end).
- The behavior is checked by hand on Windows 11, on a Linux desktop with tray support, and on GNOME without the AppIndicator extension, and the results are recorded in the M1 validation results.
