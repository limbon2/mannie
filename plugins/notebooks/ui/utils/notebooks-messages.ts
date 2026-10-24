// Messages for the plugin's codes (ADR-0019): the core words a call that got
// no answer and the host's refusals, and the plugin its own errors, the
// refusals of files, the failures of sources, searches and tools, its
// mentions, its start failure and its work in progress.
import type { WorkItem } from '@mannie/plugin-sdk';
import { formatList } from '@mannie/plugin-sdk/format';
import { hostRefusalMessage, ipcFailureMessage } from '@mannie/plugin-sdk/messages';
import type { MentionSummary } from '@mannie/plugin-sdk/slots';

import type { NotebooksError, SourceRefusal } from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import type { NotebooksFailure } from '../types/notebooks.js';

/** The code of the sources that are read or indexed, as the plugin reports its work. */
const INDEXING_SOURCES = 'indexing_sources';

/** The variant and the summary code of a mention that attached what it names, as the core sends them (ADR-0050). */
const ATTACHED_VARIANT = 'attachment';
const ATTACHED_SUMMARY = 'attached';

/** The code of a start that could not read the notebooks. */
const START_STORAGE_FAILED = 'storage_failed';

/** Why the plugin refused a file. */
export function sourceRefusalMessage(reason: SourceRefusal): string {
  switch (reason) {
    case 'duplicate': {
      return m.notebooks_refusal_duplicate();
    }
    case 'unreadable': {
      return m.notebooks_refusal_unreadable();
    }
    case 'unsupported_type': {
      return m.notebooks_refusal_unsupported_type();
    }
  }
}

/** The sentence of one of the plugin's errors. */
function errorMessage(error: NotebooksError): string {
  switch (error.code) {
    case 'dialog_failed': {
      return m.notebooks_error_dialog_failed();
    }
    case 'name_taken': {
      return m.notebooks_error_name_taken();
    }
    case 'notebook_not_found': {
      return m.notebooks_error_notebook_not_found();
    }
    case 'nothing_to_retry': {
      return m.notebooks_error_nothing_to_retry();
    }
    case 'passage_not_found': {
      return m.notebooks_error_passage_not_found();
    }
    case 'profile_not_found': {
      return m.notebooks_error_profile_not_found();
    }
    case 'profiles_unavailable': {
      return m.notebooks_error_profiles_unavailable();
    }
    case 'refused': {
      return sourceRefusalMessage(error.reason);
    }
    case 'source_not_found': {
      return m.notebooks_error_source_not_found();
    }
    case 'storage_failed': {
      return m.notebooks_error_storage_failed();
    }
    case 'upload_not_found':
    case 'upload_part_out_of_order': {
      return m.notebooks_error_upload_broken();
    }
  }
}

/** What to tell the owner about a call of the plugin that failed, and what they can do. */
export function notebooksFailureMessage(failure: NotebooksFailure): string {
  if (failure.kind === 'ipc') {
    return ipcFailureMessage();
  }
  const { error } = failure;
  if (error.origin === 'host') {
    return hostRefusalMessage(error, m.notebooks_plugin_name());
  }
  return errorMessage(error);
}

/** The plugin's code of a failed call, or `null` when the call got no answer or the host refused it. */
export function failureCode(failure: NotebooksFailure): NotebooksError['code'] | null {
  if (failure.kind === 'ipc') {
    return null;
  }
  const { error } = failure;
  return error.origin === 'host' ? null : error.code;
}

/** Why a source failed, or why its indexing waits, by the plugin's code. */
export function sourceFailureMessage(code: string): string {
  switch (code) {
    case 'encrypted': {
      return m.notebooks_failure_encrypted();
    }
    case 'input_too_long': {
      return m.notebooks_failure_input_too_long();
    }
    case 'interrupted': {
      return m.notebooks_failure_interrupted();
    }
    case 'no_embedding_profile': {
      return m.notebooks_failure_no_embedding_profile();
    }
    case 'no_text': {
      return m.notebooks_failure_no_text();
    }
    case 'not_utf8': {
      return m.notebooks_failure_not_utf8();
    }
    case 'server_error': {
      return m.notebooks_failure_server_error();
    }
    case 'server_unreachable': {
      return m.notebooks_failure_server_unreachable();
    }
    case 'storage_failed': {
      return m.notebooks_failure_storage_failed();
    }
    case 'unreadable': {
      return m.notebooks_failure_unreadable();
    }
    default: {
      return m.notebooks_failure_other();
    }
  }
}

/**
 * Why a search of the attached notebooks gave nothing, or why one could not
 * be attached, as a sentence, or `null` for a code that the plugin does not
 * know.
 */
export function contextFailureMessage(code: string): string | null {
  switch (code) {
    case 'input_too_long': {
      return m.notebooks_context_failure_input_too_long();
    }
    case 'no_embedding_profile': {
      return m.notebooks_context_failure_no_embedding_profile();
    }
    case 'notebook_not_found': {
      return m.notebooks_context_failure_notebook_not_found();
    }
    case 'nothing_indexed': {
      return m.notebooks_context_failure_nothing_indexed();
    }
    case 'server_error': {
      return m.notebooks_context_failure_server_error();
    }
    case 'server_unreachable': {
      return m.notebooks_context_failure_server_unreachable();
    }
    case 'storage_failed': {
      return m.notebooks_context_failure_storage_failed();
    }
    default: {
      return null;
    }
  }
}

/** The row of a part with passages, such as "Searched Physics: 5 passages". */
export function contextFoundMessage(labels: readonly string[], count: number): string {
  return m.notebooks_context_found({ count, labels: formatList(labels) });
}

/** Why a notebook could not be attached by "@", or `null` for a code that the plugin does not know (ADR-0050). */
export function notebookMentionRefusal(code: string): string | null {
  return contextFailureMessage(code);
}

/** What a sent mention of a notebook did: it attached the notebook to the chat. */
export function notebookMentionSummary(summary: MentionSummary): string | null {
  return summary.code === ATTACHED_SUMMARY ? m.notebooks_mention_attached() : null;
}

/** What an entry of the "@" list is, for screen readers. */
export function notebookMentionVariantName(variant: string): string | null {
  return variant === ATTACHED_VARIANT ? m.notebooks_mention_notebook() : null;
}

/** The sentence of a failure code of the plugin's tools, or `null` for a code that it does not know. */
export function notebooksToolFailure(code: string): string | null {
  switch (code) {
    case 'no_text': {
      return m.notebooks_tool_failure_no_text();
    }
    case 'page_not_found': {
      return m.notebooks_tool_failure_page_not_found();
    }
    case 'passage_not_found': {
      return m.notebooks_tool_failure_passage_not_found();
    }
    case 'section_not_found': {
      return m.notebooks_tool_failure_section_not_found();
    }
    default: {
      return contextFailureMessage(code);
    }
  }
}

/** What turning the plugin off does to the sources that are read or indexed, or `null` without any. */
export function notebooksOffQuestion(items: readonly WorkItem[]): string | null {
  const count = items.find((item) => item.code === INDEXING_SOURCES)?.count ?? 0;
  return count === 0 ? null : m.notebooks_off_question({ count });
}

/** The sentence of one of the plugin's start failure codes, or `null` for a code that it does not know. */
export function notebooksStartFailure(code: string): string | null {
  return code === START_STORAGE_FAILED ? m.notebooks_start_failed_storage() : null;
}
