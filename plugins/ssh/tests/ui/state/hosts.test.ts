// The list of SSH hosts and the owner's actions on it (plugins/ssh/docs/
// README.md, settings): the list follows the plugin's channel while the
// plugin is on, adding a host closes its dialog and opens its key dialog,
// which reads the key, trusting the key tests the login, editing keeps the
// password unless the owner typed one, a refusal stays in its dialog, a
// host that is gone counts as removed, and the dialogs close as the plugin
// turns off.
import type * as Ipc from '@mannie/plugin-sdk/ipc';
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { HostKeyScan, HostListUpdate, HostView } from '../../../ui/generated/bindings.js';
import { HostList, hostList } from '../../../ui/state/hosts.svelte.js';
import { SshActions } from '../../../ui/state/ssh-actions.svelte.js';

const core = vi.hoisted(() => ({
  addHost: vi.fn(),
  followHosts: vi.fn(),
  removeHost: vi.fn(),
  scanHostKey: vi.fn(),
  testHost: vi.fn(),
  trustHostKey: vi.fn(),
  unsubscribe: vi.fn(),
  updateHost: vi.fn(),
}));

/** Hands batches of messages to the list, as the plugin's channel does. */
const channel = vi.hoisted(() => ({
  deliver: (_messages: readonly Ipc.ChannelMessage<HostListUpdate>[]): void => undefined,
}));

vi.mock('../../../ui/generated/bindings.js', () => ({
  channels: { followHosts: core.followHosts },
  commands: {
    addHost: core.addHost,
    removeHost: core.removeHost,
    scanHostKey: core.scanHostKey,
    testHost: core.testHost,
    trustHostKey: core.trustHostKey,
    updateHost: core.updateHost,
  },
}));
vi.mock('@mannie/plugin-sdk/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  batchChannel: (onItems: (messages: readonly Ipc.ChannelMessage<HostListUpdate>[]) => void) => {
    channel.deliver = onItems;
    return {};
  },
  unsubscribe: core.unsubscribe,
}));

const FINGERPRINT = 'SHA256:3BsTpRlWni0K5eIGEPA9URiaFoYsnFlC/YHer8ymt9Y';

function host(name: string, status: HostView['status'] = 'key_not_checked'): HostView {
  return {
    address: '10.0.0.5',
    endpoint: 'deploy@10.0.0.5:22',
    id: `id-${name}`,
    key: status === 'key_not_checked' ? null : { algorithm: 'ssh-ed25519', at: 1, fingerprint: FINGERPRINT },
    name,
    note: '',
    passwordSet: true,
    port: 22,
    seenKey: null,
    status,
    username: 'deploy',
  };
}

function scan(comparison: HostKeyScan['comparison']): HostKeyScan {
  return { comparison, key: { algorithm: 'ssh-ed25519', at: 2, fingerprint: FINGERPRINT } };
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
  core.followHosts.mockResolvedValue({ data: 7, status: 'ok' });
  hostList.hosts = [];
});

describe('the list of hosts', () => {
  it('loads until the core sends the hosts, then follows each change', async () => {
    const context = new TestPluginContext('ssh', true);
    const list = new HostList();
    list.follow(context);
    await settle();

    expect(list.status).toBe('loading');
    channel.deliver([{ item: { hosts: [host('a')], kind: 'hosts' }, kind: 'item' }]);
    channel.deliver([{ item: { hosts: [host('a', 'ready'), host('b')], kind: 'hosts' }, kind: 'item' }]);

    expect(list.status).toBe('ready');
    expect(list.hosts.map((known) => [known.name, known.status])).toEqual([
      ['a', 'ready'],
      ['b', 'key_not_checked'],
    ]);
  });

  it('forgets the hosts and ends the subscription as the plugin turns off', async () => {
    const context = new TestPluginContext('ssh', true);
    const list = new HostList();
    list.follow(context);
    await settle();
    channel.deliver([{ item: { hosts: [host('a')], kind: 'hosts' }, kind: 'item' }]);

    context.turnOff();

    expect(list.hosts).toEqual([]);
    expect(list.status).toBe('loading');
    expect(core.unsubscribe).toHaveBeenCalledWith(7);
  });

  it('reports a list that could not be followed', async () => {
    core.followHosts.mockResolvedValue({
      error: { code: 'plugin_off', description: 'off', origin: 'host', plugin: 'ssh' },
      status: 'error',
    });
    const context = new TestPluginContext('ssh', true);
    const list = new HostList();
    list.follow(context);
    await settle();

    expect(list.status).toBe('error');
    expect(list.failure?.kind).toBe('core');
  });
});

