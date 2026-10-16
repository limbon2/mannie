# ADR-0013: Perform every side effect as a journaled, authorized operation

|          |                                                                                                                       |
| -------- | --------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted                                                                                                              |
| Date     | 2026-10-13                                                                                                            |
| Deciders | Owner                                                                                                                 |
| Related  | [ADR-0014](0014-contain-process-trees.md), [Tools](../architecture/tools.md), [Security](../architecture/security.md) |

## Context

- Tools change files and run commands on the owner's machines. A crash or a dropped link in the middle of a change must not lead to the change running twice.
- Approvals must hold, including when a path changes between the approval and the execution.
- Text the model reads (files, command output, web pages, reports) may contain instructions that someone else wrote.

## Decision

1. Every tool call with a side effect becomes an operation with a journal record. The record moves through these states: prepared, awaiting approval, executing, and finally completed, denied, canceled or uncertain. The record is the tool call's part in the transcript: each change of its state is stored in the same transaction as its event.
2. Prepare computes the operation's canonical accesses without side effects: each path with its access mode, and each command with its shell and working directory.
3. Authorize checks policy (workspace defaults, grants, rules and, on a worker, the policy ceiling) and asks the owner when policy does not decide.
4. Just before executing, the tool prepares again and compares the accesses. Any difference sends the operation back to authorization.
5. The journal record is committed durably before execution starts. After a crash, operations that were executing become uncertain. They are reported to the model and to the owner, and never repeated automatically.
6. Read-only tools pass authorization but do not create journal records.
7. Tool results are typed and stored once. The model sees a rendered form of them.
8. Content the model reads never grants a permission.

## Consequences

- Positive: no operation runs twice, and every side effect leaves a record.
- Negative: the durable commit adds latency to each operation, and the UI has more states to show.

## Alternatives considered

| Option                                           | Why not chosen                                                                |
| ------------------------------------------------ | ----------------------------------------------------------------------------- |
| Execute directly and log afterwards              | A crash between the execution and the log leaves no reliable record           |
| An operating system sandbox instead of approvals | Worth adding later as a second layer; it cannot replace the owner's decisions |

## Validation

M2: crash tests at every journal state, a prompt-injection corpus that must cause no unapproved side effect, and a test that changes a path between the approval and the execution.
