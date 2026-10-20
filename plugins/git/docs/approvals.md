# Git approvals

|              |                                                                                                                                                                                                                                                  |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Building                                                                                                                                                                                                                                         |
| Plugin       | `git`                                                                                                                                                                                                                                            |
| Milestone    | M9                                                                                                                                                                                                                                               |
| Last updated | 2026-10-20                                                                                                                                                                                                                                       |
| Part of      | [Git](README.md)                                                                                                                                                                                                                                 |
| Related      | [ADR-0038](../../../docs/adr/0038-let-plugins-run-installed-programs-in-the-owners-projects.md), [Operations and approvals](../../../docs/features/system-tools/operations-and-approvals.md), [Security](../../../docs/architecture/security.md) |

## Intent

Ask the owner where the owner decided that git should ask, and nowhere else ([D37](../../../docs/roadmap/milestones.md#5-decisions)).

## Behavior

| A call that                                                                                       | Accesses                                                                                 | Without full access                                                                              |
| ------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| Reads                                                                                             | Read of the repository's top, and of its git folder where it lies outside                | Runs inside the project; outside it asks, and "Allow for this project" covers the later calls    |
| Changes the repository in a way that git can take back                                            | Write of the repository's top, with its state                                            | Follows the setting for changes in the workspace, as `edit_file` does                            |
| Runs hooks, or programs that the repository's own configuration names                             | The write, and a run of git that lists each hook or program with where it comes from     | Asks; a grant or a saved rule can allow it, and keeps allowing it after a hook's content changed |
| Discards uncommitted changes, a stash entry or commits that no other branch or tag holds          | The write, and a run of git that lists what it discards and says that a snapshot is kept | Asks; uncommitted changes are kept first as a stash entry named "Mannie: before …"               |
| Rewrites commits that a remote-tracking branch holds, such as an amend or a rebase of pushed work | The write, and a run of git that names the commits and the branches                      | Asks                                                                                             |
| Fetches, pulls or pushes                                                                          | The write, and a run of git that names the remote's URL without credentials              | Asks; a saved rule applies only to that URL                                                      |

- Full access runs every call without asking.
- The approval shows the git that runs, with its path, the arguments, the folder, and each of the above, in the owner's words.
- A grant for the conversation, the project or every conversation covers the same run of git: the same arguments, folder, hooks by name and source, remote, and what it discards or rewrites by kind. A saved rule covers runs whose arguments start with its words, such as `git commit`, optionally in one folder, and for a remote only at its URL.
- The write of a change follows the setting for changes in the workspace also when a grant or a rule covers its run of git. "Allow for…" grants both, so the same change runs again without asking; a saved rule covers the run only, so a push that a rule covers runs without asking once changes in the workspace do, and asks for its write until then.
- Rules and grants of `run_command` never cover a run of the plugin's git, and the reverse.

### Which hooks a change runs

The plugin asks git which hooks a change will run: `git hook list` for each event on git 2.54 and later, which also names hooks that configuration defines, and the files of the hooks folder before, `<event>.exe` too on Windows. On Linux and the other Unix systems, git runs a file of the hooks folder only when it is executable, so the plugin counts only those there. It counts the hooks of these events, which cover every hook that git 2.40, 2.47 and 2.54 ran in the plugin's tests:

| Change                                                             | Events                                                                                                                                             |
| ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| Stage, unstage, resolve a file                                     | `post-index-change`                                                                                                                                |
| Commit                                                             | `pre-commit`, `prepare-commit-msg`, `commit-msg`, `post-commit`, `post-index-change`, `reference-transaction`, and `post-rewrite` for an amend     |
| Create, rename or delete a branch                                  | `reference-transaction`                                                                                                                            |
| Switch, abort an operation                                         | `post-checkout`, `post-index-change`, `reference-transaction`                                                                                      |
| Restore                                                            | `post-checkout`, `post-index-change`                                                                                                               |
| Reset                                                              | `reference-transaction`, and `post-index-change` unless soft                                                                                       |
| Stash, history edit                                                | `post-index-change`, `reference-transaction`                                                                                                       |
| Merge, rebase, cherry-pick, revert, continue or skip a stopped one | the commit's events, `pre-merge-commit`, `post-merge`, `pre-rebase`, `post-checkout`, `post-rewrite`, `post-index-change`, `reference-transaction` |
| Push                                                               | `pre-push`                                                                                                                                         |

A change that keeps a snapshot first also counts `reference-transaction`, since storing a stash entry moves `refs/stash`. A test installs every client hook as a recorder, runs each change tool, and checks that every hook that ran is among the ones that the change listed. It ran against git 2.54 on Windows; git 2.40 and Linux are still to check.

The programs that the repository's own configuration names count where the change runs them: filters (`filter.<driver>.clean`, `smudge` and `process`) for changes that read or write the work tree's files, merge drivers (`merge.<driver>.driver`) for changes that merge, signing programs (`gpg.program`, `gpg.<format>.program`, `gpg.ssh.defaultKeyCommand`) for changes that commit, and the programs that reach a remote (`core.sshCommand`, `core.askPass`, `core.gitProxy`, credential helpers, and a remote's `uploadpack`, `receivepack` and `vcs`) for fetches and pushes. The owner's global and system configuration is the owner's own, and its programs do not count.

## Design

### Threat notes

- T-44: a repository's hooks and its own configuration's programs run only after the owner allowed the run that lists them; reads blank the filters that the repository's configuration defines and switch off its file system monitor, external diffs and text conversion. A grant or a rule keeps covering a hook after its content changed, which the owner accepted (D37).
- T-45: no shell runs, revisions follow `--end-of-options` and become object names in the preparation, paths follow `--` as literal pathspecs, and a value that git would read as an option is refused.
- T-46: the core resolves git outside the owner's projects and Mannie's folders, `.exe` only on Windows, and the approval shows its path.
- T-47: URLs lose their user information in every result, card, access, log and error, and git's messages are scanned for credentials before anything shows them.
- T-48: accesses name the folders that git reports for the repository, so a git folder outside the project asks like any other folder there.
- T-49: a run that reaches a remote names its effective URL, after `insteadOf`; `GIT_ALLOW_PROTOCOL` allows only `file`, `git`, `http`, `https` and `ssh`, so `ext::` never runs.
- T-50: a change that discards work asks and keeps a snapshot first; a history edit changes nothing when a step conflicts; moving a branch with `reset --keep` refuses to overwrite local changes.
- Logs at `info` name the plugin, the program and how a run ended, never its arguments, which can hold the owner's content.

## Acceptance

- Policy and runtime tests of each row above, with and without full access, and the grants and rules of each (`plugins/git/crates/mannie-git/tests/approvals.rs`).
- The recording hooks of every client hook against git 2.40 and the newest.
- Planted configuration: a file system monitor, a filter, a text conversion, an external diff, a pager and a signing program that write a marker file, which no read writes.
- A remote URL with `user:token@` that appears in no result, card, access, log or table of the database.
