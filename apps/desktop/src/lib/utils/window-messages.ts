// Messages of the window and tray feature (ADR-0019): the errors of its
// commands, and the tray's labels, which the core cannot take from a catalog.
import type { TrayLabels, WindowError } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import type { Failure } from '#lib/types/calls.js';

/** What to tell the owner about a failed window command. */
export function windowFailureMessage(failure: Failure<WindowError>): string {
  if (failure.kind === 'ipc') {
    return m.shell_error_ipc_failed();
  }
  switch (failure.error.code) {
    case 'invalid_label': {
      return m.window_error_invalid_label();
    }
    case 'request_not_pending': {
      return m.window_error_request_not_pending();
    }
    case 'storage_failed': {
      return m.shell_error_storage_failed();
    }
    case 'store_unavailable': {
      return m.shell_error_store_unavailable();
    }
    case 'written_by_newer_version': {
      return m.shell_error_written_by_newer_version();
    }
  }
}

/** The tray's labels in the window's language. */
export function trayLabels(): TrayLabels {
  return { open: m.window_tray_open(), quit: m.window_tray_quit(), tooltip: m.window_tray_tooltip() };
}
