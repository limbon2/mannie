// The child agents settings as the settings page shows them
// (docs/features/child-agents/child-agent-activity.md, settings): how many
// child agents may run at once. The core checks every change, applies it at
// once and returns what it stored.
import { type ChildAgentsSettings, commands } from '#lib/generated/bindings.js';
import type { ChildAgentsFailure } from '#lib/types/child-agents.js';
import { call } from '#lib/utils/calls.js';

/** The child agents settings and the outcome of the latest request. */
export class ChildAgentsSettingsState {
  /** Why the settings could not be read, if they could not. */
  readFailure: ChildAgentsFailure | null = $state(null);
  /** Whether the latest change was saved. */
  saved = $state(false);
  /** Why the latest change failed, if it failed. */
  saveFailure: ChildAgentsFailure | null = $state(null);
  /** Whether a change is being saved. */
  saving = $state(false);
  /** The settings, once read. */
  settings: ChildAgentsSettings | null = $state(null);

  /** Reads the settings from the core. */
  async load(): Promise<void> {
    this.readFailure = null;
    const outcome = await call(() => commands.getChildAgentsSettings());
    if (outcome.ok) {
      this.settings = outcome.data;
    } else {
      this.readFailure = outcome.failure;
    }
  }

  /** Changes how many child agents may run at once. */
  async setParallelChildren(limit: number): Promise<void> {
    this.saving = true;
    this.saved = false;
    this.saveFailure = null;
    const outcome = await call(() => commands.updateChildAgentsSettings({ parallelChildren: limit }));
    this.saving = false;
    if (outcome.ok) {
      this.settings = outcome.data;
      this.saved = true;
    } else {
      this.saveFailure = outcome.failure;
    }
  }
}

/** The window's child agents settings. */
export const childAgentsSettings = new ChildAgentsSettingsState();
