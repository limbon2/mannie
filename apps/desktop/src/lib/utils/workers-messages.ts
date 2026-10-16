// Messages for the codes and states of the workers feature (ADR-0019): a
// worker's status, the commands' errors and the modes of a ceiling. The UI
// shows the catalog's message for a code and never the core's description.
// A worker's name, purpose, facts and folders are content and never pass
// through these functions.
import type { CeilingMode, WorkerStatus } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import type { WorkersFailure } from '#lib/types/workers.js';

/** A worker's status, as its row and the settings name it. */
export function workerStatusLabel(status: WorkerStatus): string {
  switch (status) {
    case 'connected': {
      return m.workers_status_connected();
    }
    case 'connecting': {
      return m.workers_status_connecting();
    }
    case 'incompatible': {
      return m.workers_status_incompatible();
    }
    case 'key_changed': {
      return m.workers_status_key_changed();
    }
    case 'offline': {
      return m.workers_status_offline();
    }
    case 'unpaired': {
      return m.workers_status_unpaired();
    }
  }
}

/** The statuses that ask the owner to act on the worker or its host. */
const NEEDS_ATTENTION: readonly WorkerStatus[] = ['incompatible', 'key_changed', 'unpaired'];

/** Whether a status asks the owner to act on the worker or its host. */
export function needsAttention(status: WorkerStatus): boolean {
  return NEEDS_ATTENTION.includes(status);
}

/** What to tell the owner about a command of the workers feature that failed, and what they can do. */
export function workersFailureMessage(failure: WorkersFailure): string {
  if (failure.kind === 'ipc') {
    return m.shell_error_ipc_failed();
  }
  const { error } = failure;
  switch (error.code) {
    case 'address_invalid': {
      return m.workers_error_address_invalid();
    }
    case 'already_added': {
      return m.workers_error_already_added();
    }
    case 'already_paired': {
      return m.workers_error_already_paired();
    }
    case 'code_refused': {
      return m.workers_error_code_refused();
    }
    case 'credential_store_unavailable': {
      return m.workers_error_credential_store_unavailable();
    }
    case 'incompatible': {
      return m.workers_error_incompatible({ version: error.version });
    }
    case 'key_changed': {
      return m.workers_error_key_changed();
    }
    case 'name_invalid': {
      return m.workers_error_name_invalid({ maxCharacters: error.maxCharacters });
    }
    case 'name_taken': {
      return m.workers_error_name_taken();
    }
    case 'not_a_worker': {
      return m.workers_error_not_a_worker();
    }
    case 'shutting_down': {
      return m.workers_error_shutting_down();
    }
    case 'storage_failed': {
      return m.workers_error_storage_failed();
    }
    case 'store_unavailable': {
      return m.shell_error_store_unavailable();
    }
    case 'too_many_attempts': {
      return m.workers_error_too_many_attempts();
    }
    case 'unreachable': {
      return m.workers_error_unreachable();
    }
    case 'worker_not_found': {
      return m.workers_error_worker_not_found();
    }
  }
}

/** What a worker's ceiling lets its tasks do. */
export function ceilingModeLabel(mode: CeilingMode): string {
  switch (mode) {
    case 'ask': {
      return m.workers_settings_ceiling_ask();
    }
    case 'autonomous': {
      return m.workers_settings_ceiling_autonomous();
    }
    case 'full_access': {
      return m.workers_settings_ceiling_full_access();
    }
    case 'read_only': {
      return m.workers_settings_ceiling_read_only();
    }
  }
}
