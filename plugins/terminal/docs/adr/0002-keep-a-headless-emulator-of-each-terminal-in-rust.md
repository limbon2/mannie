# ADR-0002: Keep a headless emulator of each terminal in Rust

|          |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D45)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Date     | 2026-10-30                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| Deciders | Owner                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| Related  | [Terminals](../README.md), [mark commands with shell integration](0001-mark-commands-with-shell-integration.md), [ADR-0004](../../../../docs/adr/0004-keep-system-access-in-the-rust-core.md), [ADR-0017](../../../../docs/adr/0017-license-and-dependency-policy.md), [ADR-0053](../../../../docs/adr/0053-let-plugins-offer-terminals.md), [ADR-0054](../../../../docs/adr/0054-let-tool-results-keep-what-they-cite-and-let-plugin-views-add-mentions.md), [M14 validation results](../../../../docs/roadmap/m14-validation.md), [Goals and scope](../../../../docs/product/goals-and-scope.md#41-performance) |

## Context

- The model reads a command's output as text with numbered lines, and a reply cites lines of it ([ADR-0054](../../../../docs/adr/0054-let-tool-results-keep-what-they-cite-and-let-plugin-views-add-mentions.md)). That text must come from Rust: the webview is untrusted ([ADR-0004](../../../../docs/adr/0004-keep-system-access-in-the-rust-core.md)) and may be closed while an agent's command runs.
- A terminal's text is what an emulator makes of its stream: a progress bar that redraws its line, cursor moves, a clear, wide characters, and full-screen programs on the alternate screen. A command's rows must stay addressable after they scroll off the screen.
- The owner asked that terminals cause no lag on low-end laptops ([D45](../../../../docs/roadmap/milestones.md#5-decisions)). NFR-PERF-07 to 09 bound the page's work while a terminal floods, the cost of idle and hidden terminals, and memory: 5 MiB in the core for each terminal, and 10 MB in the webview for each open tab ([goals and scope](../../../../docs/product/goals-and-scope.md#41-performance)).
- Facts verified on 2026-10-30: alacritty_terminal 0.26.0 (Apache-2.0) and vt100 0.18.1 (MIT) are terminal emulators in Rust that run without a window. vt100 depends on itoa, unicode-width and vte.
- S3 compared them on recorded sessions ([M14 validation results](../../../../docs/roadmap/m14-validation.md)):
  - vt100 with a scrollback of 0 hands every row that leaves the screen to `on_scroll`, which can write it styled (`RowContents::write_formatted_basic`), and every unknown OSC sequence to `unhandled_osc`, at its place in the stream and with the cursor's position.
  - It kept exactly `line 1` to `line n` of real ConPTY sessions of PowerShell and Git Bash with 1, 50 and 3,000 lines, and all 262,144 lines of a flood across scrollback, in order. A progress bar redrawn with CR left its final line, three lines redrawn with cursor-up left their final state, wide characters and emoji stayed intact, an end marker with a wrong nonce was ignored with the output kept, and a full-screen program on the alternate screen stayed out of the block.
  - vt100 parsed 30 MiB/s while it captured each row styled and plain, and 54 MiB/s without capture. alacritty_terminal parsed 120 MiB/s without capture. A cell takes 32 bytes in vt100 and 24 in alacritty_terminal.
  - A snapshot of the last 2,000 styled lines and the screen took 156 KiB and was built in 0.2 ms.
  - alacritty_terminal has no hook for rows that scroll or for unknown OSC sequences, and it brings polling, miow, piper, parking_lot, regex-automata and windows-sys 0.59.
  - ConPTY delivered a flood at 1.8 MiB/s (S1), so vt100 with capture keeps up about 15 times over.

## Decision

1. The plugin keeps a headless emulator of each terminal in Rust, with the crate vt100 0.18.1 (MIT), which S3 chose and which only this plugin may use, as its manifest's `[crates]` records.
2. vt100 runs with a scrollback of 0:
   - Its `on_scroll` receives every row that leaves the screen, which numbers rows absolutely and fills the text log.
   - Its `unhandled_osc` receives the setup's markers ([mark commands with shell integration](0001-mark-commands-with-shell-integration.md)) at their place in the stream, so no second scan of the stream is needed.
   - So a block's rows stay addressable across scrollback.
3. Each terminal's emulator runs on a thread of its own, so a flood never holds up the async runtime that runs replies.
4. Memory:
   - The emulator keeps the screen and 200 rows of history. Older rows move to the text log as compact styled lines, from which plain text is derived when it is needed (S3).
   - A text log holds 2 MiB, and all text logs together at most 32 MiB. Past that, the oldest lines of the largest idle logs go first.
   - A block's text keeps its first 256 KiB and its last 768 KiB.
5. Views and floods:
   - A view gets output only while it can show it: its tab is the visible one, its conversation is open and the window is visible. A hidden view's subscription pauses, and Rust keeps the state.
   - The view acknowledges what xterm.js has processed, every 128 KiB or 100 ms.
   - When more than 512 KiB wait unacknowledged, Rust stops sending that view's bytes and sends a snapshot of the screen at most every 100 ms instead.
   - When the output calms down or the view catches up, Rust sends a full snapshot and streams again. A full snapshot holds the latest styled lines of the text log, up to the view's scrollback, and the screen, encoded as VT sequences.
   - A view that opens, resumes or reloads starts from a full snapshot.
   - So a flood becomes a jump to the current state, and the webview's work for each frame stays bounded on a slow machine. A slow view never slows the program in the terminal. Only the emulator could, at about 30 MiB/s on the workstation (S3), 15 times what ConPTY delivers.
   - S5 settles the values of the acknowledgment step, the watermark and the snapshot interval.
6. The page's work stays small, and idle terminals cost nothing:
   - It writes into xterm.js in pieces of at most 32 KiB, so that xterm.js can yield between them.
   - Output goes straight into xterm.js and never through Svelte state. Svelte state holds the list of terminals, their states and the command list, which change on marks and changes of state only, and the command list shows the last 200 commands.
   - The view fits its size on animation frames. The resize of the pseudo terminal waits 100 ms and comes once more at the end of a drag, because each resize makes the shell and ConPTY redraw.
   - Every wait is driven by events, such as marks, output and acknowledgments, and nothing polls. A hidden or minimized window pauses every view.
   - The Rust side starts nothing until a terminal opens.
7. UTF-8 is decoded across the boundaries of chunks.
8. The emulator is the only source of text for the model. Its differences from xterm.js are risk R45.

## Consequences

- Positive: the model's text and the blocks come from one place in Rust, which outlives every view. A flood or a hidden terminal costs the webview nothing that the owner does not see, a slow view never slows the shell, and each terminal's memory stays bounded.
- Negative: two emulators parse every byte, vt100 in Rust and xterm.js in the view, and they can differ (R45). A view that falls behind jumps to the current state, so the owner sees a flood's latest screen without the scroll that led there. A text log drops its oldest lines past its limits, so a long-lived terminal forgets its start. A cell of vt100 takes a third more memory than one of alacritty_terminal. Growing the screen without scrollback inserts blank rows at its top, so a resize during a command can add blank lines to its text (S3).
- Risks: on Linux a pseudo terminal can deliver faster than the emulator parses, so there the emulator sets the limit of a flood; NFR-PERF-07 and the benchmark of the emulator watch it. A change of vt100's hooks would break the blocks, so the crate is pinned, and the plugin's tests on recorded sessions run with every upgrade.

## Alternatives considered

| Option                                                       | Why not chosen                                                                                                                                                          |
| ------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| alacritty_terminal                                           | It parses faster, but it has no hook for rows that scroll or for unknown OSC sequences, and it brings an event loop, code for pseudo terminals and a second windows-sys |
| Parsing markers in the webview                               | The webview is untrusted and may be closed while an agent's command runs; Rust sends the marks on the same channel as the output                                        |
| Holding back the shell while a view is slow, as VS Code does | It slows the program, such as a `cat` of a large file; snapshots keep a slow view current without slowing anything                                                      |
| Feeding every byte to every view                             | A flood or a hidden terminal would keep a low-end laptop's page busy parsing output that nobody sees                                                                    |

## Validation

M14:

- The plugin's tests: blocks, absolute rows and the text log, forged markers, alternate screens, wide characters and the limits of scrollback; a slow view that falls behind and gets snapshots, a paused view that gets nothing, and the caps of memory.
- The benchmark of the emulator and the scanner on recorded sessions, `plugins/terminal/crates/mannie-terminal/benches/emulator.rs`, which `task bench` runs.
- S3, which ran, and S5 on the low-end profile ([M14 validation results](../../../../docs/roadmap/m14-validation.md)).
- `task e2e:perf` meets NFR-PERF-06 to 10 on the reference machine and under the low-end profile.
