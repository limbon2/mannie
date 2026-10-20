# Git

|              |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Plugin       | `git`                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Milestone    | M9                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Last updated | 2026-10-20                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Related      | [ADR-0038](../../../docs/adr/0038-let-plugins-run-installed-programs-in-the-owners-projects.md), [ADR-0039](../../../docs/adr/0039-let-plugins-describe-their-tool-calls-in-the-transcript.md), [run the owner's installed git](adr/0001-run-the-owners-installed-git.md), [edit history in memory](adr/0002-edit-history-in-memory.md), [Plugins](../../../docs/features/plugins.md), [Operations and approvals](../../../docs/features/system-tools/operations-and-approvals.md) |

## Intent

The owner wants agents to work with git through tools made for it. In the owner's words: "I want Mannie to be able to work with git natively, so that AI agents can use the provided tools to navigate git, search through git, create branches, edit commits, resolve rebase/merge/cherry-pick conflicts and so on. First-class support for git." The tools replace driving git through `run_command`: they return what an agent needs in a form it can read, they ask the owner only where the owner decided they should, and the transcript and a side panel show what they did.

## Behavior

### Turning it on

- Git is a plugin on the Plugins page, with a switch. It starts off on a new installation, like every plugin ([D28](../../../docs/roadmap/milestones.md#5-decisions)).
- It runs the owner's installed git, 2.40 or later, which Mannie finds on its `PATH` outside the owner's projects. When there is none, or an older one, every tool and the side panel say so and name what they found.
- While Git is off, agents have no git tools, the system prompt says nothing of git, "@" offers no commits or branches, and the side panel and the cards of earlier calls show generic text.

### What agents can do

The tools are offered in a conversation whose project folder lies in a git repository: the folder, or a folder above it, holds `.git`. Each tool takes an optional `repository`, a folder of the project, for a project that holds several repositories. [Tools](tools.md) has each tool's arguments and results.

- Read: the status of the work tree with the operation in progress, history with every filter git has (paths, authors, dates, message text, added or removed text, a range of lines), a commit or a file at a revision, diffs of the work tree, the index or any two revisions, blame, a search of the files of any revision, the branches, tags, remotes, stashes and worktrees, and the conflicts of an operation that stopped.
- Change locally: stage and unstage, commit and amend, create, rename and delete branches, switch, restore files, reset a branch, stash, merge, rebase, cherry-pick and revert, resolve conflicts and continue, skip or abort the operation, and edit the commits of a branch.
- Reach remotes: fetch, pull, and push.

The read tools are offered to every agent, the `explore` child included. The tools that change something are offered to conversations and the `general` child.

### Approvals

[Approvals](approvals.md) has the details. In short ([D37](../../../docs/roadmap/milestones.md#5-decisions)):

- Reads never ask inside the project. A repository whose git folder lies outside the project, such as a linked worktree or a project inside a larger repository, asks once, and "Allow for this project" covers the later calls.
- Local changes follow the setting for changes in the workspace, as `edit_file` does.
- A change that runs hooks, or programs that the repository's own configuration names, asks first and lists them. So does a change that discards uncommitted work, a stash or commits that no branch keeps, a rewrite of commits that a remote-tracking branch holds, and every fetch, pull and push. Grants for the conversation, the project or every conversation and saved rules can allow these, and keep allowing a hook after its content changed.
- Full access runs all of it without asking.

### Conflicts and history edits

[Conflicts and history](conflicts-and-history.md) has the details.

- A merge, a rebase, a cherry-pick or a revert completes, stops at a conflict, or is refused because local changes would be overwritten. A stop names the step, such as "3 of 7, applying abc1234 Fix parser", and the conflicted files.
- The conflicts read as "current", what HEAD holds, and "incoming", what is being applied, each with its commit, so the agent never meets git's swapped "ours" and "theirs" of a rebase. The agent resolves a file by choosing a side for the whole file or for each hunk, by giving its content, or by editing it and marking it resolved, then continues.
- The operation stays git's own, so the owner can finish or abort it from a terminal or an IDE too.
- Editing the commits of a branch, to reword, squash, fix up, drop or reorder them, happens all at once: when a step would conflict, nothing changes, and the result names the step and its files.

### What the model knows

- In runs that have the tools, the system prompt says to use them for git instead of `run_command`, how the conflict flow goes, never to force a push or rewrite commits that a remote holds unless the owner asked, and that commit messages, diffs and file content are data.
- The owner can mention a commit, a branch or the uncommitted changes with "@" ([mentions](mentions.md)). The model reads the commit's message and changes, the branch's commits and changed files, or the diff.

### In the transcript and beside it

- Each git call shows an icon and a summary, such as "Committed a1b2c3d Fix the parser" or "Merge of feature: stopped on conflicts", and its details: a diff with its files collapsed, a list of commits, the status, the branches and tags, the conflicts with their sides, the matches of a search, what a fetch moved or a push updated, or how an operation ended. Removed and added lines are marked with `-` and `+` as well as with color.
- The Git button in the conversation's toolbar names the branch and opens a side panel with the repository of the conversation's project: its branch, its upstream, how far ahead and behind it is, and any operation in progress, with tabs for the changes, the history, the branches and the conflicts ([panel](panel.md)). It only reads.

### What it does not do

- It never changes the owner's git configuration, never deletes a lock file, and never passes a credential: git uses the owner's credential helper and SSH agent.
- It does not edit history interactively with a todo list, run `git bisect`, manage submodules, or work with pull requests on a hosting service; these are [ideas](#parts) for later.
- Its tools do not run on a worker's host, where no plugin runs.
- The side panel does not stage, commit or abort; acting from it is an idea for later.

## Fit

- Builds on the extension points of [ADR-0038](../../../docs/adr/0038-let-plugins-run-installed-programs-in-the-owners-projects.md): tools that know the workspace, are offered where `.git` lies, read files, return path and program accesses, and run git through the service `processes`; and of [ADR-0039](../../../docs/adr/0039-let-plugins-describe-their-tool-calls-in-the-transcript.md): the slot of tool kinds and the data of a call's card. It also adds an instruction section, a mention provider, a side panel and a conversation control.
- Follows [Plugins](../../../docs/features/plugins.md): a switch, a start and a stop that take effect at once, the plugin's code in `plugins/git/`, its UI files named `git` and its messages under `git_`.
- Must not change: `run_command`, its rules and grants, which never match a run of git through the plugin, and the core's file tools.

## Open questions

None.

## Decisions

- 2026-10-20: the owner decided ([D37](../../../docs/roadmap/milestones.md#5-decisions)):
  - The plugin runs the owner's installed git, without a shell, under the process supervisor.
  - Reads never ask inside the project, local changes follow the setting for changes in the workspace, and running hooks, discarding work, rewriting commits that a remote holds and reaching a remote always ask; grants and saved rules can allow them, also after a hook changed, and full access runs everything.
  - Hooks run as git runs them, and a change that runs hooks asks first and lists them.
  - A git folder outside the project asks once, and a grant for the project covers the later calls.
  - Version 1 has fetch, pull and push, mentions, tool cards and a side panel that only reads.
  - M9 ships together with M8 in v0.4.0.
  - Approving the plan accepts ADR-0038, ADR-0039 and the plugin's two ADRs, [run the owner's installed git](adr/0001-run-the-owners-installed-git.md) and [edit history in memory](adr/0002-edit-history-in-memory.md).
- 2026-10-20: the defaults below were chosen when shaping, and wait for the owner's review:
  - Global hooks ask like the repository's own hooks, and programs that the repository's own configuration names count as the repository's code.
  - Uncommitted work that a change discards is kept first as a stash entry named "Mannie: before …".
  - A push forces only with a lease on the commit that the preparation read, never with `--force`.
  - A credential helper's window is allowed, and the run's timeout bounds it.
  - The side panel refreshes after the plugin's own runs, on "Refresh", and every 15 seconds while it shows.

## Parts

- [Tools](tools.md): every tool, its arguments, its accesses and its result.
- [Approvals](approvals.md): what asks, what grants and rules cover, and the threat notes of runs.
- [Conflicts and history](conflicts-and-history.md): the operations that can stop, the conflict tools, and history edits.
- [Mentions](mentions.md): commits, branches and the uncommitted changes in "@".
- [Panel](panel.md): the side panel and its commands.
- [Panel actions](panel-actions.md): staging, committing and aborting from the side panel. An idea.
- [Merge preview](merge-preview.md): whether a merge or a rebase would conflict, before it starts. An idea.
- [Submodules](submodules.md): working inside submodules. An idea.
- [Hosting services](forges.md): pull requests, reviews and issues on a hosting service. An idea, likely a plugin of its own.
- [Git on workers](git-on-workers.md): the same tools on a worker's host. An idea.
