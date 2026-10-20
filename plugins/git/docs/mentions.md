# Git mentions

|              |                                                                                                                                                                                                                                            |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Building                                                                                                                                                                                                                                   |
| Plugin       | `git`                                                                                                                                                                                                                                      |
| Milestone    | M9                                                                                                                                                                                                                                         |
| Last updated | 2026-10-20                                                                                                                                                                                                                                 |
| Part of      | [Git](README.md)                                                                                                                                                                                                                           |
| Related      | [Mentions](../../../docs/features/chat/mentions.md), [ADR-0032](../../../docs/adr/0032-let-features-and-plugins-add-kinds-of-mentions.md), [ADR-0038](../../../docs/adr/0038-let-plugins-run-installed-programs-in-the-owners-projects.md) |

## Intent

Let the owner point the model at a commit, a branch or the uncommitted changes by typing "@", as they point it at a file.

## Behavior

- The provider `git` is offered in a conversation whose project lies in a repository, and in a new conversation of such a project.
- Typing "@" lists, under Git: "Uncommitted changes" when there are any, the newest local branches, and the newest commits of the current branch, matched against what the owner typed by name, subject or hash.
- A mentioned item gives the model, within the room that the mentions of a message share:
  - uncommitted changes: the status and the diff of the staged and the unstaged changes;
  - a commit: its hash, author, date, message, changed files and patch;
  - a branch: its commits since it left the default branch or its upstream, and the files that they change.
- Reading what a mention names runs git only to read, and only when the conversation's reads of the repository run without asking; otherwise the model reads that the owner mentioned it and that its content needs the owner's permission first.

## Design

- Keys: `changes`, `commit:<full hash>` and `branch:<name>`, which the provider checks again because the webview sent them. The tokens that an entry puts into the message are `@changes`, the commit's short hash and the branch's name.
- A branch's base is its upstream, or else `origin/HEAD`, `main` or `master`, whichever exists first; the branch's commits since the base's merge base are listed, at most 50.
- Every run of git for the list or a mention is checked first as a read of the repository's folders in the conversation, or in the project for a message that starts one (ADR-0038, decision 10). A read that would ask lists nothing, with the code `read_not_allowed`, and a mention gives the note instead.
- Summary codes for the UI: `changes`, `commit`, `branch` and `not_read`. The mention kind `git` shows them with the git branch, git commit and file diff icons.

## Acceptance

- Suggestions and resolving against a real repository, a mention of a repository that the conversation may not read, and a removed branch.
