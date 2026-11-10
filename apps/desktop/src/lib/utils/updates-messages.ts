// Messages of the updates feature (ADR-0019; docs/features/updates.md): the
// errors of its commands, what the owner reads about a check or a download
// that failed, and the times and amounts that the section shows.
import type { UpdateFailureCode } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import type { RestartCounts, UpdateAction, UpdatesFailure } from '#lib/types/updates.js';

/** What the owner reads about a check or a download that failed. */
export interface UpdateFailureText {
  readonly description: string;
  /** Whether the owner can only install the release by hand, from the releases page. */
  readonly offersReleasesPage: boolean;
  readonly title: string;
}

const BYTES_IN_MEGABYTE = 1_000_000;

const times = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' });
const shares = new Intl.NumberFormat(undefined, { maximumFractionDigits: 0, style: 'percent' });
const megabytes = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1, style: 'unit', unit: 'megabyte' });

/** What to tell the owner about a failed updates command. */
export function updatesFailureMessage(failure: UpdatesFailure): string {
  if (failure.kind === 'ipc') {
    return m.shell_error_ipc_failed();
  }
  switch (failure.error.code) {
    case 'browser_unavailable': {
      return m.updates_error_browser_unavailable();
    }
    case 'not_available_in_this_build': {
      return m.updates_error_not_available_in_this_build();
    }
    case 'storage_failed': {
      return m.updates_error_storage_failed();
    }
    case 'store_unavailable': {
      return m.updates_error_store_unavailable();
    }
    case 'version_not_ready': {
      return m.updates_error_version_not_ready();
    }
    case 'written_by_newer_version': {
      return m.shell_error_written_by_newer_version();
    }
  }
}

/** The title of the message about an action of the owner that failed. */
export function actionFailureTitle(action: UpdateAction): string {
  switch (action) {
    case 'check': {
      return m.updates_check_failed_title();
    }
    case 'page': {
      return m.updates_page_failed_title();
    }
    case 'restart': {
      return m.updates_restart_failed_title();
    }
  }
}

/** What to tell the owner about a check or a download that failed, with what they can do. */
export function updateFailureText(code: UpdateFailureCode): UpdateFailureText {
  switch (code) {
    case 'download_failed': {
      return {
        description: m.updates_failed_download(),
        offersReleasesPage: false,
        title: m.updates_failed_download_title(),
      };
    }
    case 'no_package_for_this_installation': {
      return {
        description: m.updates_failed_no_package_for_this_installation(),
        offersReleasesPage: true,
        title: m.updates_failed_no_release_title(),
      };
    }
    case 'no_update_information': {
      return {
        description: m.updates_failed_no_update_information(),
        offersReleasesPage: true,
        title: m.updates_failed_no_release_title(),
      };
    }
    case 'signature_invalid': {
      return {
        description: m.updates_failed_signature(),
        offersReleasesPage: false,
        title: m.updates_failed_signature_title(),
      };
    }
    case 'unreachable': {
      return {
        description: m.updates_failed_unreachable(),
        offersReleasesPage: false,
        title: m.updates_failed_unreachable_title(),
      };
    }
  }
}

/** The title of the question before a restart, which names the first of what the restart would end. */
export function restartQuestionTitle(counts: RestartCounts): string {
  if (counts.activeRuns > 0) {
    return m.updates_restart_question_title();
  }
  if (counts.backgroundCommands > 0) {
    return m.updates_restart_question_title_commands();
  }
  return m.updates_restart_question_title_terminals();
}

/** The time of a check, with its date, in the system's format. */
export function checkTime(millis: number): string {
  return times.format(millis);
}

/**
 * How much of a download arrived, from 0 to 100, or `null` when the server
 * names no size.
 */
export function downloadedPercent(receivedBytes: number, totalBytes: number | null): number | null {
  const size = totalBytes ?? 0;
  if (size <= 0) {
    return null;
  }
  const share = Math.min(receivedBytes / size, 1);
  return Math.floor(share * 100);
}

/** How much of a download arrived, as a share of its size, or in megabytes when the server names no size. */
export function downloadedAmount(receivedBytes: number, totalBytes: number | null): string {
  const percent = downloadedPercent(receivedBytes, totalBytes);
  if (percent === null) {
    return megabytes.format(receivedBytes / BYTES_IN_MEGABYTE);
  }
  return shares.format(percent / 100);
}
