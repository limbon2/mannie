// The details of the git tools' cards (ADR-0039; plugins/git/docs/
// tools.md), rendered on the server side from the data that the plugin
// returned: a status's files, a log's commits, a diff's files, the
// conflicts with the hunks of a named file, the refs with their tracking,
// a search's matches, what a fetch moved and a push updated, and how an
// operation that can stop ended. A card without data shows nothing.
import type { ToolKindCall, ToolKindDetailsProps } from '@mannie/plugin-sdk/slots';
import type { Component } from 'svelte';
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import ConflictsCard from '../../../../ui/components/cards/ConflictsCard.svelte';
import DiffCard from '../../../../ui/components/cards/DiffCard.svelte';
import GrepCard from '../../../../ui/components/cards/GrepCard.svelte';
import LogCard from '../../../../ui/components/cards/LogCard.svelte';
import OutcomeCard from '../../../../ui/components/cards/OutcomeCard.svelte';
import RefsCard from '../../../../ui/components/cards/RefsCard.svelte';
import StatusCard from '../../../../ui/components/cards/StatusCard.svelte';
import TransferCard from '../../../../ui/components/cards/TransferCard.svelte';

type Fields = Readonly<Record<string, unknown>>;

type Card = Component<ToolKindDetailsProps>;

const COMMIT = {
  author: 'Ada Lovelace',
  oid: 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678',
  parents: ['p1', 'p2'],
  refs: ['HEAD -> main', 'tag: v1.0'],
  short: 'a1b2c3d',
  subject: 'Merge the parser',
};

const REF = {
  commit: 'a1b2c3d4e5',
  current: true,
  date: '2026-10-19',
  kind: 'branch',
  name: 'main',
  short: 'a1b2c3d',
  subject: 'Fix the parser',
  tracking: { ahead: 2, behind: 1, state: 'exists' },
  upstream: 'origin/main',
  worktree: null,
};

/** The body of `card` for a call with this data. */
function shown(card: Card, data: Fields | null, args: Fields = {}): string {
  const call: ToolKindCall = { arguments: args, data, failure: null, status: 'completed' };
  return render(card, { props: { call, conversationId: 'conversation-1' } }).body;
}

/** The body without Svelte's comment markers, for checks of what shows. */
function text(card: Card, data: Fields | null, args: Fields = {}): string {
  return shown(card, data, args).replaceAll(/<!--[^]*?-->/gu, '');
}

describe('the cards of the read tools', () => {
  it('show a status’s files', () => {
    const body = text(StatusCard, {
      entries: [{ kind: 'changed', path: 'src/lib.rs', staged: '.', submodule: false, unstaged: 'M' }],
      entriesTotal: 1,
    });

    expect(body).toContain('modified: src/lib.rs');
  });

  it('show a log’s commits with their refs, merges and authors', () => {
    const body = text(LogCard, { commits: [COMMIT], mode: 'commits' });

    expect(body).toContain('a1b2c3d');
    expect(body).toContain('Merge the parser');
    expect(body).toContain('merge');
    expect(body).toContain('tag: v1.0');
    expect(body).toContain('Ada Lovelace');
  });

  it('show a diff’s files, opening a single file to its patch', () => {
    const hunk = { header: '@@ -1 +1 @@', lines: [{ kind: 'added', new: 1, noNewline: false, old: null, text: 'x' }] };
    const body = text(DiffCard, {
      files: [{ added: 1, original: null, path: 'a.txt', removed: 0 }],
      patches: [{ hunks: [hunk], path: 'a.txt' }],
    });

    expect(body).toContain('a.txt');
    expect(body).toContain('@@ -1 +1 @@');
  });

  it('show the conflicted files, and the hunks of the file that the call named', () => {
    const body = text(ConflictsCard, {
      file: {
        hunks: [{ base: ['base\n'], current: ['ours\n'], firstLine: 3, incoming: [], lastLine: 9, number: 1 }],
        path: 'src/merge.rs',
      },
      files: [{ conflict: 'both_changed', markers: { hunks: 2, state: 'left' }, path: 'src/merge.rs' }],
    });

    expect(body).toContain('both changed');
    expect(body).toContain('2 conflicts left');
    expect(body).toContain('Current');
    expect(body).toContain('ours');
    expect(body).toContain('Base');
    expect(body).toContain('Incoming');
    expect(body).toContain('No lines');
  });

  it('show the refs with how each branch stands against its upstream', () => {
    const body = text(RefsCard, {
      branches: [REF, { ...REF, current: false, name: 'old', tracking: { state: 'gone' }, upstream: 'origin/old' }],
      remoteBranches: [],
      tags: [],
    });

    expect(body).toContain('Branches');
    expect(body).toContain('current');
    expect(body).toContain('2 ahead, 1 behind origin/main');
    expect(body).toContain('origin/old is gone');
    expect(body).not.toContain('Tags');
  });

  it('show a search’s matches with their revisions, paths and lines', () => {
    const body = text(GrepCard, {
      matches: [{ commit: 'abc', line: 12, path: 'src/lib.rs', revision: 'HEAD', text: 'fn parse()' }],
    });

    expect(body).toContain('HEAD:src/lib.rs:12');
    expect(body).toContain('fn parse()');
  });
});

describe('the cards of the change tools', () => {
  it('show what a fetch moved', () => {
    const body = text(TransferCard, {
      moved: [
        { from: null, name: 'origin/feature', to: '1234567890abcdef' },
        { from: '1111111111aaaa', name: 'origin/main', to: '2222222222bbbb' },
        { from: '3333333333', name: 'origin/old', to: null },
      ],
      remote: 'origin',
    });

    expect(body).toContain('new at 1234567890');
    expect(body).toContain('moved from 1111111111 to 2222222222');
    expect(body).toContain('removed');
  });

  it('say that a fetch moved nothing, and show what a push updated', () => {
    const fetched = text(TransferCard, { moved: [], remote: 'origin' });
    const pushed = text(TransferCard, {
      branch: 'main',
      pushed: [{ destination: 'refs/heads/main', flag: ' ', summary: 'a1b2c3d..e4f5a6b' }],
      remote: 'origin',
    });

    expect(fetched).toContain('No remote-tracking branch moved.');
    expect(pushed).toContain('refs/heads/main');
    expect(pushed).toContain('a1b2c3d..e4f5a6b');
  });

  it('show the commits of an operation that completed, and the files of one that stopped', () => {
    const label = { oid: COMMIT.oid, short: 'a1b2c3d', subject: 'Apply the fix' };
    const completed = text(OutcomeCard, { commits: [label], head: label, operation: 'merge', outcome: 'completed' });
    const stopped = text(OutcomeCard, {
      files: [{ conflict: 'deleted_incoming', markers: null, path: 'src/old.rs' }],
      operation: 'rebase',
      outcome: 'stopped',
      stopped: null,
    });
    const pulled = text(TransferCard, { commits: [label], head: label, operation: 'pull', outcome: 'completed' });

    expect(completed).toContain('Apply the fix');
    expect(stopped).toContain('src/old.rs');
    expect(stopped).toContain('deleted on the incoming side');
    expect(pulled).toContain('Apply the fix');
  });

  it('show nothing without data', () => {
    for (const card of [StatusCard, LogCard, DiffCard, ConflictsCard, GrepCard, TransferCard, OutcomeCard]) {
      expect(text(card, null).trim()).toBe('');
    }
  });
});
