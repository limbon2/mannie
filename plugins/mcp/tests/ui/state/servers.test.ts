// The list of MCP servers and the owner's actions on it (plugins/mcp/docs/
// README.md, settings): the list follows the plugin's channel while the
// plugin is on, adding a server moves a variable whose name looks like a
// secret's among the secrets, editing keeps the stored secrets unless the
// owner typed new ones, a refusal stays in its dialog, a server that is gone
// counts as removed, a sign-in opens the authorization server's page, the
// tools' marks clear once the owner saw them, Paste JSON chooses the servers
// that can be added, and the dialogs close as the plugin turns off.
import type * as Ipc from '@mannie/plugin-sdk/ipc';
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ServerListUpdate, ServerView, ToolView } from '../../../ui/generated/bindings.js';
import { McpActions, PasteRequest, ToolsRequest } from '../../../ui/state/mcp-actions.svelte.js';
import { ServerList, serverList } from '../../../ui/state/servers.svelte.js';

const core = vi.hoisted(() => ({
  acknowledgeTools: vi.fn(),
  addServer: vi.fn(),
  beginSignIn: vi.fn(),
  followServers: vi.fn(),
  importConfiguration: vi.fn(),
  locateProgram: vi.fn(),
  openLink: vi.fn(),
  parseConfiguration: vi.fn(),
  removeServer: vi.fn(),
  setServerEnabled: vi.fn(),
  setToolEnabled: vi.fn(),
  unsubscribe: vi.fn(),
  updateServer: vi.fn(),
}));

/** Hands batches of messages to the list, as the plugin's channel does. */
const channel = vi.hoisted(() => ({
  deliver: (_messages: readonly Ipc.ChannelMessage<ServerListUpdate>[]): void => undefined,
}));

vi.mock('../../../ui/generated/bindings.js', () => ({
  channels: { followServers: core.followServers },
  commands: {
    acknowledgeTools: core.acknowledgeTools,
    addServer: core.addServer,
    beginSignIn: core.beginSignIn,
    importConfiguration: core.importConfiguration,
    locateProgram: core.locateProgram,
    parseConfiguration: core.parseConfiguration,
    removeServer: core.removeServer,
    setServerEnabled: core.setServerEnabled,
    setToolEnabled: core.setToolEnabled,
    updateServer: core.updateServer,
  },
}));
vi.mock('@mannie/plugin-sdk/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  batchChannel: (onItems: (messages: readonly Ipc.ChannelMessage<ServerListUpdate>[]) => void) => {
    channel.deliver = onItems;
    return {};
  },
  unsubscribe: core.unsubscribe,
}));
vi.mock('@mannie/plugin-sdk/links', () => ({ linkFailureMessage: () => 'refused', openLink: core.openLink }));

function server(name: string, overrides: Partial<ServerView> = {}): ServerView {
  return {
    arguments: ['-y', 'server'],
    clientId: null,
    clientSecretSet: false,
    enabled: true,
    enabledToolCount: 1,
    failure: null,
    folder: null,
    id: `id-${name}`,
    images: true,
    instructions: true,
    missingSecrets: [],
    name,
    oauth: false,
    plain: [{ name: 'HOST', value: 'example.com' }],
    prefix: name.toLowerCase(),
    program: 'npx',
    protocolVersion: null,
    resourceCount: 0,
    secretNames: ['API_TOKEN'],
    signedIn: false,
    status: 'starting',
    timeoutSeconds: 300,
    toolCount: 2,
    transport: 'stdio',
    url: null,
    warnings: [],
    ...overrides,
  };
}

function tool(name: string, overrides: Partial<ToolView> = {}): ToolView {
  return {
    description: 'Does a thing.',
    enabled: false,
    hints: [],
    modelName: `mcp_test_${name}`,
    name,
    new: false,
    problem: null,
    title: null,
    ...overrides,
  };
}

function ok<T>(data: T): { data: T; status: 'ok' } {
  return { data, status: 'ok' };
}

function refused(code: string): { error: { code: string; description: string; origin: 'plugin' }; status: 'error' } {
  return { error: { code, description: 'refused', origin: 'plugin' }, status: 'error' };
}

/** The request of a dialog that an action opened. */
function opened<T>(request: T | null): T {
  if (request === null) {
    throw new Error('the action opened no dialog');
  }
  return request;
}

/** Waits for the promises that a test started without awaiting. */
async function settle(): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    await Promise.resolve();
  }
}

beforeEach(() => {
  for (const command of Object.values(core)) {
    command.mockReset();
  }
  core.followServers.mockResolvedValue({ data: 7, status: 'ok' });
  core.openLink.mockResolvedValue(null);
  serverList.servers = [];
});

describe('the list of servers', () => {
  it('loads until the core sends the servers, then follows each change', async () => {
    const context = new TestPluginContext('mcp', true);
    const list = new ServerList();
    list.follow(context);
    await settle();

    expect(list.status).toBe('loading');
    channel.deliver([{ item: { enabledTools: 1, kind: 'servers', servers: [server('A')] }, kind: 'item' }]);
    channel.deliver([
      {
        item: { enabledTools: 3, kind: 'servers', servers: [server('A', { status: 'connected' }), server('B')] },
        kind: 'item',
      },
    ]);

    expect(list.status).toBe('ready');
    expect(list.enabledTools).toBe(3);
    expect(list.servers.map((known) => [known.name, known.status])).toEqual([
      ['A', 'connected'],
      ['B', 'starting'],
    ]);
    context.turnOff();
    expect(list.servers).toEqual([]);
    expect(core.unsubscribe).toHaveBeenCalledWith(7);
  });
});

