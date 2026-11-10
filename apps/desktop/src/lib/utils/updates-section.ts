// The way to the settings section of updates (docs/features/updates.md).
import { updates } from '#lib/state/updates.svelte.js';
import { openSettings } from '#lib/utils/settings-sections.js';

/** The settings section of updates. */
export const UPDATES_SETTINGS_SECTION = 'updates.settings';

/** Starts a check and opens the section, which shows the check and what it finds. */
export function checkForUpdatesInSettings(): void {
  void openSettings(UPDATES_SETTINGS_SECTION);
  void updates.check();
}
