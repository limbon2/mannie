# Mannie

Mannie is a desktop AI agent harness for one owner. It starts with three capabilities:

- Chat with AI models from OpenAI and from OpenAI-compatible servers such as vLLM, SGLang, LM Studio, Ollama and OpenRouter.
- System tools that let a model read, write, edit and search files and run commands on the owner's computer, under approvals and rules the owner controls.
- Work across machines: the desktop app orchestrates, child agents split work on the same machine, and workers run on other hosts, each taking care of its own host's environment.

New features are added one at a time through a [feature protocol](docs/features/README.md).

> Status: [M0 (Foundation)](docs/roadmap/milestones.md#m0-foundation). The engineering platform is in place: the desktop shell, typed IPC, the store, credentials, logging, the worker service, CI and the release pipeline. No feature ships yet. [M1 (Chat)](docs/roadmap/milestones.md#m1-chat) ends with v0.1.0, the first release.

## Principles

- Production quality from the first commit, with no prototype or MVP stage.
- Features plug into extension points, so adding one does not mean changing the others.
- Nothing changes on a machine unless policy or the owner allowed it.
- Runs, tasks and operations survive reloads, crashes and dropped connections.
- The master orchestrates, and workers own their hosts.
- Provider credentials stay in the operating system's credential store, and Mannie sends no telemetry.

## Technology

| Concern               | Choice                                                                             |
| --------------------- | ---------------------------------------------------------------------------------- |
| Desktop app           | Tauri 2 with a Rust core                                                           |
| User interface        | Svelte 5 and SvelteKit (static single-page app), shadcn-svelte, Tailwind CSS       |
| Worker                | A headless Rust service for Windows, Linux and FreeBSD                             |
| Storage               | SQLite through rusqlite                                                            |
| Master to worker link | gRPC with tonic over mutual TLS with pinned keys                                   |
| Model providers       | Mannie's own adapters for the OpenAI Responses API and Chat Completions            |
| Credentials           | The operating system credential store, through keyring-core                        |
| Interface text        | Message catalogs compiled by Paraglide JS; English first, ready for more languages |

## Components

| Component       | Role                                                                                                                            |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| Desktop app     | The master: the owner's conversations, tools on the owner's machine, provider profiles, workers and approvals                   |
| `mannie-worker` | A service on each host that runs tasks from the master with the host's own tools and knowledge, within a policy set on the host |

## Documentation

- [Documentation index](docs/README.md)
- [Vision](docs/product/vision.md) and [goals and scope](docs/product/goals-and-scope.md)
- [Feature protocol](docs/features/README.md)
- [Milestones](docs/roadmap/milestones.md)
- [Architecture overview](docs/architecture/overview.md)
- [Architecture decision records](docs/adr/README.md)
- [Contributing](CONTRIBUTING.md) and [agent rules](AGENTS.md)

## License

Apache-2.0. See [LICENSE](LICENSE).
