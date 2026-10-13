// The interface settings as the UI knows them: the locale the owner chose,
// and the state of reading and changing it.
import { commands } from '#lib/generated/bindings.js';
import type { SettingsFailure } from '#lib/types/interface-settings.js';

import { useStoredLocale } from './locale.js';

/** The interface settings and the outcome of the latest request. */
export class InterfaceSettings {
  /** The locale the owner chose, or `null` to follow the system. */
  chosenLocale: string | null = $state(null);
  /** Why the settings could not be read at startup, if they could not. */
  readFailure: SettingsFailure | null = $state(null);
  /** Why the latest change failed, if it failed. */
  saveFailure: SettingsFailure | null = $state(null);
  /** Whether a change is being saved. */
  saving = $state(false);

  /**
   * Reads the settings from the core and applies their locale. When the core
   * cannot read them, the window follows the system's language and the
   * settings view says why. It throws when the core does not answer at all.
   */
  async load(): Promise<void> {
    const result = await commands.getInterfaceSettings();
    if (result.status === 'error') {
      this.readFailure = { error: result.error, kind: 'core' };
      useStoredLocale(null);
      return;
    }
    this.chosenLocale = result.data.locale;
    useStoredLocale(result.data.locale);
  }

  /**
   * Stores a new locale, or `null` to follow the system, and reloads the
   * window so that it shows the new language (ADR-0019). The core's work
   * continues across the reload.
   */
  async changeLocale(locale: string | null): Promise<void> {
    this.saving = true;
    this.saveFailure = null;
    try {
      const result = await commands.updateInterfaceSettings({ locale });
      if (result.status === 'error') {
        this.saveFailure = { error: result.error, kind: 'core' };
        return;
      }
      window.location.reload();
    } catch {
      this.saveFailure = { kind: 'ipc' };
    } finally {
      this.saving = false;
    }
  }
}

/** The window's interface settings. */
export const interfaceSettings = new InterfaceSettings();
