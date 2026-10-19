// Messages for the codes and states of the workers feature (ADR-0019): a
// worker's status, the commands' errors, the question before turning Workers
// off and the modes of a ceiling. The UI shows the catalog's message for a
// code and never the core's description. A worker's name, purpose, facts and
// folders are content and never pass through these functions.
import type { WorkItem } from '@mannie/plugin-sdk';
import { hostRefusalMessage, ipcFailureMessage } from '@mannie/plugin-sdk/messages';

import type { CeilingMode, WorkerStatus } from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import type { WorkersFailure } from '../types/workers.js';

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
    return ipcFailureMessage();
  }
  const { error } = failure;
  if (error.origin === 'host') {
    return hostRefusalMessage(error, m.workers_plugin_name());
  }
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
    case 'storage_failed': {
      return m.workers_error_storage_failed();
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

/**
 * Why the executor of the Workers plugin did not take a task, by the code
 * that it gave (ADR-0031, decision 6), or `null` for a code that this
 * version does not know. The worker's name is content.
 */
export function workersRefusalMessage(code: string, worker: string | null): string | null {
  const name = worker ?? '';
  switch (code) {
    case 'profile_not_reachable': {
      return m.workers_refusal_profile_not_reachable({ worker: name });
    }
    case 'still_running': {
      return m.workers_refusal_still_running({ worker: name });
    }
    case 'worker_incompatible': {
      return m.workers_refusal_worker_incompatible({ worker: name });
    }
    case 'worker_offline': {
      return m.workers_refusal_worker_offline({ worker: name });
    }
    case 'worker_unknown': {
      return m.workers_refusal_worker_unknown({ worker: name });
    }
    default: {
      return null;
    }
  }
}

/** The code of the work item that counts the worker tasks that are running, as the core reports it. */
export const WORKERS_ACTIVE_TASKS = 'active_tasks';

/**
 * What turning Workers off does to its worker tasks that are running, or
 * `null` when none is. Items with other codes are not Workers' own.
 */
export function workersOffQuestion(items: readonly WorkItem[]): string | null {
  const count = items.find((item) => item.code === WORKERS_ACTIVE_TASKS)?.count ?? 0;
  return count === 0 ? null : m.workers_plugin_off_question({ count });
}

/** Why the Workers plugin could not start, by its code, or `null` for the host's codes. */
export function workersStartFailureMessage(code: string): string | null {
  return code === 'storage_failed' ? m.workers_plugin_failure_storage_failed() : null;
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
