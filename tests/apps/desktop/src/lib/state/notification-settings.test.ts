// Notification settings state (docs/features/notifications.md; ADR-0074):
// loads the settings, saves individual toggle changes and reports read and
// save failures without losing the loaded values.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { UpdateNotificationSettings } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { NotificationSettingsState } from '../../../../../../apps/desktop/src/lib/state/notification-settings.svelte.js';

const DEFAULT_SETTINGS: UpdateNotificationSettings = {
  enabled: true,
  onApproval: true,
  onCompletion: true,
  onFailure: true,
  onQuestion: true,
  taskbarBadge: true,
  trayBadge: true,
};

const core = vi.hoisted(() => ({
  getNotificationSettings: vi.fn(),
  updateNotificationSettings: vi.fn(),
}));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({
  commands: {
    getNotificationSettings: core.getNotificationSettings,
    updateNotificationSettings: core.updateNotificationSettings,
  },
}));

beforeEach(() => {
  core.getNotificationSettings.mockReset().mockResolvedValue({ data: DEFAULT_SETTINGS, status: 'ok' });
  core.updateNotificationSettings
    .mockReset()
    .mockImplementation((next: UpdateNotificationSettings) => Promise.resolve({ data: next, status: 'ok' }));
});

describe('NotificationSettingsState', () => {
  it('loads notification settings from the core and fills omitted defaults with true', async () => {
    core.getNotificationSettings.mockResolvedValue({ data: { onFailure: false }, status: 'ok' });
    const state = new NotificationSettingsState();

    await state.load();

    expect(state.settings).toEqual({ ...DEFAULT_SETTINGS, onFailure: false });
    expect(state.readFailure).toBeNull();
  });

  it('records a read failure when loading settings fails', async () => {
    core.getNotificationSettings.mockResolvedValue({ error: { code: 'store_unavailable' }, status: 'error' });
    const state = new NotificationSettingsState();

    await state.load();

    expect(state.settings).toBeNull();
    expect(state.readFailure).toEqual({ error: { code: 'store_unavailable' }, kind: 'core' });
  });

  it('saves a changed switch and ignores unchanged values or calls before loading', async () => {
    const state = new NotificationSettingsState();

    await state.setToggle('trayBadge', false);
    expect(core.updateNotificationSettings).not.toHaveBeenCalled();

    await state.load();
    await state.setToggle('trayBadge', true);
    expect(core.updateNotificationSettings).not.toHaveBeenCalled();

    await state.setToggle('trayBadge', false);
    expect(core.updateNotificationSettings).toHaveBeenCalledWith({ ...DEFAULT_SETTINGS, trayBadge: false });
    expect(state.settings?.trayBadge).toBe(false);
    expect(state.saveFailure).toBeNull();
    expect(state.saving).toBe(false);
  });

  it('keeps the previous settings and records the failure when saving fails', async () => {
    core.updateNotificationSettings.mockResolvedValue({ error: { code: 'storage_failed' }, status: 'error' });
    const state = new NotificationSettingsState();
    await state.load();

    await state.setToggle('enabled', false);

    expect(state.settings?.enabled).toBe(true);
    expect(state.saveFailure).toEqual({ error: { code: 'storage_failed' }, kind: 'core' });
    expect(state.saving).toBe(false);
  });
});
