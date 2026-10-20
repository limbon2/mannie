// Messages for the git plugin's codes (ADR-0019): the variants, summaries
// and refusals of its mentions, the failures of its tools' calls, the
// failures of its side panel's commands, and the words of git's states.
import { hostRefusalMessage, ipcFailureMessage } from '@mannie/plugin-sdk/messages';
import type { MentionSummary } from '@mannie/plugin-sdk/slots';

import type { Conflict, GitError } from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import type { GitFailure } from '../types/git.js';

/** The sentences of the codes that the plugin's git runs fail with, in its mentions and its panel. */
const FAILURES: Readonly<Partial<Record<string, () => string>>> = {
  bare_repository: () => m.git_error_bare_repository(),
  dubious_ownership: () => m.git_error_dubious_ownership(),
  git_failed: () => m.git_error_git_failed(),
  git_missing: () => m.git_error_git_missing(),
  git_too_old: () => m.git_error_git_too_old(),
  invalid_key: () => m.git_mention_invalid_key(),
  locked: () => m.git_error_locked(),
  not_a_repository: () => m.git_error_not_a_repository(),
  read_not_allowed: () => m.git_error_read_not_allowed(),
  timed_out: () => m.git_error_timed_out(),
  unknown_revision: () => m.git_mention_unknown_revision(),
};

/** The sentences of the codes that the tools' calls fail with, beyond the shared ones. */
const TOOL_FAILURES: Readonly<Partial<Record<string, () => string>>> = {
  ...FAILURES,
  identity_missing: () => m.git_failure_identity_missing(),
  invalid_argument: () => m.git_failure_invalid_argument(),
  operation_refused: () => m.git_failure_operation_refused(),
  repository_changed: () => m.git_failure_repository_changed(),
  unknown_revision: () => m.git_failure_unknown_revision(),
};

/** The sentences of the panel's errors, which the type checks hold to the plugin's error type. */
const PANEL_ERRORS: Record<GitError['code'], () => string> = {
  bare_repository: () => m.git_error_bare_repository(),
  dubious_ownership: () => m.git_error_dubious_ownership(),
  git_failed: () => m.git_error_git_failed(),
  git_missing: () => m.git_error_git_missing(),
  git_too_old: () => m.git_error_git_too_old(),
  locked: () => m.git_error_locked(),
  not_a_repository: () => m.git_error_not_a_repository(),
  project_not_found: () => m.git_error_project_not_found(),
  read_not_allowed: () => m.git_error_read_not_allowed(),
  timed_out: () => m.git_error_timed_out(),
  unknown_commit: () => m.git_error_unknown_commit(),
};

/** The sentence of a refusal or a failure of a git mention, or `null` for a code it does not know. */
export function gitMentionRefusal(code: string): string | null {
  const message = FAILURES[code];
  return message === undefined ? null : message();
}

/** The sentence of a failure of a git tool's call, or `null` for a code it does not know. */
export function gitToolFailure(code: string): string | null {
  const message = TOOL_FAILURES[code];
  return message === undefined ? null : message();
}

/** What to tell the owner about a command of the panel that failed, and what they can do. */
export function gitFailureMessage(failure: GitFailure): string {
  if (failure.kind === 'ipc') {
    return ipcFailureMessage();
  }
  const { error } = failure;
  if (error.origin === 'plugin') {
    return PANEL_ERRORS[error.code]();
  }
  return hostRefusalMessage(error, m.git_plugin_name());
}

/** The number that a summary holds under `name`, 0 when it holds none. */
function count(summary: MentionSummary, name: string): number {
  return summary.values?.[name] ?? 0;
}

/** What the model got of a sent git mention. */
export function gitMentionSummary(summary: MentionSummary): string | null {
  switch (summary.code) {
    case 'branch': {
      return m.git_mention_summary_branch({ count: count(summary, 'commits') });
    }
    case 'changes': {
      return m.git_mention_summary_changes({ count: count(summary, 'files') });
    }
    case 'commit': {
      return m.git_mention_summary_commit({ count: count(summary, 'files') });
    }
    case 'not_read': {
      return m.git_mention_summary_not_read();
    }
    default: {
      return null;
    }
  }
}

/** What an entry of the list is, for screen readers. */
export function gitMentionVariantName(variant: string): string | null {
  switch (variant) {
    case 'branch': {
      return m.git_mention_branch();
    }
    case 'changes': {
      return m.git_mention_changes();
    }
    case 'commit': {
      return m.git_mention_commit();
    }
    default: {
      return null;
    }
  }
}

/** What a status letter, such as `M`, says a file's change is. */
export function changeWords(letter: string): string {
  switch (letter) {
    case 'A': {
      return m.git_change_added();
    }
    case 'C': {
      return m.git_change_copied();
    }
    case 'D': {
      return m.git_change_deleted();
    }
    case 'M': {
      return m.git_change_modified();
    }
    case 'R': {
      return m.git_change_renamed();
    }
    case 'T': {
      return m.git_change_type_changed();
    }
    default: {
      return m.git_change_changed();
    }
  }
}

/** How a conflicted file conflicts, in words. */
export function conflictWords(conflict: Conflict): string {
  switch (conflict) {
    case 'added_current': {
      return m.git_conflict_added_current();
    }
    case 'added_incoming': {
      return m.git_conflict_added_incoming();
    }
    case 'both_added': {
      return m.git_conflict_both_added();
    }
    case 'both_changed': {
      return m.git_conflict_both_changed();
    }
    case 'both_deleted': {
      return m.git_conflict_both_deleted();
    }
    case 'deleted_current': {
      return m.git_conflict_deleted_current();
    }
    case 'deleted_incoming': {
      return m.git_conflict_deleted_incoming();
    }
  }
}
