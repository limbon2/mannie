// The lines of the git tools' cards (ADR-0039; plugins/git/docs/tools.md):
// what a call did, from the data that the plugin returned for its card, or
// from its arguments while it runs or when it failed. Names, paths, hashes
// and subjects are content and stay as they are; every word around them
// comes from the catalog (ADR-0019).
import type { ToolKindCall } from '@mannie/plugin-sdk/slots';

import { m } from '../generated/messages.js';

type Fields = Readonly<Record<string, unknown>>;

/** A line of a call, or `null` for the core's own. */
type Summary = (call: ToolKindCall) => string | null;

function isFields(value: unknown): value is Fields {
  const isObject = typeof value === 'object' && value !== null;
  return isObject && !Array.isArray(value);
}

/** A field that holds text that is not empty, or `null`. */
export function textOf(fields: Fields | null, key: string): string | null {
  const value = fields?.[key];
  if (typeof value !== 'string') {
    return null;
  }
  return value === '' ? null : value;
}

/** A field that holds an object, or `null`. */
export function objectOf(fields: Fields | null, key: string): Fields | null {
  const value = fields?.[key];
  return isFields(value) ? value : null;
}

/** A field that holds a list, or an empty one. */
export function listOf(fields: Fields | null, key: string): readonly unknown[] {
  const value = fields?.[key];
  return Array.isArray(value) ? value : [];
}

/** A field that holds a number, or `null`. */
export function numberOf(fields: Fields | null, key: string): number | null {
  const value = fields?.[key];
  if (typeof value !== 'number') {
    return null;
  }
  return Number.isFinite(value) ? value : null;
}

/** Whether a field holds `true`. */
function isSet(fields: Fields | null, key: string): boolean {
  return fields?.[key] === true;
}

/** A commit as a line names it: its short hash and its subject. */
export function commitText(commit: Fields | null): string | null {
  const short = textOf(commit, 'short');
  if (short === null) {
    return null;
  }
  const subject = textOf(commit, 'subject') ?? firstLine(textOf(commit, 'message'));
  return subject === null ? short : `${short} ${subject}`;
}

/** The first line of a text, or `null`. */
function firstLine(text: string | null): string | null {
  const line = text?.split('\n')[0]?.trim() ?? '';
  return line === '' ? null : line;
}

/** The line of an operation that can stop, after how it ended. */
function withOutcome(summary: string, call: ToolKindCall): string {
  switch (textOf(call.data, 'outcome')) {
    case 'completed': {
      return m.git_outcome_completed({ summary });
    }
    case 'stopped': {
      return m.git_outcome_stopped({ summary });
    }
    case null:
    default: {
      return summary;
    }
  }
}

function status(call: ToolKindCall): string | null {
  if (call.data === null) {
    return null;
  }
  const branch = textOf(objectOf(call.data, 'branch'), 'head');
  return branch === null ? m.git_summary_status_detached() : m.git_summary_status({ branch });
}

function log(call: ToolKindCall): string {
  if (call.data === null) {
    return m.git_summary_log_pending();
  }
  const listed = ['commits', 'entries', 'changes'].map((key) => listOf(call.data, key).length);
  return m.git_summary_log({ count: Math.max(...listed) });
}

function show(call: ToolKindCall): string {
  const revision = textOf(call.arguments, 'revision') ?? 'HEAD';
  const path = textOf(call.arguments, 'path');
  if (path !== null) {
    return m.git_summary_show_file({ path, revision });
  }
  const commit = commitText(objectOf(call.data, 'commit'));
  return commit === null ? m.git_summary_show_pending({ revision }) : m.git_summary_show_commit({ commit });
}

function diff(call: ToolKindCall): string {
  if (call.data === null) {
    return m.git_summary_diff_pending();
  }
  const files = m.git_files({ count: listOf(call.data, 'files').length });
  const from = textOf(call.arguments, 'from');
  const to = textOf(call.arguments, 'to');
  if (from !== null) {
    return to === null ? m.git_summary_diff_from({ files, from }) : m.git_summary_diff_between({ files, from, to });
  }
  return isSet(call.arguments, 'staged')
    ? m.git_summary_diff_staged({ files })
    : m.git_summary_diff_unstaged({ files });
}

function grep(call: ToolKindCall): string | null {
  const pattern = textOf(call.arguments, 'pattern');
  if (pattern === null) {
    return null;
  }
  if (call.data === null) {
    return m.git_summary_grep_pending({ pattern });
  }
  return m.git_summary_grep({ count: listOf(call.data, 'matches').length, pattern });
}

function conflicts(call: ToolKindCall): string {
  if (call.data === null) {
    return m.git_summary_conflicts_pending();
  }
  return m.git_summary_conflicts({ count: listOf(call.data, 'files').length });
}

function stage(call: ToolKindCall): string {
  if (isSet(call.arguments, 'unstage')) {
    return m.git_summary_unstage();
  }
  return isSet(call.arguments, 'all') ? m.git_summary_stage_all() : m.git_summary_stage();
}

