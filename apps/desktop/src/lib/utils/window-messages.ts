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

/** What quitting would end, as a question about quitting counts it. */
export interface QuitCounts {
  readonly activeRuns: number;
  readonly backgroundCommands: number;
  readonly terminals: number;
}

/** The title of the warning before quitting, which names the first of what quitting would end. */
export function quitWarningTitle(counts: QuitCounts): string {
  if (counts.activeRuns > 0) {
    return m.window_quit_warning_title();
  }
  if (counts.backgroundCommands > 0) {
    return m.window_quit_warning_title_commands();
  }
  return m.window_quit_warning_title_terminals();
}

/** The tray's labels in the window's language. */
export function trayLabels(): TrayLabels {
  return { open: m.window_tray_open(), quit: m.window_tray_quit(), tooltip: m.window_tray_tooltip() };
}
