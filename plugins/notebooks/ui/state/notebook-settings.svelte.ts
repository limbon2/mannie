// The Notebooks settings (plugins/notebooks/docs/README.md, the models): how
// the plugin searches, with the owner's search profiles to choose from,
// which the section reads as it opens and saves at the owner's request.
import { call } from '@mannie/plugin-sdk/ipc';

import { commands, type Preferences, type SearchProfiles } from '../generated/bindings.js';
import type { NotebooksFailure } from '../types/notebooks.js';

/** What the settings section shows and saves. */
export class NotebookSettings {
  /** How the plugin searches, once it was read. */
  preferences: Preferences | null = $state(null);
  /** The owner's search profiles, once they were read. */
  profiles: SearchProfiles | null = $state(null);
  /** Why the settings could not be read, if they could not. */
  readFailure: NotebooksFailure | null = $state(null);
  /** Whether the latest save kept the settings. */
  saved = $state(false);
  /** Why the latest save failed. */
  saveFailure: NotebooksFailure | null = $state(null);
  saving = $state(false);
  status: 'error' | 'loading' | 'ready' = $state('loading');

  /** Reads the preferences and the search profiles. */
  async load(): Promise<void> {
    this.status = 'loading';
    this.readFailure = null;
    this.saveFailure = null;
    this.saved = false;
    const [preferences, profiles] = await Promise.all([
      call(() => commands.readPreferences()),
      call(() => commands.listSearchProfiles()),
    ]);
    if (!preferences.ok) {
      this.readFailure = preferences.failure;
      this.status = 'error';
      return;
    }
    if (!profiles.ok) {
      this.readFailure = profiles.failure;
      this.status = 'error';
      return;
    }
    this.preferences = preferences.data;
    this.profiles = profiles.data;
    this.status = 'ready';
  }

  /** Saves how the plugin searches, and returns whether the plugin kept it. */
  async save(preferences: Preferences): Promise<boolean> {
    this.saving = true;
    this.saveFailure = null;
    this.saved = false;
    const outcome = await call(() => commands.savePreferences(preferences));
    this.saving = false;
    if (!outcome.ok) {
      this.saveFailure = outcome.failure;
      return false;
    }
    this.preferences = outcome.data;
    this.saved = true;
    return true;
  }
}

/** The settings of the window's section. */
export const notebookSettings = new NotebookSettings();
