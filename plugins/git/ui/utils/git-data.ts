// The data of the git tools' cards (ADR-0039): the JSON that the plugin's
// Rust code wrote for a call, which the core checked as an object. The
// types come from the plugin's bindings. Each reader keeps the items that
// have the fields that a card needs, so data that another version of the
// plugin wrote shows less rather than breaking the card.
import type {
  Branch,
  Commit,
  CommitLabel,
  ConflictedFile,
  ConflictHunk,
  Entry,
  FilePatch,
  FileStat,
  OperationView,
  Ref,
} from '../generated/bindings.js';

import { listOf, numberOf, objectOf, textOf } from './git-summaries.js';

type Fields = Readonly<Record<string, unknown>>;

/** The conflicted file that a call of `git_conflicts` named, with its hunks. */
export interface NamedConflict {
  readonly hunks: readonly ConflictHunk[];
  readonly path: string;
}

/** A line that matched a search, as `git_grep`'s data holds it. */
export interface GrepMatch {
  readonly commit: string | null;
  readonly line: number;
  readonly path: string;
  readonly revision: string | null;
  readonly text: string;
}

/** A remote-tracking branch that a fetch moved. */
export interface MovedRef {
  readonly from: string | null;
  readonly name: string;
  readonly to: string | null;
}

/** A ref that a push updated, with git's flag. */
export interface PushedRef {
  readonly destination: string;
  readonly flag: string;
  readonly summary: string;
}

function isFields(value: unknown): value is Fields {
  const isObject = typeof value === 'object' && value !== null;
  return isObject && !Array.isArray(value);
}

/**
 * The items of a list that have every field in `required`. The plugin's
 * Rust code wrote them in the type `T`, which its bindings give.
 */
function itemsOf<T>(fields: Fields | null, key: string, required: readonly string[]): T[] {
  return listOf(fields, key)
    .filter(isFields)
    .filter((item) => required.every((name) => name in item))
    .map((item) => item as T);
}

/** An object field that has every field in `required`, or `null`. */
function itemOf(fields: Fields | null, key: string, required: readonly string[]): Fields | null {
  const item = objectOf(fields, key);
  if (item === null) {
    return null;
  }
  return required.every((name) => name in item) ? item : null;
}

/** The files of a status, and how many there are in all. */
export function entriesOf(data: Fields | null): { readonly entries: readonly Entry[]; readonly total: number } {
  const entries = itemsOf<Entry>(data, 'entries', ['path', 'kind', 'staged', 'unstaged']);
  return { entries, total: numberOf(data, 'entriesTotal') ?? entries.length };
}

/** The branch part of a status. */
export function branchOf(data: Fields | null): Branch | null {
  return itemOf(data, 'branch', []) as Branch | null;
}

/** The operation in progress that the data names. */
export function operationOf(data: Fields | null): OperationView | null {
  return itemOf(data, 'operation', ['kind']) as OperationView | null;
}

/** The commits of a list, with their short hashes and subjects. */
export function commitsOf(data: Fields | null, key = 'commits'): Commit[] {
  return itemsOf<Commit>(data, key, ['oid', 'short', 'subject']);
}

/** The commits that an operation made, as it labels them. */
export function labelsOf(data: Fields | null): CommitLabel[] {
  return itemsOf<CommitLabel>(data, 'commits', ['oid', 'short', 'subject']);
}

/**
 * The commits of a call of `git_log`: the commits that it listed, the
 * commits of a line history, whose changes each name their commit, or the
 * commits that a reflog's entries name.
 */
export function loggedCommitsOf(data: Fields | null): (Commit | CommitLabel)[] {
  switch (data?.mode) {
    case 'lines': {
      const commits = listOf(data, 'changes')
        .filter(isFields)
        .map((change) => change.commit);
      return commitsOf({ commits });
    }
    case 'reflog': {
      return itemsOf<CommitLabel>(data, 'entries', ['oid', 'short', 'subject']);
    }
    default: {
      return commitsOf(data);
    }
  }
}

/** The changed files with their line counts. */
export function filesOf(data: Fields | null): FileStat[] {
  return itemsOf<FileStat>(data, 'files', ['path']);
}

/** The patches of the changed files, by their hunks. */
export function patchesOf(data: Fields | null): FilePatch[] {
  return itemsOf<FilePatch>(data, 'patches', ['path', 'hunks']);
}

/** The conflicted files. */
export function conflictedOf(data: Fields | null): ConflictedFile[] {
  return itemsOf<ConflictedFile>(data, 'files', ['path', 'conflict']);
}

/** The conflicted file that the call named, with its remaining hunks, or `null`. */
export function namedConflictOf(data: Fields | null): NamedConflict | null {
  const file = objectOf(data, 'file');
  const path = textOf(file, 'path');
  if (path === null) {
    return null;
  }
  return { hunks: itemsOf<ConflictHunk>(file, 'hunks', ['number', 'current', 'incoming']), path };
}

/** A list of branches, remote branches or tags. */
export function refsOf(data: Fields | null, key: string): Ref[] {
  return itemsOf<Ref>(data, key, ['name', 'short']);
}

/** The lines that a search found. */
export function matchesOf(data: Fields | null): GrepMatch[] {
  return itemsOf<GrepMatch>(data, 'matches', ['path', 'line', 'text']);
}

/** The remote-tracking branches that a fetch moved. */
export function movedOf(data: Fields | null): MovedRef[] {
  return itemsOf<MovedRef>(data, 'moved', ['name']);
}

/** The refs that a push updated. */
export function pushedOf(data: Fields | null): PushedRef[] {
  return itemsOf<PushedRef>(data, 'pushed', ['destination', 'flag', 'summary']);
}
