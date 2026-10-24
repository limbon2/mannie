# Git mentions

|              |                                                                                                                                                                                                                                            |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Building                                                                                                                                                                                                                                   |
| Plugin       | `git`                                                                                                                                                                                                                                      |
| Milestone    | M9                                                                                                                                                                                                                                         |
| Last updated | 2026-10-24                                                                                                                                                                                                                                 |
| Part of      | [Git](README.md)                                                                                                                                                                                                                           |
| Related      | [Mentions](../../../docs/features/chat/mentions.md), [ADR-0032](../../../docs/adr/0032-let-features-and-plugins-add-kinds-of-mentions.md), [ADR-0038](../../../docs/adr/0038-let-plugins-run-installed-programs-in-the-owners-projects.md) |

## Intent

Let the owner point the model at a commit, a branch or the uncommitted changes by typing "@", as they point it at a file.

## Behavior

- The provider `git` is offered in a conversation whose project lies in a repository, and in a new conversation of such a project.
- Typing "@" lists, under Git: "Uncommitted changes" when there are any, the newest local branches, and the newest commits of the current branch, matched against what the owner typed by name, subject or hash.
- The list reads the repository once for a conversation, or for the project of a message that starts one, and matches the keys that the owner types against that reading for 10 seconds, so typing runs git once instead of for every key. A change made in that time shows after it.
- A mentioned item gives the model, within the room that the mentions of a message share:
  - uncommitted changes: the status and the diff of the staged and the unstaged changes;
  - a commit: its hash, author, date, message, changed files and patch;
  - a branch: its commits since it left the default branch or its upstream, and the files that they change.
- Reading what a mention names runs git only to read, and only when the conversation's reads of the repository run without asking; otherwise the model reads that the owner mentioned it and that its content needs the owner's permission first.

## Design

- Keys: `changes`, `commit:<full hash>` and `branch:<name>`, which the provider checks again because the webview sent them. The tokens that an entry puts into the message are `@changes`, the commit's short hash and the branch's name.
- A branch's base is its upstream, or else `origin/HEAD`, `main` or `master`, whichever exists first; the branch's commits since the base's merge base are listed, at most 50.
- Every run of git for the list or a mention is checked first as a read of the repository's folders in the conversation, or in the project for a message that starts one (ADR-0038, decision 10). A read that would ask lists nothing, with the code `read_not_allowed`, and a mention gives the note instead.
- The list's reading holds whether there are changes, the branches and the commits, and is kept for 10 seconds for its place: the workspace, the conversation or the project, and whether the conversation is in full access, so another conversation, or a change of full access, reads again. Queries that arrive while it is read wait for that reading, a reading that fails is not kept, and at most 8 readings are kept.
- Summary codes for the UI: `changes`, `commit`, `branch` and `not_read`. The mention kind `git` shows them with the git branch, git commit and file diff icons.

## Acceptance

- Suggestions and resolving against a real repository, a mention of a repository that the conversation may not read, and a removed branch.
- The list reads the repository once while the owner types, again in another conversation, and again once its reading is past its lifetime.
