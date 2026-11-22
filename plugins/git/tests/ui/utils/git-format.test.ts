// Formatting, ref-badge classification, blame age heat bucket and mention
// entry builder tests (plugins/git/docs/panel.md).
import { describe, expect, it } from 'vitest';

import {
  formatFullDate,
  formatRelativeTime,
  heatLevel,
  mentionEntryForBranch,
  mentionEntryForChanges,
  mentionEntryForCommit,
  parseRefBadge,
} from '../../../ui/utils/git-format.js';

describe('parseRefBadge', () => {
  it('classifies HEAD, detached HEAD, tags, remotes and local branches', () => {
    expect(parseRefBadge('HEAD -> main')).toEqual({
      isHead: true,
      kind: 'head',
      label: 'main',
      raw: 'HEAD -> main',
    });
    expect(parseRefBadge('HEAD')).toEqual({
      isHead: true,
      kind: 'head',
      label: 'HEAD',
      raw: 'HEAD',
    });
    expect(parseRefBadge('tag: v0.8.6')).toEqual({
      isHead: false,
      kind: 'tag',
      label: 'v0.8.6',
      raw: 'tag: v0.8.6',
    });
    expect(parseRefBadge('origin/main')).toEqual({
      isHead: false,
      kind: 'remote',
      label: 'origin/main',
      raw: 'origin/main',
    });
    expect(parseRefBadge('main')).toEqual({
      isHead: false,
      kind: 'branch',
      label: 'main',
      raw: 'main',
    });
  });
});

describe('heatLevel', () => {
  it('buckets commit timestamps from 0 (oldest) to 4 (newest)', () => {
    expect(heatLevel(1000, 1000, 2000)).toBe(0);
    expect(heatLevel(1300, 1000, 2000)).toBe(1);
    expect(heatLevel(1500, 1000, 2000)).toBe(2);
    expect(heatLevel(1700, 1000, 2000)).toBe(3);
    expect(heatLevel(1950, 1000, 2000)).toBe(4);
    // Degenerate range or uncommitted line defaults to middle bucket.
    expect(heatLevel(0, 1000, 2000)).toBe(2);
    expect(heatLevel(1500, 1500, 1500)).toBe(2);
  });
});

describe('date formatting and mention entry builders', () => {
  it('formats ISO timestamps and falls back cleanly on invalid strings', () => {
    const now = Date.parse('2026-11-22T12:00:00Z');
    const relative = formatRelativeTime('2026-11-20T12:00:00Z', now);
    expect(relative.length).toBeGreaterThan(0);
    expect(formatRelativeTime('not-a-date', now)).toBe('not-a-date');
    expect(formatFullDate('not-a-date')).toBe('not-a-date');
    expect(formatFullDate('2026-11-20T12:00:00Z').length).toBeGreaterThan(0);
  });

  it('builds chat mention entries for uncommitted changes, commits and branches', () => {
    expect(mentionEntryForChanges()).toMatchObject({
      expands: false,
      key: 'changes',
      token: '@changes',
      variant: 'changes',
      warning: null,
    });
    expect(mentionEntryForCommit({ oid: 'abcdef123456', short: 'abcdef1', subject: 'Fix parser' })).toEqual({
      expands: false,
      key: 'commit:abcdef123456',
      label: 'abcdef1 Fix parser',
      token: '@abcdef1',
      variant: 'commit',
      warning: null,
    });
    expect(mentionEntryForBranch({ name: 'feat/ui' })).toEqual({
      expands: false,
      key: 'branch:feat/ui',
      label: 'feat/ui',
      token: '@feat/ui',
      variant: 'branch',
      warning: null,
    });
  });
});
