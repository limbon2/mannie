// Messages of the notifications and tray badge feature (docs/features/
// notifications.md; ADR-0019; ADR-0074): error codes and the localized toast
// titles and attention tooltip sent to the core from the catalog.
import type { NotificationError, NotificationLabels } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import type { Failure } from '#lib/types/calls.js';

/** What to tell the owner about a failed notifications command. */
export function notificationFailureMessage(failure: Failure<NotificationError>): string {
  if (failure.kind === 'ipc') {
    return m.shell_error_ipc_failed();
  }
  switch (failure.error.code) {
    case 'invalid_label': {
      return m.notifications_error_invalid_label();
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

/** The localized notification titles and badged tray tooltip in the window's language. */
export function notificationLabels(): NotificationLabels {
  return {
    approvalTitle: m.notifications_toast_approval_title(),
    completedTitle: m.notifications_toast_completed_title(),
    failedTitle: m.notifications_toast_failed_title(),
    questionTitle: m.notifications_toast_question_title(),
    trayAttentionTooltip: m.notifications_tray_attention_tooltip(),
  };
}
