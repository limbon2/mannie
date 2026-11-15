import { m } from '#lib/paraglide/messages.js';
import type { FileLinkFailure } from '#lib/state/file-link.svelte.js';

/** The message for a failed local navigation action. */
export function fileLinkFailureMessage(failure: FileLinkFailure): string {
  switch (failure.code) {
    case 'invalid_path':
      return m.markdown_file_invalid();
    case 'not_local':
      return m.markdown_file_not_local();
    case 'no_project':
      return m.markdown_file_no_project();
    case 'conversation_missing':
      return m.markdown_file_conversation_missing();
    case 'not_found':
      return m.markdown_file_not_found();
    case 'unsupported_kind':
      return m.markdown_file_unsupported_kind();
    case 'unavailable':
      return m.markdown_file_unavailable();
    case 'unsupported_target':
      return m.markdown_file_unsupported_target();
    case 'open_failed':
      return m.markdown_file_open_failed();
    case 'ipc':
      return m.shell_error_ipc_failed();
  }
}
