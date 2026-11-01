# Changelog of the terminal plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Added

- Terminals beside a conversation, as tabs of the side panel area, on this computer in its shells and on the hosts that plugins offer, such as SSH hosts, opened from the toolbar's Terminal menu, the command palette or Ctrl+`, with Find, a list of commands, End and Start again (ADR-0053).
- Agents run one command at a time in the conversation's terminals with `run_in_terminal`, under the master's policy, at an idle prompt and never while the owner types, and list them with `list_terminals`; the result cites the command's output by a link, which a reply's citation shows and marks in the terminal (ADR-0053, ADR-0054).
- Shells that mark their prompts and commands with a setup that the plugin injects on this computer and types on a remote host: bash, zsh, fish, sh, PowerShell and Command Prompt ([mark commands with shell integration](docs/adr/0001-mark-commands-with-shell-integration.md)).
- The owner points an agent at a command or a selection with "Mention in message", "Mention selection" and the "@" group Terminal.
- A headless emulator of each terminal in Rust, the only source of the text that a model reads, with a text log of 2 MiB for each terminal and 32 MiB in all, and views that get snapshots instead of a flood and nothing while hidden ([keep a headless emulator of each terminal in Rust](docs/adr/0002-keep-a-headless-emulator-of-each-terminal-in-rust.md)).
- Settings for the default shell of this computer, the scrollback, the font size, a blinking cursor, the renderer and screen reader mode.
