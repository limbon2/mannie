import type { ExportError } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import type { Failure } from '#lib/types/calls.js';

/** Actionable text for stable export failures. */
export function exportFailureMessage(failure: Failure<ExportError>): string {
  if (failure.kind === 'ipc') {
    return m.export_error_ipc();
  }
  switch (failure.error.code) {
    case 'conversation_not_found':
      return m.export_error_conversation_not_found();
    case 'storage_failed':
      return m.export_error_storage_failed();
    case 'store_unavailable':
      return m.export_error_store_unavailable();
    case 'dialog_failed':
      return m.export_error_dialog_failed();
    case 'write_failed':
      return m.export_error_write_failed();
    case 'export_active':
      return m.export_error_export_active();
    case 'unsupported':
      return m.export_error_unsupported();
    case 'host_unavailable':
      return m.export_error_host_unavailable();
    case 'plugin_off':
      return m.export_error_plugin_off();
    case 'attachment_missing':
      return m.export_error_attachment_missing();
    case 'invalid_record':
      return m.export_error_invalid_record();
  }
}
