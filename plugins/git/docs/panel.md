# Git panel

|              |                                                                                                                                                                                                        |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Building                                                                                                                                                                                               |
| Plugin       | `git`                                                                                                                                                                                                  |
| Milestone    | M9                                                                                                                                                                                                     |
| Last updated | 2026-10-20                                                                                                                                                                                             |
| Part of      | [Git](README.md)                                                                                                                                                                                       |
| Related      | [ADR-0025](../../../docs/adr/0025-add-side-panels-to-the-shell.md), [ADR-0038](../../../docs/adr/0038-let-plugins-run-installed-programs-in-the-owners-projects.md), [Panel actions](panel-actions.md) |

## Intent

Show the owner the repository that the agents work in, beside the conversation, without leaving Mannie.

## Behavior

- A conversation in a repository's project has a Git control in its toolbar with the branch's name, which opens the Git side panel for the conversation's project. The control reads the branch when the conversation opens, when a turn starts and when its run ends. Where the project is no repository, or cannot be read without asking, the toolbar shows nothing of it.
- The panel's header shows the branch or the detached HEAD, the upstream with how far ahead and behind it is, the operation in progress with its step, and the number of stash entries.
- Its tabs:
  - Changes: the conflicted, staged, unstaged and untracked files; choosing a file shows its diff, and "Back" returns to the files.
  - History: the newest commits of the current branch, 50 at a time, more on "Show more"; choosing a commit shows its message, files and diff.
  - Branches: the local branches with their upstreams, the remote branches and the tags, each with its last commit.
  - Conflicts, while an operation is in progress or a file is conflicted: each conflicted file with its hunks and their sides. The tab goes away with the conflicts, and the panel then shows Changes.
- The panel reads the repository again after each of the plugin's own changes in a repository, on "Refresh", and every 15 seconds while it shows, so changes made in a terminal show at the next read. After a read fails, it waits twice as long before the next one, up to 2 minutes. Each read also reads the tabs that were opened again, and the history from its first page when HEAD moved.
- A value that shows stays while it is read again, and a read that fails keeps it, with what failed above it.
- The panel only reads: it does not stage, commit or abort ([panel actions](panel-actions.md)).
- A repository whose git folder lies outside the project shows that Mannie needs the owner's permission to read it, which "Allow for this project" on an agent's request gives.

## Design

- Commands of the plugin, each with the project's ID and never a path: `repository_overview`, `repository_history`, `repository_branches`, `repository_commit`, `repository_file_diff`, `repository_conflicts`, and the channel `follow_repository`, which says when the plugin's own runs changed the repository. Each runs git only to read, with the project's folder and git folder checked as a model's reads in that project (ADR-0038, decision 10).
- Errors: `project_not_found`, `not_a_repository`, `bare_repository`, `git_missing`, `git_too_old`, `dubious_ownership`, `read_not_allowed`, `unknown_commit`, `locked`, `timed_out` and `git_failed`.
- The panel's state lives in `ui/state/git-repository.svelte.ts`: each value keeps only its newest read, so a slow answer never replaces a newer one, and a new watch of the repository ends the one before it.

### Loading, empty and error states

| View      | Loading                                                        | Empty                                                        | Error                         |
| --------- | -------------------------------------------------------------- | ------------------------------------------------------------ | ----------------------------- |
| The panel | Placeholders, and "Loading the repository…" for screen readers | "The project's folder is not in a git repository."           | What failed, with "Try again" |
| Changes   | The panel's                                                    | "Nothing to commit: the work tree and the index match HEAD." | The panel's                   |
| A diff    | A placeholder                                                  | "No changes in this file."                                   | What failed, with "Try again" |
| History   | A placeholder                                                  | "The branch has no commits yet."                             | What failed, with "Try again" |
| A commit  | A placeholder                                                  | "Choose a commit to see its changes."                        | What failed, with "Try again" |
| Branches  | A placeholder                                                  | No list without entries                                      | What failed, with "Try again" |
| Conflicts | A placeholder                                                  | "No file is conflicted."                                     | What failed, with "Try again" |

## Acceptance

- The commands against a real repository, a project outside a repository and a repository whose git folder may not be read.
- The panel's Vitest tests for each state (`plugins/git/tests/ui/components/GitPanel.test.ts`) and for its reads (`plugins/git/tests/ui/state/git-repository.test.ts`), and the end-to-end check with axe on WebView2.
