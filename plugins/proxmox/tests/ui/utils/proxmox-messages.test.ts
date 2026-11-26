// Unit tests for Proxmox plugin error classification and localized messages.
import { describe, expect, it } from 'vitest';

import type { ProxmoxFailure } from '../../../ui/types/proxmox.js';
import {
  isCertificateFailure,
  isServerGone,
  proxmoxFailureMessage,
  proxmoxOffQuestion,
  refusedField,
} from '../../../ui/utils/proxmox-messages.js';

function pluginFailure(
  code: 'certificate_mismatch' | 'certificate_untrusted' | 'server_not_found' | 'vm_not_allowed',
): ProxmoxFailure {
  return {
    error: { code, description: 'test', origin: 'plugin' },
    kind: 'core',
  };
}

describe('proxmox message helpers', () => {
  it('classifies certificate failures and missing server failures accurately', () => {
    expect(isCertificateFailure(pluginFailure('certificate_untrusted'))).toBe(true);
    expect(isCertificateFailure(pluginFailure('certificate_mismatch'))).toBe(true);
    expect(isCertificateFailure(pluginFailure('vm_not_allowed'))).toBe(false);
    expect(isCertificateFailure({ kind: 'ipc' })).toBe(false);

    expect(isServerGone(pluginFailure('server_not_found'))).toBe(true);
    expect(isServerGone(pluginFailure('vm_not_allowed'))).toBe(false);
  });

  it('extracts refused validation fields and returns localized error text', () => {
    const invalidHost: ProxmoxFailure = {
      error: { code: 'invalid_input', description: 'invalid', field: 'host', origin: 'host' },
      kind: 'core',
    };
    expect(refusedField(invalidHost)).toBe('host');
    expect(proxmoxFailureMessage(invalidHost)).not.toBe('');
    expect(proxmoxFailureMessage(pluginFailure('vm_not_allowed'))).not.toBe('');
    expect(proxmoxFailureMessage({ kind: 'ipc' })).not.toBe('');
  });

  it('pluralizes the turn-off confirmation message for active VNC sessions', () => {
    expect(proxmoxOffQuestion([{ code: 'vnc_sessions', count: 1 }])).toContain('1');
    expect(proxmoxOffQuestion([{ code: 'vnc_sessions', count: 3 }])).toContain('3');
    expect(proxmoxOffQuestion([{ code: 'vnc_sessions', count: 0 }])).toBeNull();
  });
});
