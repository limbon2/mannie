// Locale-aware date/time formatting (`Intl`), ref badge parsing, blame age
// heatmap bucketing, and `@mannie/plugin-sdk/message-box` mention builders.
import type { MentionOfEntry } from '@mannie/plugin-sdk/message-box';

import type { Commit, CommitDetails, Ref } from '../generated/bindings.js';
import { m } from '../generated/messages.js';

/** A Git mention entry passed to `@mannie/plugin-sdk/message-box`. */
export type GitMentionEntry = MentionOfEntry['entry'];

/** Parsed kind and display label of a git `%D` ref decoration. */
export interface ParsedRefBadge {
  readonly isHead: boolean;
  readonly kind: 'branch' | 'head' | 'remote' | 'tag';
  readonly label: string;
  readonly raw: string;
}

/** Heat bucket from 0 (oldest) to 4 (newest) for blame gutter age stripes. */
export type HeatLevel = 0 | 1 | 2 | 3 | 4;

interface RelativeUnit {
  readonly seconds: number;
  readonly unit: Intl.RelativeTimeFormatUnit;
}

const RELATIVE_UNITS: readonly RelativeUnit[] = [
  { seconds: 31_536_000, unit: 'year' },
  { seconds: 2_592_000, unit: 'month' },
  { seconds: 604_800, unit: 'week' },
  { seconds: 86_400, unit: 'day' },
  { seconds: 3600, unit: 'hour' },
  { seconds: 60, unit: 'minute' },
];

/** Formats an ISO-8601 timestamp as a compact relative time using `Intl.RelativeTimeFormat`. */
export function formatRelativeTime(isoDate: string, nowMs: number = Date.now()): string {
  const parsed = Date.parse(isoDate);
  if (Number.isNaN(parsed)) {
    return isoDate.slice(0, 10);
  }
  const diffSeconds = Math.round((parsed - nowMs) / 1000);
  const absSeconds = Math.abs(diffSeconds);
  const formatter = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto', style: 'narrow' });

  for (const { seconds, unit } of RELATIVE_UNITS) {
    if (absSeconds >= seconds) {
      return formatter.format(Math.trunc(diffSeconds / seconds), unit);
    }
  }
  return formatter.format(diffSeconds, 'second');
}

/** Formats an ISO-8601 timestamp as a localized full date and time. */
export function formatFullDate(isoDate: string): string {
  const parsed = Date.parse(isoDate);
  if (Number.isNaN(parsed)) {
    return isoDate;
  }
  return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(parsed);
}

/** Classifies a raw `%D` decoration string such as `HEAD -> main`, `tag: v1.0`, or `origin/main`. */
export function parseRefBadge(raw: string): ParsedRefBadge {
  const trimmed = raw.trim();
  if (trimmed.startsWith('HEAD -> ')) {
    return {
      isHead: true,
      kind: 'head',
      label: trimmed.slice('HEAD -> '.length),
      raw: trimmed,
    };
  }
  if (trimmed === 'HEAD') {
    return {
      isHead: true,
      kind: 'head',
      label: 'HEAD',
      raw: trimmed,
    };
  }
  if (trimmed.startsWith('tag: ')) {
    return {
      isHead: false,
      kind: 'tag',
      label: trimmed.slice('tag: '.length),
      raw: trimmed,
    };
  }
  if (trimmed.includes('/') && !trimmed.startsWith('refs/')) {
    return {
      isHead: false,
      kind: 'remote',
      label: trimmed,
      raw: trimmed,
    };
  }
  return {
    isHead: false,
    kind: 'branch',
    label: trimmed,
    raw: trimmed,
  };
}

/** Maps a commit's timestamp to a 0..4 age heat bucket across `[min, max]`. */
export function heatLevel(timestamp: number, min: number, max: number): HeatLevel {
  const isUncommitted = timestamp <= 0;
  const isFlatRange = max <= min;
  if (isUncommitted || isFlatRange) {
    return 2;
  }
  const ratio = Math.min(1, Math.max(0, (timestamp - min) / (max - min)));
  if (ratio >= 0.8) {
    return 4;
  }
  if (ratio >= 0.6) {
    return 3;
  }
  if (ratio >= 0.4) {
    return 2;
  }
  if (ratio >= 0.2) {
    return 1;
  }
  return 0;
}

/** Builds a mention entry for the repository's uncommitted changes. */
export function mentionEntryForChanges(): GitMentionEntry {
  return {
    expands: false,
    key: 'changes',
    label: m.git_mention_changes(),
    token: '@changes',
    variant: 'changes',
    warning: null,
  };
}

/** Minimal commit metadata needed to build a commit mention entry. */
export type MentionCommitInput = Pick<Commit | CommitDetails, 'oid' | 'short'> & {
  readonly subject?: string;
};

/** Builds a mention entry for a commit. */
export function mentionEntryForCommit(commit: MentionCommitInput): GitMentionEntry {
  const subject = commit.subject?.trim() ?? '';
  return {
    expands: false,
    key: `commit:${commit.oid}`,
    label: subject.length > 0 ? `${commit.short} ${subject}` : commit.short,
    token: `@${commit.short}`,
    variant: 'commit',
    warning: null,
  };
}

/** Builds a mention entry for a branch. */
export function mentionEntryForBranch(branch: Pick<Ref, 'name'>): GitMentionEntry {
  return {
    expands: false,
    key: `branch:${branch.name}`,
    label: branch.name,
    token: `@${branch.name}`,
    variant: 'branch',
    warning: null,
  };
}
