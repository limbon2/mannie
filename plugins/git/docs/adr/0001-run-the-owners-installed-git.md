# ADR-0001: Run the owner's installed git

|          |                                                                                                                                                                                                                                                                        |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D37)                                                                                                                                                                                                                                                         |
| Date     | 2026-10-20                                                                                                                                                                                                                                                             |
| Deciders | Owner                                                                                                                                                                                                                                                                  |
| Related  | [Git](../README.md), [ADR-0038](../../../../docs/adr/0038-let-plugins-run-installed-programs-in-the-owners-projects.md), [ADR-0017](../../../../docs/adr/0017-license-and-dependency-policy.md), [Technology stack](../../../../docs/architecture/technology-stack.md) |

## Context

- The plugin Git gives agents tools for the owner's repositories: reads, local changes, merges, rebases, cherry-picks and reverts that can stop at conflicts, history edits and remotes ([D37](../../../../docs/roadmap/milestones.md#5-decisions)). The owner's terminal and IDE work on the same repositories, so what the tools leave behind must be what git itself leaves.
- Mannie's dependency policy admits permissive licenses and MPL-2.0, and keeps GPL and AGPL code out of its binaries ([ADR-0017](../../../../docs/adr/0017-license-and-dependency-policy.md) d4). ring is its one crypto provider.
- Facts verified on 2026-10-19 and 2026-10-20:
  - libgit2, through the `git2` crate, is GPL-2.0 with a linking exception, and its HTTPS and SSH transports pull in OpenSSL and libssh2.
  - gix is MIT or Apache-2.0. Its crate status lists rebase, cherry-pick, revert and stash as still needing plumbing, hooks as partial and Git LFS as missing.
  - git 2.54 is installed on the owner's workstation. `git merge-tree --write-tree` arrived in 2.38 and its `--merge-base` in 2.40; `git hook list` and hooks defined in configuration arrived in 2.54.

## Decision

1. The plugin runs the owner's installed git, 2.40 or later, through the service `processes` (ADR-0038, decision 7). The core finds it on Mannie's `PATH` outside the owner's projects and Mannie's folders, and the plugin checks its version when it starts and before its tools run, and refuses to run an older one with `git_too_old`.
2. Every run is an argument vector without a shell, after a fixed prefix in the manifest that keeps git from starting programs that a read does not need: `core.fsmonitor=false`, no automatic garbage collection or maintenance, no recursion into submodules, the `zdiff3` style of conflict markers, no signatures in logs, unquoted paths and no color.
3. Reads add `--no-optional-locks`, `--no-ext-diff` and `--no-textconv`, and blank the filters that the repository's own configuration defines, so a read starts no program that the repository names. Filters of the owner's global or system configuration, such as Git LFS's, still run, because git needs them to compare the work tree.
4. The plugin reads git's machine formats only: `status --porcelain=v2 -z`, `log -z` with its own separators, `diff -z --raw` with patches, `blame --porcelain`, `for-each-ref` with formats, `ls-files -z`. It never reads git's files of references or its index, so it works with the reftable format and with object names of 64 characters.
5. The plugin never changes the owner's git configuration and never passes `safe.directory`, which would bypass git's check of a repository's owner. It reports a refused repository, a missing identity and a lock file with what the owner can do.

## Consequences

- Positive: the tools do what the owner's git does, with the owner's configuration, hooks, filters, signing, credential helpers and SSH agent, and leave operations in the state that the owner's terminal and IDE continue. No crate of git and no crypto library joins Mannie's graph.
- Negative: git must be installed, 2.40 or later, and starting it costs tens of milliseconds on Windows. Parsing machine formats is the plugin's own work, with tests against real git.
- Risks: git's output changes between versions; the formats that the plugin reads are the ones that git keeps stable for scripts, and the tests run against 2.40 and the newest git. A repository's own configuration can name programs that a change starts, which the plugin finds and declares (ADR-0038, decision 5).

## Alternatives considered

| Option                         | Why not chosen                                                                                                             |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------------------- |
| libgit2 through `git2`         | GPL-2.0 with a linking exception, which ADR-0017 keeps out, and its transports bring OpenSSL                               |
| gix                            | No sequencer for rebase, cherry-pick and revert, partial hooks, no Git LFS, and behavior that differs from the owner's git |
| gix for reads, git for changes | Two engines that must agree, a large dependency, and reads that could differ from the owner's git                          |
| Shipping git with Mannie       | A GPL program to distribute and keep current, and one that differs from the owner's own                                    |

## Validation

M9: the plugin's tests against the installed git, 2.40 and the newest, on Windows and Linux.
