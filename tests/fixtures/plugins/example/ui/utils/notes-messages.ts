// Messages for the plugin's codes (ADR-0019): the core words a call that
// got no answer and the host's refusals, and the plugin its own errors.
import { hostRefusalMessage, ipcFailureMessage } from '@mannie/plugin-sdk/messages';

import type { ExampleError } from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import type { NotesFailure } from '../types/notes.js';

/** The plugin's own errors by their codes, which the type checks hold to the plugin's error type. */
const ERRORS: Record<ExampleError['code'], () => string> = {
  storage_failed: () => m.example_error_storage_failed(),
};

/** What to tell the owner about a call of the plugin that failed, and what they can do. */
export function notesFailureMessage(failure: NotesFailure): string {
  if (failure.kind === 'ipc') {
    return ipcFailureMessage();
  }
  const { error } = failure;
  if (error.origin === 'host') {
    return hostRefusalMessage(error, m.example_plugin_name());
  }
  return ERRORS[error.code]();
}
