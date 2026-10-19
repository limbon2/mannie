// Every code of the SSH plugin has words of its own (ADR-0019): the
// plugin's errors, the fields that the host's check refused, the host's
// refusals, a call without an answer, and the statuses of hosts.
import { describe, expect, it } from 'vitest';

import type { HostStatus, SshError } from '../../../ui/generated/bindings.js';
import type { SshFailure } from '../../../ui/types/ssh.js';
import { hostStatusLabel, isHostGone, refusedField, sshFailureMessage } from '../../../ui/utils/ssh-messages.js';

const ERRORS: SshError['code'][] = [
  'name_taken',
  'host_not_found',
  'unreachable',
  'timed_out',
  'handshake_failed',
  'host_key_unsupported',
  'host_key_changed',
  'host_key_not_trusted',
  'scan_outdated',
  'auth_failed',
  'auth_method_unsupported',
  'password_missing',
  'credential_store_unavailable',
  'storage_failed',
];

const FIELDS = ['name', 'address', 'port', 'username', 'password', 'note'];

const STATUSES: HostStatus[] = ['ready', 'key_not_checked', 'key_changed', 'password_missing'];

function pluginError(code: SshError['code']): SshFailure {
  return { error: { code, description: 'refused', origin: 'plugin' }, kind: 'core' };
}

function invalidInput(field: string | null): SshFailure {
  return { error: { code: 'invalid_input', description: 'refused', field, origin: 'host' }, kind: 'core' };
}

function expectDistinct(messages: string[]): void {
  for (const message of messages) {
    expect(message.trim()).not.toBe('');
  }
  expect(new Set(messages).size).toBe(messages.length);
}

describe('SSH messages', () => {
  it('say what each error of the plugin means', () => {
    expectDistinct(ERRORS.map((code) => sshFailureMessage(pluginError(code))));
  });

  it('say what each refused field needs, and fall back to the host for any other refusal', () => {
    const fields = FIELDS.map((field) => sshFailureMessage(invalidInput(field)));
    const unnamed = sshFailureMessage(invalidInput(null));
    const off = sshFailureMessage({
      error: { code: 'plugin_off', description: 'off', origin: 'host', plugin: 'ssh' },
      kind: 'core',
    });

    expectDistinct([...fields, unnamed, off]);
    expect(sshFailureMessage({ kind: 'ipc' })).not.toBe('');
    expect(refusedField(invalidInput('address'))).toBe('address');
    expect(refusedField(pluginError('name_taken'))).toBeNull();
    expect(refusedField(null)).toBeNull();
  });

  it('name each status of a host', () => {
    expectDistinct(STATUSES.map((status) => hostStatusLabel(status)));
  });

  it('count a host that is already gone as removed', () => {
    expect(isHostGone(pluginError('host_not_found'))).toBe(true);
    expect(isHostGone(pluginError('storage_failed'))).toBe(false);
    expect(isHostGone({ kind: 'ipc' })).toBe(false);
  });
});
