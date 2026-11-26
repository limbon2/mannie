// Configuration approvals in `ToolApproval` (ADR-0081): always offer only
// Allow once and Deny (never reusable grants or saved rules), render masked
// password inputs with autocomplete off for every declared secret field, and
// distinguish required, optional and chat-provided secrets.
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import ToolApproval from '../../../../../../../../apps/desktop/src/lib/components/app/tools/ToolApproval.svelte';
import type { Access, ApprovalRequest } from '../../../../../../../../apps/desktop/src/lib/generated/bindings.js';

const CONVERSATION = '0199e0f2-0000-7000-8000-000000000001';
const PART = '0199e0f2-0000-7000-8000-000000000002';

const CONFIG_ACCESS: Access = {
  action: 'add_host',
  domain: 'ssh',
  fields: [{ label: 'Host', value: 'prod.internal' }],
  kind: 'app_config',
  owner: 'ssh',
  payload: '{"host":"prod.internal"}',
  payloadCut: false,
  secretFields: [
    { key: 'password', label: 'SSH password', providedInChat: false, required: true },
    { key: 'passphrase', label: 'Key passphrase', providedInChat: true, required: true },
    { key: 'note', label: 'Optional token', providedInChat: false, required: false },
  ],
  securitySensitive: true,
  title: 'Add SSH host prod.internal',
};

const CONFIG_REQUEST: ApprovalRequest = {
  digest: 'sha256:cfg',
  grants: [],
  project: null,
  reasons: ['app_config_security'],
  rule: null,
};

function renderApproval(accesses: readonly Access[], request: ApprovalRequest): string {
  return render(ToolApproval, {
    props: {
      accesses,
      conversationId: CONVERSATION,
      partId: PART,
      request,
    },
  }).body;
}

describe('ToolApproval', () => {
  it('renders masked password inputs, required and chat-provided indicators, and only Allow once and Deny', () => {
    const body = renderApproval([CONFIG_ACCESS], CONFIG_REQUEST);

    expect(body).toContain('data-config-secrets');
    expect(body).toContain('data-config-secret-field="password"');
    expect(body).toContain('data-config-secret-field="passphrase"');
    expect(body).toContain('data-config-secret-field="note"');
    expect(body).toContain('type="password"');
    expect(body).toContain('autocomplete="off"');
    expect(body).toContain('data-secret-in-chat');
    expect(body).toContain('data-app-config-security-note');
    expect(body).toContain('Allow once');
    expect(body).toContain('Deny');
    expect(body).not.toContain('data-allow-for');
    expect(body).not.toContain('Save rule');
  });

  it('omits the secrets box when a configuration action takes no secrets', () => {
    const access: Access = { ...CONFIG_ACCESS, secretFields: [], securitySensitive: false };
    const request: ApprovalRequest = { ...CONFIG_REQUEST, reasons: ['app_config'] };
    const body = renderApproval([access], request);

    expect(body).not.toContain('data-config-secrets');
    expect(body).not.toContain('data-app-config-security-note');
    expect(body).toContain('Allow once');
    expect(body).toContain('Deny');
  });
});
