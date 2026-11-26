// Unit tests for ProxmoxActions console opening and overlay error state.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ProxmoxActions } from '../../../ui/state/proxmox-actions.svelte.js';

const bindings = vi.hoisted(() => ({ openVncSession: vi.fn() }));
const navigation = vi.hoisted(() => ({
  openConversationId: vi.fn<() => string | null>(() => 'conv-1'),
  openSidePanel: vi.fn(() => Promise.resolve()),
  setSidePanelLabel: vi.fn(),
}));

vi.mock('../../../ui/generated/bindings.js', () => ({ commands: bindings }));
vi.mock('@mannie/plugin-sdk/navigation', () => navigation);

describe('ProxmoxActions.openConsole', () => {
  beforeEach(() => {
    bindings.openVncSession.mockReset();
    navigation.openConversationId.mockReset();
    navigation.openConversationId.mockReturnValue('conv-1');
    navigation.openSidePanel.mockReset();
    navigation.openSidePanel.mockResolvedValue(undefined);
  });

  it('opens the live VNC side panel with the menu opener on success', async () => {
    bindings.openVncSession.mockResolvedValue({ data: {}, status: 'ok' });
    const actions = new ProxmoxActions();
    const opener = {} as HTMLElement;

    await actions.openConsole('conv-1', 'srv-1', 100, 'debian-lab', opener);

    expect(bindings.openVncSession).toHaveBeenCalledWith({
      conversationId: 'conv-1',
      serverId: 'srv-1',
      vmid: 100,
    });
    expect(navigation.openSidePanel).toHaveBeenCalledWith('proxmox.vnc', 'conv-1::srv-1::100', {
      label: { isContent: true, text: 'debian-lab (100)' },
      opener,
    });
    expect(actions.consoleFailure).toBeNull();
    expect(actions.consoleFailureOpen).toBe(false);
  });

  it('records the failure for the overlay dialog when still on the same conversation', async () => {
    bindings.openVncSession.mockResolvedValue({
      error: { code: 'vnc_failed', description: 'refused', origin: 'plugin' },
      status: 'error',
    });
    const actions = new ProxmoxActions();

    await actions.openConsole('conv-1', 'srv-1', 100, 'debian-lab', null);

    expect(navigation.openSidePanel).not.toHaveBeenCalled();
    expect(actions.consoleFailureOpen).toBe(true);
    expect(actions.consoleFailure).toEqual({
      error: { code: 'vnc_failed', description: 'refused', origin: 'plugin' },
      kind: 'core',
    });

    actions.closeAll();
    expect(actions.consoleFailureOpen).toBe(false);
    expect(actions.consoleFailure).toBeNull();
  });

  it('drops a failure when the owner moved to another conversation', async () => {
    navigation.openConversationId.mockReturnValue('conv-2');
    bindings.openVncSession.mockResolvedValue({
      error: { code: 'vnc_failed', description: 'refused', origin: 'plugin' },
      status: 'error',
    });
    const actions = new ProxmoxActions();

    await actions.openConsole('conv-1', 'srv-1', 100, 'debian-lab', null);

    expect(actions.consoleFailureOpen).toBe(false);
    expect(actions.consoleFailure).toBeNull();
  });
});
