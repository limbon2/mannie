// The messages of the workers feature (docs/features/workers/
// worker-activity.md; ADR-0019): every status, error and ceiling mode has a
// catalog message of its own, and an error tells the owner what to do.
import { describe, expect, it } from 'vitest';

import type {
  CeilingMode,
  WorkersError,
  WorkerStatus,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  ceilingModeLabel,
  needsAttention,
  workersFailureMessage,
  workerStatusLabel,
} from '../../../../../../apps/desktop/src/lib/utils/workers-messages.js';

const STATUSES: WorkerStatus[] = ['connecting', 'connected', 'offline', 'incompatible', 'key_changed', 'unpaired'];

describe('workerStatusLabel', () => {
  it('names every status differently', () => {
    const labels = STATUSES.map((status) => workerStatusLabel(status));

    expect(new Set(labels).size).toBe(STATUSES.length);
  });

  it('asks the owner to act only where the host needs it', () => {
    expect(STATUSES.filter((status) => needsAttention(status))).toEqual(['incompatible', 'key_changed', 'unpaired']);
  });
});

describe('workersFailureMessage', () => {
  it('has a message for every error, and names the version of an incompatible worker', () => {
    const errors: WorkersError[] = [
      { code: 'address_invalid' },
      { code: 'unreachable' },
      { code: 'not_a_worker' },
      { code: 'code_refused' },
      { code: 'too_many_attempts' },
      { code: 'already_paired' },
      { code: 'already_added' },
      { code: 'key_changed' },
      { code: 'incompatible', version: '0.2.7' },
      { code: 'worker_not_found' },
      { code: 'credential_store_unavailable' },
      { code: 'storage_failed' },
      { code: 'store_unavailable' },
      { code: 'shutting_down' },
      { code: 'name_invalid', maxCharacters: 64 },
      { code: 'name_taken' },
    ];

    const messages = errors.map((error) => workersFailureMessage({ error, kind: 'core' }));

    expect(new Set(messages).size).toBe(errors.length);
    expect(messages[8]).toContain('0.2.7');
    expect(messages[5]).toContain('mannie-worker pair --reset');
    expect(messages[14]).toContain('64');
  });

  it('says that the core did not answer a call', () => {
    expect(workersFailureMessage({ kind: 'ipc' })).not.toBe('');
  });
});

describe('ceilingModeLabel', () => {
  it('describes every mode differently', () => {
    const modes: CeilingMode[] = ['read_only', 'ask', 'autonomous', 'full_access'];

    expect(new Set(modes.map((mode) => ceilingModeLabel(mode))).size).toBe(modes.length);
  });
});