function commit(call: ToolKindCall): string | null {
  const made = commitText(objectOf(call.data, 'commit'));
  const amend = isSet(call.arguments, 'amend');
  if (made !== null) {
    return amend ? m.git_summary_amend({ commit: made }) : m.git_summary_commit({ commit: made });
  }
  if (amend) {
    return m.git_summary_amend_pending();
  }
  const subject = firstLine(textOf(call.arguments, 'message'));
  return subject === null ? null : m.git_summary_commit_pending({ subject });
}

function branch(call: ToolKindCall): string | null {
  const name = textOf(call.arguments, 'name');
  if (name === null) {
    return null;
  }
  switch (textOf(call.arguments, 'action')) {
    case 'create': {
      return m.git_summary_branch_create({ name });
    }
    case 'delete': {
      return m.git_summary_branch_delete({ name });
    }
    case 'rename': {
      return m.git_summary_branch_rename({ name, newName: textOf(call.arguments, 'new_name') ?? '' });
    }
    case 'set_upstream': {
      return m.git_summary_branch_set_upstream({ name, upstream: textOf(call.arguments, 'upstream') ?? '' });
    }
    case 'unset_upstream': {
      return m.git_summary_branch_unset_upstream({ name });
    }
    case null:
    default: {
      return null;
    }
  }
}

function switchTo(call: ToolKindCall): string | null {
  const target = textOf(call.arguments, 'target');
  if (target === null) {
    return null;
  }
  if (isSet(call.arguments, 'create')) {
    return m.git_summary_switch_create({ target });
  }
  return isSet(call.arguments, 'detach') ? m.git_summary_switch_detach({ target }) : m.git_summary_switch({ target });
}

function stash(call: ToolKindCall): string | null {
  switch (textOf(call.arguments, 'action')) {
    case 'apply': {
      return withOutcome(m.git_summary_stash_apply(), call);
    }
    case 'drop': {
      return m.git_summary_stash_drop();
    }
    case 'pop': {
      return withOutcome(m.git_summary_stash_pop(), call);
    }
    case 'push': {
      return m.git_summary_stash_push();
    }
    case null:
    default: {
      return null;
    }
  }
}

function operation(call: ToolKindCall): string | null {
  switch (textOf(call.arguments, 'action')) {
    case 'abort': {
      return m.git_summary_operation_abort();
    }
    case 'continue': {
      return withOutcome(m.git_summary_operation_continue(), call);
    }
    case 'skip': {
      return withOutcome(m.git_summary_operation_skip(), call);
    }
    case null:
    default: {
      return null;
    }
  }
}

function fetch(call: ToolKindCall): string {
  const remote = textOf(call.arguments, 'remote');
  if (textOf(call.arguments, 'integrate') !== null) {
    const summary = remote === null ? m.git_summary_pull_default() : m.git_summary_pull({ remote });
    return withOutcome(summary, call);
  }
  return remote === null ? m.git_summary_fetch_default() : m.git_summary_fetch({ remote });
}

function push(call: ToolKindCall): string {
  const remote = textOf(call.data, 'remote') ?? textOf(call.arguments, 'remote');
  const branch = textOf(call.data, 'branch') ?? textOf(call.arguments, 'branch');
  const isNamed = remote !== null && branch !== null;
  return isNamed ? m.git_summary_push({ branch, remote }) : m.git_summary_push_default();
}

/** The line of each git tool's calls, by the tool's name. */
export const SUMMARIES: Readonly<Partial<Record<string, Summary>>> = {
  git_blame: (call) => {
    const path = textOf(call.arguments, 'path');
    return path === null ? null : m.git_summary_blame({ path });
  },
  git_branch: branch,
  git_cherry_pick: (call) =>
    withOutcome(m.git_summary_cherry_pick({ count: listOf(call.arguments, 'commits').length }), call),
  git_commit: commit,
  git_conflicts: conflicts,
  git_diff: diff,
  git_edit_history: (call) => {
    const base = textOf(call.arguments, 'base');
    return base === null ? null : m.git_summary_edit_history({ base });
  },
  git_fetch: fetch,
  git_grep: grep,
  git_log: log,
  git_merge: (call) => {
    const branchName = textOf(call.arguments, 'branch');
    return branchName === null ? null : withOutcome(m.git_summary_merge({ branch: branchName }), call);
  },
  git_operation: operation,
  git_push: push,
  git_rebase: (call) => {
    const onto = textOf(call.arguments, 'onto');
    return onto === null ? null : withOutcome(m.git_summary_rebase({ onto }), call);
  },
  git_refs: () => m.git_summary_refs(),
  git_reset: (call) => {
    const target = textOf(call.arguments, 'target');
    return target === null ? null : m.git_summary_reset({ target });
  },
  git_resolve: (call) => {
    const path = textOf(call.arguments, 'path');
    return path === null ? null : m.git_summary_resolve({ path });
  },
  git_restore: (call) => m.git_summary_restore({ count: listOf(call.arguments, 'paths').length }),
  git_revert: (call) => withOutcome(m.git_summary_revert({ count: listOf(call.arguments, 'commits').length }), call),
  git_show: show,
  git_stage: stage,
  git_stash: stash,
  git_status: status,
  git_switch: switchTo,
};
