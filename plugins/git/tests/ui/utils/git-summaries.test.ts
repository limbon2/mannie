// The lines of the git tools' cards (plugins/git/docs/tools.md; ADR-0039):
// each tool's line comes from the data that the plugin returned, or from
// the call's arguments while it runs, an operation that can stop says how
// it ended, and a call whose arguments name nothing leaves the core's line.
import type { ToolKindCall } from '@mannie/plugin-sdk/slots';
import { describe, expect, it } from 'vitest';

import { commitText, listOf, numberOf, objectOf, SUMMARIES, textOf } from '../../../ui/utils/git-summaries.js';

type Fields = Readonly<Record<string, unknown>>;

const COMMIT = { message: 'Fix the parser\n\nIt read past the end.', oid: 'a1b2c3d4e5', short: 'a1b2c3d' };

/** A finished call of a tool with these arguments and data. */
function toolCall(fields: { readonly arguments?: Fields; readonly data?: Fields | null }): ToolKindCall {
  return { arguments: fields.arguments ?? {}, data: fields.data ?? null, failure: null, status: 'completed' };
}

/** The line of a call of `tool`. */
function line(tool: string, fields: { readonly arguments?: Fields; readonly data?: Fields | null }): string | null {
  const summary = SUMMARIES[tool];
  if (summary === undefined) {
    throw new Error(`no summary for ${tool}`);
  }
  return summary(toolCall(fields));
}

describe('the readers of a call', () => {
  it('read text, objects, lists and numbers, and nothing of another type', () => {
    const fields = { count: 3, empty: '', list: [1], name: 'main', nested: { a: 1 }, wrong: Number.NaN };

    expect(textOf(fields, 'name')).toBe('main');
    expect(textOf(fields, 'empty')).toBeNull();
    expect(textOf(fields, 'count')).toBeNull();
    expect(objectOf(fields, 'nested')).toEqual({ a: 1 });
    expect(objectOf(fields, 'list')).toBeNull();
    expect(listOf(fields, 'list')).toEqual([1]);
    expect(listOf(null, 'list')).toEqual([]);
    expect(numberOf(fields, 'count')).toBe(3);
    expect(numberOf(fields, 'wrong')).toBeNull();
  });

  it('name a commit by its short hash and subject, or its message’s first line', () => {
    expect(commitText({ short: 'a1b2c3d', subject: 'Add tests' })).toBe('a1b2c3d Add tests');
    expect(commitText(COMMIT)).toBe('a1b2c3d Fix the parser');
    expect(commitText({ short: 'a1b2c3d' })).toBe('a1b2c3d');
    expect(commitText({ subject: 'No hash' })).toBeNull();
  });
});

describe('the lines of the read tools', () => {
  it('say what each read found', () => {
    expect(line('git_status', { data: { branch: { head: 'main' } } })).toBe('Read the status of main');
    expect(line('git_status', { data: { branch: { head: null } } })).toBe('Read the status of a detached HEAD');
    expect(line('git_status', {})).toBeNull();
    expect(line('git_log', { data: { commits: [{}, {}], mode: 'commits' } })).toBe('Listed 2 commits');
    expect(line('git_log', { data: { changes: [{}], mode: 'lines' } })).toBe('Listed 1 commit');
    expect(line('git_log', {})).toBe('Listing commits');
    expect(line('git_show', { data: { commit: COMMIT } })).toBe('Showed a1b2c3d Fix the parser');
    expect(line('git_show', { arguments: { path: 'src/lib.rs', revision: 'v1.0' } })).toBe('Read src/lib.rs at v1.0');
    expect(line('git_show', {})).toBe('Showing HEAD');
    expect(line('git_blame', { arguments: { path: 'src/lib.rs' } })).toBe('Blamed src/lib.rs');
    expect(line('git_grep', { arguments: { pattern: 'parse' }, data: { matches: [{}, {}, {}] } })).toBe(
      'Searched for “parse”: 3 matches',
    );
    expect(line('git_grep', { arguments: { pattern: 'parse' } })).toBe('Searching for “parse”');
    expect(line('git_refs', {})).toBe('Listed branches, tags and remotes');
    expect(line('git_conflicts', { data: { files: [{}] } })).toBe('1 conflicted file');
  });

  it('say which changes a diff compared', () => {
    const data = { files: [{}, {}] };

    expect(line('git_diff', { data })).toBe('Unstaged changes: 2 files');
    expect(line('git_diff', { arguments: { staged: true }, data })).toBe('Staged changes: 2 files');
    expect(line('git_diff', { arguments: { from: 'main' }, data })).toBe('Changes from main: 2 files');
    expect(line('git_diff', { arguments: { from: 'main', to: 'feature' }, data })).toBe(
      'Changes from main to feature: 2 files',
    );
    expect(line('git_diff', {})).toBe('Comparing changes');
  });
});

