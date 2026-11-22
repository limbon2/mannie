# Changelog of the git plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Added

- Journaled owner actions in the Git side panel (staging, unstaging, per-hunk staging and discarding, committing and amending, branching, tagging, stashing, fetching, pulling, pushing, merging, rebasing, cherry-picking, reverting, and conflict resolution) with server-enforced confirmation for hooks, repository-configured programs, discarded work, published rewrites, and branch or tag deletions ([ADR-0078](../../docs/adr/0078-let-plugins-run-installed-programs-for-owner-actions.md)).
- Topological SVG Commit Graph tab with scope switching (Current branch, All branches, Reflog), search and filter controls (message, author, path, `-S` pickaxe, merges, first-parent), and a responsive split Commit Inspector.
- Compare tab for comparing any two refs or `WORKTREE` in three-dot and two-dot modes with Changed files, Ahead, and Behind views.
- Inspect tab with interactive Blame (age heatmap gutter and commit details popover), File History (`--follow`), and Code Search (`git grep`).
- Unified and side-by-side split diff modes with word-level intra-line change highlights and an Ignore whitespace toggle.
- `BlameCard` transcript details card for `git_blame` tool calls, one-click "Mention in chat" controls from the side panel, live dirty and conflicted counts on the toolbar button, and an "Open Git panel" command palette entry.

### Changed

- Redesigned the Git side panel header with a searchable branch switcher, compact ahead/behind indicators, sync buttons, and an operation banner with Continue, Skip, and Abort controls.
- Replaced flat file lists in Changes with a filterable Tree/List browser featuring compressed single-child directory chains, status badges, and inline `+N -M` line counts.
- Upgraded the Branches tab into a searchable explorer with two-line branch rows, grouped remote branches, newest-first tags, clickable stashes with diff previews, worktrees, and credential-redacted remotes.

### Fixed

- Removed the duplicate repository name heading inside the Git side panel by enabling `hideHeading` on the side panel registration.
- Fixed ref badges in commit rows clipping on both sides by truncating the inner text span.
- Fixed untracked files showing "No changes in this file." by synthesizing an added-lines diff preview with binary and size guards, and expanded untracked directories in the panel overview.
- Named unborn branches explicitly in the panel header instead of hiding the branch name behind "No commits yet".
