// The data of the git tools' cards (ADR-0039): each reader keeps the items
// that have the fields that a card needs, so data that another version of
// the plugin wrote shows less instead of breaking the card.
import { describe, expect, it } from 'vitest';

import {
  branchOf,
  commitsOf,
  conflictedOf,
  entriesOf,
  filesOf,
  labelsOf,
  loggedCommitsOf,
  matchesOf,
  movedOf,
  namedConflictOf,
  operationOf,
  patchesOf,
  pushedOf,
  refsOf,
} from '../../../ui/utils/git-data.js';

const LABEL = { oid: 'a1b2c3d4e5', short: 'a1b2c3d', subject: 'Fix the parser' };

describe('the readers of the cards’ data', () => {
  it('read a status’s files, branch and operation, keeping only complete items', () => {
    const data = {
      branch: { head: 'main', oid: 'a1b2' },
      entries: [{ kind: 'changed', path: 'a.txt', staged: 'M', unstaged: '.' }, { path: 'no-kind' }, 'text'],
      entriesTotal: 1500,
      operation: { kind: 'rebase', step: null },
    };

    const { entries, total } = entriesOf(data);

    expect(entries.map((entry) => entry.path)).toEqual(['a.txt']);
    expect(total).toBe(1500);
    expect(branchOf(data)?.head).toBe('main');
    expect(operationOf(data)?.kind).toBe('rebase');
    expect(operationOf({ operation: { step: null } })).toBeNull();
    expect(entriesOf({ entries: [] }).total).toBe(0);
  });

  it('read the commits of each mode of a log', () => {
    const commit = { ...LABEL, author: 'Ada', parents: [], refs: [] };

    expect(loggedCommitsOf({ commits: [commit], mode: 'commits' })).toEqual([commit]);
    expect(loggedCommitsOf({ changes: [{ commit, hunks: [] }, { hunks: [] }], mode: 'lines' })).toEqual([commit]);
    expect(loggedCommitsOf({ entries: [{ ...LABEL, selector: 'HEAD@{0}' }], mode: 'reflog' })).toEqual([
      { ...LABEL, selector: 'HEAD@{0}' },
    ]);
    expect(loggedCommitsOf(null)).toEqual([]);
    expect(commitsOf({ commits: [{ oid: 'x' }] })).toEqual([]);
    expect(labelsOf({ commits: [LABEL, { short: 'b' }] })).toEqual([LABEL]);
  });

  it('read the files and patches of a diff', () => {
    const data = {
      files: [{ added: 1, original: null, path: 'a.txt', removed: 0 }, { added: 1 }],
      patches: [{ hunks: [], path: 'a.txt' }, { path: 'b.txt' }],
    };

    expect(filesOf(data).map((file) => file.path)).toEqual(['a.txt']);
    expect(patchesOf(data).map((patch) => patch.path)).toEqual(['a.txt']);
  });

  it('read the conflicted files and the hunks of the file that a call named', () => {
    const hunk = { base: null, current: ['a\n'], firstLine: 1, incoming: ['b\n'], lastLine: 5, number: 1 };
    const data = {
      file: { hunks: [hunk, { number: 2 }], path: 'a.txt' },
      files: [{ conflict: 'both_changed', markers: null, path: 'a.txt' }, { path: 'b.txt' }],
    };

    expect(conflictedOf(data).map((file) => file.path)).toEqual(['a.txt']);
    expect(namedConflictOf(data)).toEqual({ hunks: [hunk], path: 'a.txt' });
    expect(namedConflictOf({ files: [] })).toBeNull();
  });

  it('read refs, matches, moved branches and pushed refs', () => {
    expect(refsOf({ tags: [{ name: 'v1.0', short: 'a1b2' }, { name: 'v2.0' }] }, 'tags')).toHaveLength(1);
    expect(refsOf({ tags: [] }, 'branches')).toEqual([]);
    expect(
      matchesOf({ matches: [{ line: 3, path: 'a.rs', revision: null, text: 'fn parse' }, { path: 'b' }] }),
    ).toEqual([{ line: 3, path: 'a.rs', revision: null, text: 'fn parse' }]);
    expect(movedOf({ moved: [{ from: null, name: 'origin/main', to: 'a1b2' }, {}] })).toEqual([
      { from: null, name: 'origin/main', to: 'a1b2' },
    ]);
    expect(
      pushedOf({ pushed: [{ destination: 'refs/heads/main', flag: ' ', summary: 'a..b' }, { flag: '!' }] }),
    ).toEqual([{ destination: 'refs/heads/main', flag: ' ', summary: 'a..b' }]);
  });
});