describe('the lines of the change tools', () => {
  it('name the commit that a commit made, or its subject while it runs', () => {
    expect(line('git_commit', { data: { commit: COMMIT } })).toBe('Committed a1b2c3d Fix the parser');
    expect(line('git_commit', { arguments: { amend: true }, data: { commit: COMMIT } })).toBe(
      'Amended the commit to a1b2c3d Fix the parser',
    );
    expect(line('git_commit', { arguments: { message: 'Fix the parser\n\nBody' } })).toBe('Commit “Fix the parser”');
    expect(line('git_commit', { arguments: { amend: true } })).toBe('Amending the last commit');
    expect(line('git_commit', {})).toBeNull();
  });

  it('say what each local change does', () => {
    expect(line('git_stage', { arguments: { paths: ['a'] } })).toBe('Staged changes');
    expect(line('git_stage', { arguments: { all: true } })).toBe('Staged every change');
    expect(line('git_stage', { arguments: { unstage: true } })).toBe('Unstaged changes');
    expect(line('git_branch', { arguments: { action: 'create', name: 'feature' } })).toBe('Created the branch feature');
    expect(line('git_branch', { arguments: { action: 'rename', name: 'a', new_name: 'b' } })).toBe(
      'Renamed the branch a to b',
    );
    expect(line('git_branch', { arguments: { action: 'set_upstream', name: 'a', upstream: 'origin/a' } })).toBe(
      'Made a track origin/a',
    );
    expect(line('git_branch', { arguments: { action: 'other', name: 'a' } })).toBeNull();
    expect(line('git_branch', { arguments: { action: 'delete' } })).toBeNull();
    expect(line('git_switch', { arguments: { target: 'main' } })).toBe('Switched to main');
    expect(line('git_switch', { arguments: { create: true, target: 'feature' } })).toBe(
      'Created feature and switched to it',
    );
    expect(line('git_switch', { arguments: { detach: true, target: 'v1.0' } })).toBe('Detached HEAD at v1.0');
    expect(line('git_restore', { arguments: { paths: ['a', 'b'] } })).toBe('Restored 2 paths');
    expect(line('git_reset', { arguments: { target: 'HEAD~1' } })).toBe('Reset the branch to HEAD~1');
    expect(line('git_stash', { arguments: { action: 'push' } })).toBe('Stashed the uncommitted changes');
    expect(line('git_stash', { arguments: { action: 'drop' } })).toBe('Dropped a stash entry');
    expect(line('git_stash', {})).toBeNull();
    expect(line('git_resolve', { arguments: { path: 'src/lib.rs' } })).toBe('Resolved src/lib.rs');
    expect(line('git_edit_history', { arguments: { base: 'main' } })).toBe('Rewrote the commits after main');
  });

  it('say how an operation that can stop ended', () => {
    const completed = { outcome: 'completed' };
    const stopped = { outcome: 'stopped' };

    expect(line('git_merge', { arguments: { branch: 'feature' } })).toBe('Merge of feature');
    expect(line('git_merge', { arguments: { branch: 'feature' }, data: completed })).toBe(
      'Merge of feature: completed',
    );
    expect(line('git_rebase', { arguments: { onto: 'main' }, data: stopped })).toBe(
      'Rebase onto main: stopped on conflicts',
    );
    expect(line('git_cherry_pick', { arguments: { commits: ['a', 'b'] }, data: completed })).toBe(
      'Cherry-pick of 2 commits: completed',
    );
    expect(line('git_revert', { arguments: { commits: ['a'] } })).toBe('Revert of 1 commit');
    expect(line('git_stash', { arguments: { action: 'pop' }, data: stopped })).toBe(
      'Popped a stash entry: stopped on conflicts',
    );
    expect(line('git_operation', { arguments: { action: 'continue' }, data: completed })).toBe(
      'Continued the operation: completed',
    );
    expect(line('git_operation', { arguments: { action: 'abort' } })).toBe('Aborted the operation');
    expect(line('git_operation', {})).toBeNull();
  });

  it('name the remote and branch of a fetch, a pull and a push', () => {
    expect(line('git_fetch', { arguments: { remote: 'origin' } })).toBe('Fetched from origin');
    expect(line('git_fetch', {})).toBe('Fetched from the remote');
    expect(line('git_fetch', { arguments: { integrate: 'rebase' }, data: { outcome: 'completed' } })).toBe(
      'Pulled from the remote: completed',
    );
    expect(line('git_push', { data: { branch: 'main', remote: 'origin' } })).toBe('Pushed main to origin');
    expect(line('git_push', { arguments: { branch: 'main' } })).toBe('Pushed the current branch');
  });
});