describe('the actions on hosts', () => {
  it('adds a host, then opens its key dialog and reads the key', async () => {
    const actions = new SshActions();
    core.addHost.mockResolvedValue(ok(host('build-box')));
    core.scanHostKey.mockResolvedValue(ok(scan('not_pinned')));
    actions.startAdd();
    const request = opened(actions.edit);
    request.name = ' build-box ';
    request.address = '10.0.0.5';
    request.username = 'deploy';
    request.password = 'correct horse battery';

    await actions.confirmHost(request);
    await settle();

    expect(core.addHost).toHaveBeenCalledWith({
      address: '10.0.0.5',
      name: 'build-box',
      note: '',
      password: 'correct horse battery',
      port: 22,
      username: 'deploy',
    });
    expect(request.password).toBe('');
    expect(actions.editOpen).toBe(false);
    expect(actions.keyOpen).toBe(true);
    expect(actions.key?.scan).toEqual(scan('not_pinned'));
    expect(hostList.hosts.map((known) => known.name)).toEqual(['build-box']);
  });

  it('keeps the password of a host that the owner edits without typing one', async () => {
    const actions = new SshActions();
    core.updateHost.mockResolvedValue(ok(host('build-box', 'ready')));
    actions.startEdit(host('build-box', 'ready'));
    const request = opened(actions.edit);
    request.note = 'Builds the releases.';

    await actions.confirmHost(request);

    expect(core.updateHost).toHaveBeenCalledWith(
      expect.objectContaining({ hostId: 'id-build-box', note: 'Builds the releases.', password: null }),
    );
    expect(actions.keyOpen).toBe(false);
  });

  it('keeps a refused host in its dialog', async () => {
    const actions = new SshActions();
    core.addHost.mockResolvedValue(refused('name_taken'));
    actions.startAdd();
    const request = opened(actions.edit);

    await actions.confirmHost(request);

    expect(actions.editOpen).toBe(true);
    expect(request.failure?.kind).toBe('core');
    expect(actions.keyOpen).toBe(false);
  });

  it('trusts the key that the dialog shows, then tests the login', async () => {
    const actions = new SshActions();
    core.scanHostKey.mockResolvedValue(ok(scan('not_pinned')));
    core.trustHostKey.mockResolvedValue(ok(host('build-box', 'ready')));
    core.testHost.mockResolvedValue(ok(null));
    actions.startCheckKey(host('build-box'));
    await settle();
    const request = actions.key;
    if (request === null) {
      throw new Error('the key dialog is open');
    }

    await actions.confirmTrust(request);

    expect(core.trustHostKey).toHaveBeenCalledWith({ fingerprint: FINGERPRINT, hostId: 'id-build-box' });
    expect(core.testHost).toHaveBeenCalledWith({ hostId: 'id-build-box' });
    expect(request.tested).toBe(true);
    expect(request.host.status).toBe('ready');
    expect(request.scan?.comparison).toBe('same');
  });

  it('names the step that failed: reading the key, or the login after trusting it', async () => {
    const actions = new SshActions();
    core.scanHostKey.mockResolvedValueOnce(refused('unreachable'));
    actions.startCheckKey(host('build-box'));
    await settle();
    const unreachable = actions.key;
    core.scanHostKey.mockResolvedValueOnce(ok(scan('not_pinned')));
    core.trustHostKey.mockResolvedValue(ok(host('build-box', 'ready')));
    core.testHost.mockResolvedValue(refused('auth_failed'));
    actions.startCheckKey(host('build-box'));
    await settle();
    const request = actions.key;
    if (request === null) {
      throw new Error('the key dialog is open');
    }
    await actions.confirmTrust(request);

    expect(unreachable?.failedStep).toBe('scan');
    expect(request.failedStep).toBe('test');
    expect(request.tested).toBe(false);
  });

  it('counts a host that is gone as removed, and keeps one whose removal failed', async () => {
    const actions = new SshActions();
    hostList.hosts = [host('a'), host('b')];
    core.removeHost.mockResolvedValueOnce(refused('host_not_found'));
    actions.startRemove(host('a'));
    const gone = actions.removing;
    if (gone === null) {
      throw new Error('the confirmation is open');
    }
    await actions.confirmRemove(gone);
    core.removeHost.mockResolvedValueOnce(refused('credential_store_unavailable'));
    actions.startRemove(host('b'));
    const kept = actions.removing;
    if (kept === null) {
      throw new Error('the confirmation is open');
    }
    await actions.confirmRemove(kept);

    expect(hostList.hosts.map((known) => known.name)).toEqual(['b']);
    expect(kept.failure?.kind).toBe('core');
    expect(actions.removeOpen).toBe(true);
  });

  it('keeps the outcome of a login test for the settings, and closes everything as the plugin turns off', async () => {
    const actions = new SshActions();
    core.testHost.mockResolvedValueOnce(ok(null));
    core.testHost.mockResolvedValueOnce(refused('auth_failed'));

    await actions.test(host('a', 'ready'));
    await actions.test(host('b', 'ready'));
    const outcomes = [actions.tests['id-a']?.status, actions.tests['id-b']?.status];
    actions.startAdd();
    actions.closeAll();

    expect(outcomes).toEqual(['passed', 'failed']);
    expect(actions.tests).toEqual({});
    expect(actions.editOpen).toBe(false);
  });
});
