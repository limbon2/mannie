// Messages of the TypeScript plugin SDK (ADR-0034, decisions 9 and 11): the
// core words the host's refusals once for every plugin, and names the
// optional feature that is off from the plugin catalog (ADR-0055).
import { afterEach, describe, expect, it } from 'vitest';

import type { HostRefusal } from '../../../../../../apps/desktop/src/lib/generated/plugin-sdk.js';
import { hostRefusalMessage } from '../../../../../../apps/desktop/src/lib/sdk/messages.js';
import { setPluginCatalog } from '../../../../../../apps/desktop/src/lib/state/plugin-catalog.js';

afterEach(() => {
  setPluginCatalog([]);
});

describe('hostRefusalMessage', () => {
  it('has a message for every refusal', () => {
    const refusals: HostRefusal[] = [
      { code: 'plugin_off', plugin: 'workers' },
      { code: 'feature_off', feature: 'databases', plugin: 'workers' },
      { code: 'unknown_command' },
      { code: 'input_too_large', maxBytes: 1024 },
      { code: 'invalid_input', field: null },
      { code: 'not_allowed' },
      { code: 'host_service_failed' },
      { code: 'store_unavailable' },
      { code: 'shutting_down' },
    ];

    const messages = refusals.map((refusal) => hostRefusalMessage(refusal, 'Workers'));

    expect(new Set(messages).size).toBe(refusals.length);
  });

  it('names the feature that is off and its plugin, and the feature’s ID when this version does not list it', () => {
    setPluginCatalog([
      {
        description: () => '',
        failure: () => null,
        features: [{ description: () => '', id: 'databases', name: () => 'Databases' }],
        id: 'workers',
        name: () => 'Workers',
        offQuestion: () => null,
      },
    ]);

    const listed = hostRefusalMessage({ code: 'feature_off', feature: 'databases', plugin: 'workers' }, 'Workers');
    const unlisted = hostRefusalMessage({ code: 'feature_off', feature: 'history', plugin: 'workers' }, 'Workers');

    expect(listed).toContain('Databases');
    expect(listed).toContain('Workers');
    expect(unlisted).toContain('history');
  });
});
