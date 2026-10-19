// The messages of the workers feature (plugins/workers/docs/
// worker-activity.md; ADR-0019): every status, error, refusal and ceiling
// mode has a catalog message of its own, an error tells the owner what to
// do, and the host's refusals of its commands name the plugin.
import { describe, expect, it } from 'vitest';

import type { CeilingMode, WorkersError, WorkerStatus } from '../../../ui/generated/bindings.js';
import {
  ceilingModeLabel,
  needsAttention,
  WORKERS_ACTIVE_TASKS,
  workersFailureMessage,
  workersOffQuestion,
  workersRefusalMessage,
  workersStartFailureMessage,
  workerStatusLabel,
} from '../../../ui/utils/workers-messages.js';

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
      { code: 'name_invalid', maxCharacters: 64 },
      { code: 'name_taken' },
    ];

    const messages = errors.map((error) =>
      workersFailureMessage({ error: { ...error, description: '', origin: 'plugin' }, kind: 'core' }),
    );

    expect(new Set(messages).size).toBe(errors.length);
    expect(messages[8]).toContain('0.2.7');
    expect(messages[5]).toContain('mannie-worker pair --reset');
    expect(messages[12]).toContain('64');
  });

  it('words the host’s refusals with the plugin’s name', () => {
    const off = workersFailureMessage({
      error: { code: 'plugin_off', description: '', origin: 'host', plugin: 'workers' },
      kind: 'core',
    });
    const quitting = workersFailureMessage({
      error: { code: 'shutting_down', description: '', origin: 'host' },
      kind: 'core',
    });

    expect(off).toBe('Workers is off. Turn it on in Settings, Plugins, then try again.');
    expect(quitting).toBe('Mannie is quitting.');
  });

  it('says that the core did not answer a call', () => {
    expect(workersFailureMessage({ kind: 'ipc' })).not.toBe('');
  });
});

describe('workersRefusalMessage', () => {
  it('says why the executor took no task for each code, by the worker’s name', () => {
    const codes = ['worker_offline', 'worker_unknown', 'worker_incompatible', 'profile_not_reachable', 'still_running'];

    const messages = codes.map((code) => workersRefusalMessage(code, 'build-01'));

    expect(new Set(messages).size).toBe(codes.length);
    expect(messages.every((message) => message?.includes('build-01') ?? false)).toBe(true);
  });

  it('leaves a code that this version does not know to the core', () => {
    expect(workersRefusalMessage('newer_code', 'build-01')).toBeNull();
  });
});

describe('workersStartFailureMessage', () => {
  it('words the plugin’s own start failure, and leaves the host’s codes to the core', () => {
    expect(workersStartFailureMessage('storage_failed')).not.toBeNull();
    expect(workersStartFailureMessage('grant_missing')).toBeNull();
  });
});

describe('workersOffQuestion', () => {
  it('asks nothing without running worker tasks, also for work of another code', () => {
    expect(workersOffQuestion([])).toBeNull();
    expect(workersOffQuestion([{ code: 'newer_code', count: 2 }])).toBeNull();
  });

  it('says what happens to one running worker task, and to several by their count', () => {
    const one = workersOffQuestion([{ code: WORKERS_ACTIVE_TASKS, count: 1 }]);
    const three = workersOffQuestion([
      { code: 'newer_code', count: 2 },
      { code: WORKERS_ACTIVE_TASKS, count: 3 },
    ]);

    expect(WORKERS_ACTIVE_TASKS).toBe('active_tasks');
    expect(one).toMatch(/^A worker task is running\. It keeps running on its worker/);
    expect(three).toMatch(/^3 worker tasks are running\. They keep running on their workers/);
  });
});

describe('ceilingModeLabel', () => {
  it('describes every mode differently', () => {
    const modes: CeilingMode[] = ['read_only', 'ask', 'autonomous', 'full_access'];

    expect(new Set(modes.map((mode) => ceilingModeLabel(mode))).size).toBe(modes.length);
  });
});
