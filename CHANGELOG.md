# Changelog

All notable changes to Mannie are recorded in this file. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/). Each change adds one line under Unreleased, and a release turns that section into its notes.

## Unreleased

### Added

- System tools for the model on the owner's computer: system information, listing, finding and searching files, reading, writing and editing text files with revisions, creating, copying, moving and deleting files and folders, running commands in Windows PowerShell or `/bin/sh`, and reading back long outputs.
- Projects in the sidebar: "+" turns a folder into a project, its conversations work in that folder, a general conversation moves into a project when the owner's request is about it, and removing a project deletes its conversations; reads inside a project's folder run without asking, and changes ask unless the settings allow them.
- Approvals that show exactly what will happen, with Allow once, Allow for this conversation, this project or always, Save rule… and Deny, and lists of the permissions that apply, which the owner can revoke.
- Tool calls in the reply as compact rows that the model names in a few words, such as "Reading package.json", with their details and full output on demand; the status line and the conversation list say when a reply waits for an approval.
- Commands that end with every process they started, through Job Objects on Windows and a subreaper on Linux, and Stop that ends a running command within a second.
- A journal of every operation, so that a crash never repeats one and an operation that may have run is shown as such.
- The Tools settings: changes inside the workspace without asking, the command rules, the grants for projects and every conversation, and the environment variables that commands receive.
- A "Use tools" switch for each provider profile, and a Tools check in the provider test.
- Evaluations of the system tools with real models and with a scripted oracle, `task tools:eval` and a weekly workflow.
- Child agents: the conversation agent gives self-contained tasks to explore children, which only read and search, and general children, which can also change files and run commands, on the same computer and in the conversation's workspace, in parallel, in the background, and as follow-ups that keep the child's context; only each child's confirmed report reaches the conversation.
- The card of each task in the reply, with the child's number, title, state and current activity, the child's approval requests to answer in place, Stop for one child, and the instruction and the report on demand.
- Side panels beside the main view: "Open conversation" shows a child's whole conversation read-only, with a tab for each open conversation that has its own Close, a resizable split that works from the keyboard, and Close the side panel in the command palette.
- A "Child agents" button in the conversation's toolbar that says how many child agents work, lists them, and opens a child's conversation in a side panel.
- The Child agents settings: how many child agents work at the same time, from 1 to 16, 4 by default.
- Evaluations of tasks split across child agents.

### Fixed

- The conversation view stops following a reply only when the owner scrolls up; a tool call or an approval request that appeared at once, or a message box that shrank after sending, could end the following and leave the newest activity out of view.
- A long reply no longer stretches the pane around the conversation: the text that only screen readers read escaped the transcript's scrolling, so the wheel could scroll the whole view away into empty space.
- The message box keeps the focus after the first message of a new conversation; the conversation's own pane replaced the new conversation's, and the focus fell to the page.
- A collapsed project expands whenever the open conversation moves into it; a move that came in the same batch of updates as the new conversation, or before the view showed the conversation, left it collapsed.

### Security

- rustls 0.23.46, which fixes the advisories RUSTSEC-2026-0346, RUSTSEC-2026-0347, RUSTSEC-2026-0348 and RUSTSEC-2026-0349.

## 0.1.0 - 2026-10-14

### Added

- Documentation baseline: vision, goals and scope, the feature protocol, architecture, architecture decision records, milestones, engineering standards and agent rules.
- Documentation checks, run with `task docs:check`.
- Decisions on the interface language, closing the window, code signing and release hosting, with the window and tray feature and ADR-0019 for message catalogs.
- The Cargo workspace on a pinned Rust 1.99 toolchain, with the `mannie-types`, `mannie-store`, `mannie-logging` and `mannie-secrets` crates, the desktop app and the worker.
- The desktop shell: Tauri 2 with the security baseline of ADR-0004, a SvelteKit single-page app built from shadcn-svelte components, a sidebar, a command palette (Ctrl+K), the settings view (Ctrl+,), light and dark themes, and one running instance.
- Typed IPC bindings generated from Rust (`task gen`), and channel streams in batches paced by the display's frames.
- The SQLite store, with migrations, a copy of the database before each migration, and refusal of a database from a newer version.
- Credentials in Windows Credential Manager and the Linux Secret Service; without a Secret Service, Mannie stores none.
- Logs in daily rotating files, with secrets redacted, and crash files.
- The worker: a headless service that starts, logs and opens its store, and installs itself as a Windows service, a systemd unit or an rc.d service with `mannie-worker service install`.
- The interface language as a setting, with English message catalogs and a pseudo-locale that the end-to-end tests check.
- Mannie's application icon.
- CI on Windows and Linux, nightly worker tests on FreeBSD, and a release workflow that builds, checks, signs and drafts a release from a version tag.
- The tasks `gen`, `dev`, `build`, `lint`, `test`, `e2e` and `release:prepare`.
- The shadcn-svelte agent skill and the Svelte MCP server for coding agents.
- Chat with OpenAI models and OpenAI-compatible servers, such as vLLM, SGLang, LM Studio, Ollama and OpenRouter, through the Responses API or Chat Completions, with replies that stream into the conversation view.
- Provider profiles in the settings, with credentials in the system's credential store, the known limits of common models filled in, and a provider test of the credential, the server, the model and a short reply.
- Replies that continue in the core while the window reloads or hides: Stop, also with Escape, Retry, automatic retries that wait as long as the provider asks, and replies that a crash or a quit interrupted, which can be retried.
- The conversation list, with rename and delete; a deleted conversation's text is overwritten in the database.
- Model output as Markdown: raw HTML shows as text, images are never loaded, and a link opens only after the owner confirms its address.
- Reasoning shown apart from the answer, and encrypted reasoning sent back only to the profile that wrote it.
- The context budget of each profile, with a meter, automatic compaction into a summary near the limit, and Compact now.
- Instructions from the owner that apply to every conversation.
- A note under a reply when a server, such as Ollama beyond a model's context size, cut the request without an error.
- The choice on the first close between running in the system tray and quitting, the tray menu, and a warning before quitting while replies are being written.
- The tasks `e2e:perf`, `bench`, `provider:live` and `provider:record`, a nightly workflow of live provider tests, and end-to-end tests that keep the app's window off the screen.
- The settings as a page of their own, with a page per section in the settings sidebar and a link back to the main view.
- A provider profile dialog that shows example values in empty fields, says when an API key is saved, and warns before saving replaces or removes it; icon buttons for Send and Retry; and plainer wording across the interface.
