// Messages for the codes the core reports (ADR-0007, ADR-0019): the UI shows
// the catalog's message for a code and never the core's English description.
import type { StartupProblem } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import type { SettingsFailure } from '#lib/types/interface-settings.js';

/** What to tell the owner about a failed settings request. */
export function settingsFailureMessage(failure: SettingsFailure): string {
  if (failure.kind === 'ipc') {
    return m.shell_error_ipc_failed();
  }
  const { error } = failure;
  switch (error.code) {
    case 'invalid_locale': {
      return m.shell_error_invalid_locale({ tag: error.tag });
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

/** What to tell the owner about why the core did not start. */
export function startupProblemMessage(problem: StartupProblem): string {
  const numbers = new Intl.NumberFormat(undefined, { useGrouping: false });
  switch (problem.code) {
    case 'data_directory_unavailable': {
      return m.shell_startup_data_directory_unavailable();
    }
    case 'database_too_new': {
      return m.shell_startup_database_too_new({
        databaseVersion: numbers.format(problem.databaseVersion),
        path: problem.path,
        supportedVersion: numbers.format(problem.supportedVersion),
      });
    }
    case 'database_unavailable': {
      return m.shell_startup_database_unavailable({ path: problem.path });
    }
    case 'migration_failed': {
      return m.shell_startup_migration_failed({
        backupDirectory: problem.backupDirectory,
        path: problem.path,
        version: numbers.format(problem.version),
      });
    }
  }
}
