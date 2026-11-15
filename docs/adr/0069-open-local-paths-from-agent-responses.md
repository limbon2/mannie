# ADR-0069: Open local paths from agent responses

|          |                                                                                                                                                                                         |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D64)                                                                                                                                                                          |
| Date     | 2026-11-14                                                                                                                                                                              |
| Deciders | Owner                                                                                                                                                                                   |
| Related  | [ADR-0004](0004-keep-system-access-in-the-rust-core.md), [ADR-0021](0021-render-model-output-as-markdown-with-micromark.md), [Conversation view](../features/chat/conversation-view.md) |

## Context

The owner approved the implementation plan on 2026-11-14: Markdown links and paths occupying an entire inline-code span open in the desktop's file manager immediately, and an Open in submenu offers the file manager alone. ADR-0004's decision 7 and ADR-0021's decision 4 currently describe only confirmed web links.

## Decision

1. Refine ADR-0004's decisions 2 and 7 and ADR-0021's decisions 2 and 4 for local paths. An owner's click or menu selection requests local navigation without another confirmation. This is an owner navigation action, like opening a confirmed web link, not an agent tool call or a grant for future tools. Record its outcome through structured tracing; it creates no tool operation or transcript part.
2. Render local references as buttons with no content-derived navigation attributes. Recognize entire inline-code paths conservatively, never prose, fenced code or commands, and never nest a path button inside another link. Keep web confirmation and citations unchanged.
3. Rust resolves and validates every request. Relative paths use the project of the containing conversation, including a child conversation; without a project they fail. Absolute local paths may lie outside that project. Only the fixed file-manager target is accepted. No caller supplies a program or command line.
4. Resolve symbolic links, require an existing regular file or directory, and validate again when opening. Accept native paths and local file URLs; reject controls, device paths, remote file URLs and UNC paths. Source locations such as a line or column suffix identify the underlying file, with an existing literal filename taking precedence.
5. Reveal a file in its containing folder and open a directory itself. Use the existing opener dependency only from Rust, without registering its JavaScript commands. On Linux a reveal that is unavailable falls back to opening the containing folder. Neither rendering nor resolution launches a program.
6. The context menu offers Open in, with File Explorer on Windows and File manager on Linux. Loading and errors remain accessible. More applications, remote paths and context-aware actions are unscheduled; no new plugin extension point or stored setting is introduced.

## Consequences

- Positive: the owner reaches files and folders directly from replies, including existing inline-code references.
- Negative: conservative detection leaves ambiguous references as code, and remote paths cannot open on the desktop.
- Risks: model text can name an unintended local target. Only an owner's action opens a validated location, never the file's associated program or executable. Native file-manager failures are reported without repeating the action automatically.

## Alternatives considered

| Option                               | Why not chosen                                                      |
| ------------------------------------ | ------------------------------------------------------------------- |
| Confirm each path                    | The owner chose immediate navigation.                               |
| Discover every installed application | The first version offers the file manager only.                     |
| Put file URLs in anchor elements     | The webview must not navigate or contact a host from model content. |

## Validation

Extend the Markdown hostile-content tests and desktop validation harness. Test path resolution, source suffixes, project context, denied targets, menu and keyboard behavior, and failures. Validation builds record file-manager actions without launching them. Check real file-manager behavior on Windows and Linux.
