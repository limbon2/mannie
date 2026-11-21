// Notification settings as the settings view shows them (docs/features/
// notifications.md; ADR-0074).
import {
  commands,
  type NotificationError,
  type NotificationSettings,
  type UpdateNotificationSettings,
} from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';
import { call } from '#lib/utils/calls.js';

/** The boolean switches of the notification settings section. */
export type NotificationToggleKey = keyof UpdateNotificationSettings;

/** Fills any omitted wire field with the documented default of `true`. */
function normalizeSettings(raw: NotificationSettings): UpdateNotificationSettings {
  return {
    enabled: raw.enabled ?? true,
    onApproval: raw.onApproval ?? true,
    onCompletion: raw.onCompletion ?? true,
    onFailure: raw.onFailure ?? true,
    onQuestion: raw.onQuestion ?? true,
    taskbarBadge: raw.taskbarBadge ?? true,
    trayBadge: raw.trayBadge ?? true,
  };
}

/** The notification settings and the outcome of the latest request. */
export class NotificationSettingsState {
  /** Why the settings could not be read, if they could not. */
  readFailure: Failure<NotificationError> | null = $state(null);
  /** Why the latest change failed, if it failed. */
  saveFailure: Failure<NotificationError> | null = $state(null);
  /** Whether a change is being saved. */
  saving = $state(false);
  /** The loaded settings, or `null` before they have been read. */
  settings: UpdateNotificationSettings | null = $state(null);

  /** Reads the settings from the core. */
  async load(): Promise<void> {
    this.readFailure = null;
    const outcome = await call(() => commands.getNotificationSettings());
    if (outcome.ok) {
      this.settings = normalizeSettings(outcome.data);
    } else {
      this.readFailure = outcome.failure;
    }
  }

  /** Changes one switch and saves the full settings document. */
  async setToggle(key: NotificationToggleKey, value: boolean): Promise<void> {
    if (this.settings === null) {
      return;
    }
    if (this.settings[key] === value) {
      return;
    }
    const next: UpdateNotificationSettings = { ...this.settings, [key]: value };

    this.saving = true;
    this.saveFailure = null;

    const outcome = await call(() => commands.updateNotificationSettings(next));

    this.saving = false;

    if (outcome.ok) {
      this.settings = normalizeSettings(outcome.data);
    } else {
      this.saveFailure = outcome.failure;
    }
  }
}

/** The window's notification settings. */
export const notificationSettings = new NotificationSettingsState();
