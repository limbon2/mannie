# Changelog of the git plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Added

- The git plugin gives agents tools for the owner's repositories, which run the installed git 2.40 or later without a shell: reading the status, the history, diffs, blame, searches, branches and tags and the conflicts; staging, committing, branching, switching, restoring, resetting and stashing; merging, rebasing, cherry-picking and reverting; fetching, pulling and pushing. The tools show only in a conversation whose project is in a repository.
- Reads run without asking inside the project. A change asks like a file change, and a change that runs the repository's hooks or programs that its configuration names, discards work, rewrites commits that a remote already has or connects to a remote asks first and says so. Grants and saved rules can allow these, and full access runs everything without asking. Uncommitted work that a change discards is kept in a stash entry first. Only fetches, pulls and pushes connect to a remote, so in a partial clone, a read or a change that needs an object that the clone lacks fails instead of fetching it.
- A merge, a rebase, a cherry-pick or a revert that stops on conflicts says where it stopped and which files conflict, with each conflict's current, base and incoming lines named by their commits. `git_resolve` resolves a conflict by taking a side, both sides or new content, and `git_operation` continues, skips or aborts. The operation stays git's own, so the owner can finish it in a terminal.
- `git_edit_history` rewords, squashes, fixes up, drops and reorders a branch's commits in one step, which changes nothing when a step conflicts.
- Each call of a git tool shows a card in the transcript: the changed files with their diffs, the commits, the branches and tags, the conflicts, what a fetch moved or a push updated, and how an operation ended.
- "@" in the message box mentions the uncommitted changes, a commit or a branch, which the model reads as a diff, a commit's message and patch, or a branch's commits and changed files.
- The Git button in a conversation's toolbar names the project's branch and opens the Git side panel: the branch with its upstream, the operation in progress, the changed files with their diffs, the history with each commit's changes, the branches and tags, and the conflicts. The panel reads the repository again after each change of the plugin's tools, on Refresh and every 15 seconds.
