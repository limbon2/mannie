# Changelog of the notebooks plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Added

- The plugin Notebooks, off by default: notebooks of PDF, Markdown and text files of any size and pasted texts, indexed and searched through the owner's search profiles of an embedding model and, if the owner chooses one, a reranker. The Notebooks entry of the activity bar opens the latest notebook, whose page shows each source's state and progress, with "Use in answers", "Try again" and its text in a viewer that fills the window. The sidebar lists the notebooks, each with "New chat" and its own chats, which search the notebook with every message and stay out of the Chat list; "@" attaches a notebook to any other chat. Replies get the best passages, searched with the message and the owner's earlier messages, and the model lists, searches and reads the attached notebooks with three tools, and cites passages by links that open them in the viewer. A PDF is read page by page, so a page that fails costs only that page.

### Security

- tokio 1.53.3, which fixes the advisory RUSTSEC-2026-0386 in the constructors of `AsyncFd` on Unix, which neither the plugin's crates nor their dependencies call.