describe('the server dialog', () => {
  it('adds a server with a secret-looking variable among the secrets and closes', async () => {
    const actions = new McpActions();
    actions.startAdd();
    const request = opened(actions.edit);
    request.name = 'GitHub';
    request.program = 'npx';
    request.argumentsText = '-y\n@modelcontextprotocol/server-github';
    request.plain = [{ name: 'GITHUB_TOKEN', value: 'planted' }];
    core.addServer.mockResolvedValue(ok(server('GitHub')));

    await actions.confirmServer(request);

    const [input] = core.addServer.mock.calls[0] as [{ plain: unknown[]; secrets: unknown[] }];
    expect(input.plain).toEqual([]);
    expect(input.secrets).toEqual([{ name: 'GITHUB_TOKEN', value: 'planted' }]);
    expect(request.moved).toEqual(['GITHUB_TOKEN']);
    expect(actions.editOpen).toBe(false);
    expect(serverList.servers.map((known) => known.name)).toEqual(['GitHub']);
  });

  it('keeps the stored secrets of a server to edit, and keeps a refusal in the dialog', async () => {
    const actions = new McpActions();
    actions.startEdit(server('GitHub', { missingSecrets: [] }));
    const request = opened(actions.edit);
    core.updateServer.mockResolvedValue(refused('name_taken'));

    await actions.confirmServer(request);

    const [update] = core.updateServer.mock.calls[0] as [{ server: { secrets: unknown[] }; serverId: string }];
    expect(update.serverId).toBe('id-GitHub');
    expect(update.server.secrets).toEqual([{ name: 'API_TOKEN', value: null }]);
    expect(actions.editOpen).toBe(true);
    expect(request.failure).not.toBeNull();
  });
});

describe('the actions of a server', () => {
  it('counts a server that is gone as removed', async () => {
    const actions = new McpActions();
    serverList.servers = [server('Old')];
    actions.startRemove(server('Old'));
    core.removeServer.mockResolvedValue(refused('server_not_found'));

    await actions.confirmRemove(opened(actions.removing));

    expect(serverList.servers).toEqual([]);
    expect(actions.removeOpen).toBe(false);
  });

  it('starts a sign-in with the page of the catalog and opens the authorization server', async () => {
    const actions = new McpActions();
    const remote = server('Linear', { oauth: true, transport: 'http', url: 'https://mcp.linear.app/mcp' });
    core.beginSignIn.mockResolvedValue(ok({ host: 'linear.app', url: 'https://linear.app/oauth/authorize?state=s' }));
    core.openLink.mockResolvedValue(null);

    await actions.signIn(remote);

    const [begin] = core.beginSignIn.mock.calls[0] as [{ page: { title: string }; serverId: string }];
    expect(begin.serverId).toBe('id-Linear');
    expect(begin.page.title).toBe('Mannie sign-in');
    expect(core.openLink).toHaveBeenCalledWith('https://linear.app/oauth/authorize?state=s');
    expect(actions.signIns['id-Linear']?.status).toBe('waiting');
    expect(actions.signIns['id-Linear']?.host).toBe('linear.app');
  });

  it('switches a tool, and clears the marks of new tools once the owner saw them', async () => {
    const actions = new McpActions();
    const request = new ToolsRequest(server('Test'));
    request.tools = [tool('echo'), tool('late', { new: true })];
    core.setToolEnabled.mockResolvedValue(ok(null));
    core.acknowledgeTools.mockResolvedValue(ok(null));

    await actions.setToolEnabled(request, opened(request.tools[0] ?? null), true);
    actions.closeTools(request);

    expect(core.setToolEnabled).toHaveBeenCalledWith({ enabled: true, serverId: 'id-Test', tool: 'echo' });
    expect(request.tools[0]?.enabled).toBe(true);
    expect(core.acknowledgeTools).toHaveBeenCalledWith({ serverId: 'id-Test' });
  });
});

describe('Paste JSON', () => {
  it('chooses the servers that can be added and adds the chosen ones', async () => {
    const actions = new McpActions();
    const request = new PasteRequest();
    request.text = '{"mcpServers":{}}';
    const preview = (key: string, refusal: 'sse_only' | null) => ({
      arguments: [],
      disabled: false,
      key,
      missingSecrets: [],
      plainNames: [],
      program: 'npx',
      refusal,
      secretNames: [],
      transport: 'stdio' as const,
      url: null,
    });
    core.parseConfiguration.mockResolvedValue(ok([preview('files', null), preview('events', 'sse_only')]));
    core.importConfiguration.mockResolvedValue(ok({ added: [server('files')], skipped: [] }));

    await actions.readPaste(request);
    await actions.importChosen(request);

    expect(request.chosen).toEqual(['files']);
    expect(core.importConfiguration).toHaveBeenCalledWith({ keys: ['files'], text: '{"mcpServers":{}}' });
    expect(request.outcome?.added.map((added) => added.name)).toEqual(['files']);
    expect(serverList.servers.map((known) => known.name)).toEqual(['files']);
  });

  it('closes every dialog as the plugin turns off', () => {
    const actions = new McpActions();
    actions.startAdd();
    actions.startPaste();

    actions.closeAll();

    expect([actions.editOpen, actions.pasteOpen, actions.logOpen, actions.toolsOpen]).toEqual([
      false,
      false,
      false,
      false,
    ]);
  });
});
