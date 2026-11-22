// Tests registration, lifecycle messages, mentions, commands and tool cards of
// the Network Client plugin UI (ADR-0030; ADR-0075).
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { describe, expect, it, vi } from 'vitest';

import plugin from '../../ui/index.js';

const library = vi.hoisted(() => ({
  findCollection: vi.fn(),
  findRequest: vi.fn(),
  follow: vi.fn(),
}));

vi.mock('../../ui/state/network.svelte.js', () => ({ networkLibrary: library }));
vi.mock('@mannie/plugin-sdk/navigation', () => ({
  openView: vi.fn(),
  viewParameters: () => new URLSearchParams(),
}));

function toolCall(argumentsOf: Record<string, unknown>, data: Record<string, unknown> | null) {
  return {
    arguments: argumentsOf,
    data,
    failure: null,
    status: 'completed' as const,
  };
}

describe('the network plugin', () => {
  it('registers its shell entries only while the plugin is on', () => {
    const context = new TestPluginContext('network');

    plugin.register(context);
    const whileOff = [
      context.shell.navigation.entries,
      context.shell.views.entries,
      context.shell.sidebarPanels.entries,
    ];
    context.turnOn();

    expect(whileOff).toEqual([[], [], []]);
    expect(
      context.shell.navigation.entries.map((entry) => [entry.id, entry.order, entry.view, entry.sidebarResizable]),
    ).toEqual([['network', 25, 'network', true]]);
    expect(context.shell.views.entries.map((view) => view.id)).toEqual(['network']);
    expect(context.shell.sidebarPanels.entries.map((panel) => [panel.id, panel.activity])).toEqual([
      ['network.collections', 'network'],
    ]);
    expect(context.shell.overlays.entries.map((overlay) => overlay.id)).toEqual(['network.dialogs']);
    expect(context.shell.commands.entries.map((command) => command.id)).toEqual([
      'network.new-collection',
      'network.import-openapi',
      'network.cookies',
      'network.global-variables',
    ]);
    expect(library.follow).toHaveBeenCalledWith(context);
  });

  it('words start failures and active-request off questions', () => {
    expect(plugin.failure?.('storage_failed')).toContain('could not read its collections');
    expect(plugin.failure?.('credential_store_failed')).toContain('credential store');
    expect(plugin.failure?.('http_client_failed')).toContain('HTTP client');
    expect(plugin.failure?.('unknown_code')).toBeNull();

    expect(plugin.offQuestion?.([])).toBeNull();
    expect(plugin.offQuestion?.([{ code: 'active_requests', count: 1 }])).toContain('A network request is running');
    expect(plugin.offQuestion?.([{ code: 'active_requests', count: 3 }])).toContain('3');
  });

  it('registers mentions and all four tool kinds', () => {
    const context = new TestPluginContext('network', true);

    plugin.register(context);

    const mention = context.shell.mentionKinds.find('network');
    expect(mention?.group()).toBe('Network Client');
    expect(mention?.variantName('collection')).toBe('Collection');
    expect(mention?.variantName('request')).toBe('Saved request');
    expect(mention?.variantName('other')).toBeNull();
    expect(mention?.summary({ code: 'collection', values: {} })).toBe('Collection');
    expect(mention?.refusal('item_not_found')).toContain('no longer available');
    expect(mention?.warning('anything')).toBeNull();

    const listCollections = context.shell.toolKinds.find('network_list_collections');
    const listRequests = context.shell.toolKinds.find('network_list_requests');
    const getRequest = context.shell.toolKinds.find('network_get_request');
    const sendRequest = context.shell.toolKinds.find('network_send_request');

    expect(listCollections?.summary(toolCall({}, null))).toBe('Listed enabled API collections');
    expect(listRequests?.summary(toolCall({ collection: 'Payments' }, null))).toBe('Listed saved requests in Payments');
    expect(getRequest?.summary(toolCall({}, { method: 'GET', requestName: 'Get balance' }))).toBe(
      'Inspected GET Get balance',
    );
    expect(
      sendRequest?.summary(
        toolCall(
          {},
          {
            collectionName: 'Payments',
            method: 'POST',
            requestName: 'Charge',
            status: 201,
          },
        ),
      ),
    ).toBe('POST Charge returned 201');
    expect(sendRequest?.details).toBeDefined();
    expect(sendRequest?.failure('insecure_transport')).toContain('plain HTTP');
  });
});
