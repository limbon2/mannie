// The window settings as the settings view shows them (docs/features/
// window-and-tray.md, setting): what closing the main window does.
import { type CloseAction, commands, type WindowError } from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';
import { call } from '#lib/utils/calls.js';

/** The window settings and the outcome of the latest request. */
export class WindowSettingsState {
  /** What closing the main window does, once read. */
  closeAction: CloseAction | null = $state(null);
  /** Why the settings could not be read, if they could not. */
  readFailure: Failure<WindowError> | null = $state(null);
  /** Why the latest change failed, if it failed. */
  saveFailure: Failure<WindowError> | null = $state(null);
  /** Whether a change is being saved. */
  saving = $state(false);

  /** Changes what closing the window does. */
  async changeCloseAction(closeAction: CloseAction): Promise<void> {
    this.saving = true;
    this.saveFailure = null;
    const outcome = await call(() => commands.updateWindowSettings({ closeAction }));
    this.saving = false;
    if (outcome.ok) {
      this.closeAction = outcome.data.closeAction;
    } else {
      this.saveFailure = outcome.failure;
    }
  }

  /** Reads the settings from the core. */
  async load(): Promise<void> {
    this.readFailure = null;
    const outcome = await call(() => commands.getWindowSettings());
    if (outcome.ok) {
      this.closeAction = outcome.data.closeAction;
    } else {
      this.readFailure = outcome.failure;
    }
  }
}

/** The window's window settings. */
export const windowSettings = new WindowSettingsState();
