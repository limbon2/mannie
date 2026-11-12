# Mannie

Mannie is a desktop AI agent harness for one owner. It chats with AI models, runs tools on the owner's computer under approvals the owner controls, and can hand work to other machines.

The core covers what every conversation needs:

- Chat with OpenAI models, with the owner's ChatGPT plan, and with OpenAI-compatible servers such as vLLM, SGLang, LM Studio, Ollama and OpenRouter.
- System tools that read, write, edit and search files and run commands, in the foreground or the background, under rules and approvals.
- Child agents that split work on the same machine, and custom agents that the owner defines without code, such as an Orchestrator and a Plan agent that saves plans to run later.
- Projects, "@" mentions of files and folders, questions that an agent asks the owner during a reply, skills installed with the `skills` CLI, and Markdown export of a conversation.

Everything else is a plugin that the owner turns on and off in the settings:

| Plugin                                        | What it adds                                                                                                            |
| --------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| [Workers](plugins/workers/docs/README.md)     | A service on other Windows, Linux and FreeBSD hosts that takes tasks from the desktop app, and SQLite databases on them |
| [Images](plugins/images/docs/README.md)       | Images in the owner's messages, from a button, a paste or a drop                                                        |
| [SSH](plugins/ssh/docs/README.md)             | Commands on computers without Mannie that the owner registered                                                          |
| [Git](plugins/git/docs/README.md)             | Tools for the owner's repositories, run with the installed git                                                          |
| [MCP](plugins/mcp/docs/README.md)             | The tools, resources and images of MCP servers, local and remote                                                        |
| [Notebooks](plugins/notebooks/docs/README.md) | Notebooks of PDF, Markdown and text sources that conversations search and cite                                          |
| [Terminal](plugins/terminal/docs/README.md)   | Terminals of this machine and of SSH hosts beside a conversation, where the owner types and agents run commands         |

New features and plugins go in one at a time through the [feature protocol](docs/features/README.md).

## Status

v0.6.0 was released on 2026-11-09 with the MCP and Notebooks plugins. The next release is being built from [M14 to M19](docs/roadmap/milestones.md#m14-terminals): terminals, databases, questions to the owner, custom agents and plans, skills, and the ChatGPT plan. [CHANGELOG.md](CHANGELOG.md) lists what each release added, and the [milestones](docs/roadmap/milestones.md) list what comes next.

## Principles

- Production quality from the first commit, with no prototype or MVP stage.
- Features plug into extension points, so adding one leaves the others alone.
- Nothing changes on a machine unless policy or the owner allowed it, and every side effect is journaled.
- Runs, tasks and operations survive reloads, crashes and dropped connections.
- The desktop app orchestrates, and workers own their hosts.
- The core never depends on a plugin.
- Provider credentials stay in the operating system's credential store, and Mannie sends no telemetry.

## Technology

| Concern               | Choice                                                                                        |
| --------------------- | --------------------------------------------------------------------------------------------- |
| Desktop app           | Tauri 2 with a Rust core                                                                      |
| User interface        | Svelte 5 and SvelteKit (static single-page app), shadcn-svelte, Tailwind CSS                  |
| Worker                | A headless Rust service for Windows, Linux and FreeBSD                                        |
| Storage               | SQLite through rusqlite                                                                       |
| Master to worker link | gRPC with tonic over mutual TLS with pinned keys                                              |
| Model providers       | Mannie's own adapters for the OpenAI Responses API and Chat Completions, and the ChatGPT plan |
| Credentials           | The operating system credential store, through keyring-core                                   |
| Interface text        | Message catalogs compiled by Paraglide JS, English first and ready for more languages         |

## Components

| Component       | Role                                                                                                                                   |
| --------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| Desktop app     | The master: the owner's conversations, tools on the owner's machine, provider profiles, approvals and the plugins that are on          |
| `mannie-worker` | The Workers plugin's service on each host, which runs tasks from the master with the host's own tools, within a policy set on the host |

## Documentation

- [Documentation index](docs/README.md)
- [Vision](docs/product/vision.md) and [goals and scope](docs/product/goals-and-scope.md)
- [Feature protocol](docs/features/README.md)
- [Milestones](docs/roadmap/milestones.md)
- [Architecture overview](docs/architecture/overview.md)
- [Architecture decision records](docs/adr/README.md)
- [Setting up a worker](plugins/workers/docs/guides/worker-setup.md)
- [Contributing](CONTRIBUTING.md) and [agent rules](AGENTS.md)

## License

Apache-2.0. See [LICENSE](LICENSE).
