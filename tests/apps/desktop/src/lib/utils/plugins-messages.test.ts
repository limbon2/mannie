// The messages of the plugins feature (docs/features/plugins.md; ADR-0019):
// every error of the plugins commands has a message of its own, a plugin words
// its own start failures and the core words the host's, and a plugin's state
// line says when it starts, stops or failed.
import { afterEach, describe, expect, it } from 'vitest';

import type { PluginsError } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { setPluginCatalog } from '../../../../../../apps/desktop/src/lib/state/plugin-catalog.js';
import {
  failedForPluginOff,
  pluginNativeLabel,
  pluginServiceLabel,
  pluginsFailureMessage,
  pluginStartFailureMessage,
  pluginStateMessage,
} from '../../../../../../apps/desktop/src/lib/utils/plugins-messages.js';

/** What a plugin uses that declares nothing. */
const NO_USES = {
  channels: [],
  commands: [],
  crates: [],
  installedPrograms: [],
  native: [],
  programs: [],
  secrets: [],
  services: [],
};

afterEach(() => {
  setPluginCatalog([]);
});

describe('pluginsFailureMessage', () => {
  it('has a message for every error, and one for a call that the core did not answer', () => {
    const errors: PluginsError[] = [
      { code: 'unknown_plugin' },
      { code: 'storage_failed' },
      { code: 'store_unavailable' },
      { code: 'shutting_down' },
    ];

    const messages = errors.map((error) => pluginsFailureMessage({ error, kind: 'core' }));

    expect(new Set(messages).size).toBe(errors.length);
    expect(pluginsFailureMessage({ kind: 'ipc' })).not.toBe('');
  });
});

describe('failedForPluginOff', () => {
  it('knows the failures of actions that need a plugin that is off', () => {
    expect(failedForPluginOff({ error: { code: 'plugin_off' }, kind: 'core' })).toBe(true);
    expect(failedForPluginOff({ error: { code: 'storage_failed' }, kind: 'core' })).toBe(false);
    expect(failedForPluginOff({ kind: 'ipc' })).toBe(false);
    expect(failedForPluginOff(null)).toBe(false);
  });
});

describe('pluginStartFailureMessage', () => {
  it('words a plugin’s own codes as the plugin does, the host’s codes as the core does, and names any other code', () => {
    setPluginCatalog([
      {
        description: () => '',
        failure: (code) => (code === 'storage_failed' ? 'The workers could not be read.' : null),
        id: 'workers',
        name: () => 'Workers',
        offQuestion: () => null,
      },
    ]);

    const own = pluginStartFailureMessage('workers', 'storage_failed');
    const host = [
      pluginStartFailureMessage('workers', 'grant_missing'),
      pluginStartFailureMessage('workers', 'service_unavailable'),
      pluginStartFailureMessage('workers', 'sdk_incompatible'),
      pluginStartFailureMessage('workers', 'contributions_refused'),
    ];
    const other = pluginStartFailureMessage('workers', 'disk_on_fire');

    expect(own).toBe('The workers could not be read.');
    expect(new Set([own, ...host, other]).size).toBe(6);
    expect(other).toContain('disk_on_fire');
  });
});

describe('pluginStateMessage', () => {
  it('says nothing for a plugin that is on or off, and says when one starts, stops or failed', () => {
    const quiet = [
      pluginStateMessage({ enabled: true, failure: null, id: 'workers', state: 'on', uses: NO_USES }),
      pluginStateMessage({ enabled: false, failure: null, id: 'workers', state: 'off', uses: NO_USES }),
    ];
    const lines = [
      pluginStateMessage({ enabled: true, failure: null, id: 'workers', state: 'starting', uses: NO_USES }),
      pluginStateMessage({ enabled: false, failure: null, id: 'workers', state: 'stopping', uses: NO_USES }),
      pluginStateMessage({ enabled: true, failure: 'grant_missing', id: 'workers', state: 'failed', uses: NO_USES }),
    ];

    expect(quiet).toEqual([null, null]);
    expect(new Set(lines).size).toBe(3);
    expect(lines[2]).toContain(pluginStartFailureMessage('workers', 'grant_missing'));
  });
});

describe('pluginServiceLabel', () => {
  it('words each service of SDK 1 in a sentence of its own and names any other service', () => {
    const services = ['credentials', 'executor-host', 'file-dialog', 'secrets', 'store'];

    const labels = services.map((name) => pluginServiceLabel(name));
    const other = pluginServiceLabel('teleport');

    expect(new Set([...labels, other]).size).toBe(services.length + 1);
    expect(other).toContain('teleport');
  });
});

describe('pluginNativeLabel', () => {
  it('words each kind of direct native call in a sentence of its own and names any other kind', () => {
    const categories = ['file-writes', 'processes', 'sockets'];

    const labels = categories.map((category) => pluginNativeLabel(category));
    const other = pluginNativeLabel('threads');

    expect(new Set([...labels, other]).size).toBe(categories.length + 1);
    expect(pluginNativeLabel('sockets')).toBe('Opens network connections itself');
    expect(other).toContain('threads');
  });
});
