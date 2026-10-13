# Mannie product vision

|              |                                                                                                                                                      |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Draft for review                                                                                                                                     |
| Last updated | 2026-10-13                                                                                                                                           |
| Related      | [Goals and scope](goals-and-scope.md), [Feature protocol](../features/README.md), [Milestones](../roadmap/milestones.md), [Glossary](../glossary.md) |

## 1. Summary

Mannie is a desktop AI agent harness for one person, its owner. It starts with three capabilities:

- Chat with AI models. Replies stream as they are generated, and conversations are stored safely on disk.
- System tools. The model can read, write, edit and search files and run commands on the owner's computer, under approval rules the owner controls.
- Work across machines. The desktop app, called the master, handles the owner's request. It hands tasks to child agents on the same machine, or to workers: autonomous agents that each run on a host and know how to work on it.

Everything else is added one feature at a time, when the owner asks for it.

## 2. The problem

The owner needs a harness shaped around their own setup. They use OpenAI models and their own OpenAI-compatible model servers, and their work is spread across Windows, Linux and FreeBSD machines, each with its own purpose. General-purpose AI tools are built for other setups and cannot be changed at the pace the owner's needs change.

Building a harness quickly without structure has a cost of its own: each new feature gets patched into the rest of the code until nothing can be changed safely. The [feature protocol](../features/README.md) and the [extension points](../architecture/extension-points.md) of the architecture exist to prevent that.

## 3. The owner

Mannie has one user, who also directs its development.

| Need                                          | How Mannie meets it                                                                            |
| --------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| Use OpenAI models and their own model servers | Provider profiles for the OpenAI Responses API and for OpenAI-compatible servers               |
| Let the model do real work on a computer      | System tools under approvals, rules and workspace defaults                                     |
| Get work done on other hosts                  | Workers that run on those hosts and take tasks from the master                                 |
| Keep control                                  | Explicit approvals, policy ceilings configured on each host, no permissions taken from content |
| Never lose work                               | Durable runs, crash-safe storage, and operations that are never repeated after a crash         |
| Add features when needs appear                | The feature protocol and an architecture that grows by addition                                |

There are no accounts, teams or tenants, and Mannie never phones home.

## 4. Product principles

These principles decide trade-offs. A design that conflicts with one of them needs an ADR that explains why.

1. Production quality from the first commit, with no prototype or MVP stage ([decision D2](../roadmap/milestones.md#5-decisions)). Every release meets the Definition of Done for the scope it ships.
2. Mannie grows feature by feature. New capabilities arrive through the [feature protocol](../features/README.md) and plug into named [extension points](../architecture/extension-points.md), so a feature changes its own code and its own document while the rest of the system stays as it was ([ADR-0002](../adr/0002-grow-through-extension-points.md)).
3. The owner stays in control. A side effect happens only after policy or the owner allows it. Content the model reads, such as files, command output, web pages or reports from other agents, never grants a permission.
4. Work survives failures. Runs continue through window reloads, crashes and dropped connections. An operation never runs twice, and an operation that may have run is never repeated automatically.
5. The master orchestrates and workers own their hosts. The master takes care of the owner's request. A worker takes care of the master's request and handles its host's environment on its own ([ADR-0015](../adr/0015-run-an-agent-runtime-on-every-machine.md)).
6. Provider credentials are stored only in the operating system's credential store on the master. Anywhere else they exist only in memory.
7. Mannie writes its own agent loop, provider adapters, tools and orchestration, and uses established components for infrastructure: Tauri, Svelte, SQLite, gRPC and rustls.
8. Data stays on the owner's machines, and Mannie sends no telemetry.
9. Code is written for the people and coding agents who will read and change it later.

## 5. What success looks like

- From the first release (v0.1.0), the owner can use Mannie for everyday chat with OpenAI models and their own model servers.
- The model reads, edits and runs things on the owner's workstation, and the owner never finds a side effect they did not allow.
- The owner hands a task to a worker on a FreeBSD or Linux host without explaining how that host works, and gets a confirmed report back.
- Crashes, reloads and dropped links do not lose conversations or tasks.
- A new feature lands as its own slice with its own document, and nothing unrelated changes.

## 6. What Mannie will not be

[Goals and scope](goals-and-scope.md#3-non-goals) lists every non-goal. These three affect most design decisions:

- Mannie is not a hosted or multi-user service. It runs on the owner's machines for the owner.
- Mannie is not a remote shell. The master never runs tools on a worker's host; it gives the worker a task.
- No release trades quality for scope.
