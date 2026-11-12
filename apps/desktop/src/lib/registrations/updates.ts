// Desktop updates (docs/features/updates.md, ADR-0063): the notices and the
// question above every view, the settings section and the palette's commands.
import CircleArrowUpIcon from '@lucide/svelte/icons/circle-arrow-up';

import UpdatesOverlay from '#lib/components/app/updates/UpdatesOverlay.svelte';
import UpdatesSettingsSection from '#lib/components/app/updates/UpdatesSettingsSection.svelte';
import { m } from '#lib/paraglide/messages.js';
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';
import { updates } from '#lib/state/updates.svelte.js';
import { checkForUpdatesInSettings, UPDATES_SETTINGS_SECTION } from '#lib/utils/updates-section.js';

/** Registers desktop updates. */
export function registerUpdates(slots: ShellSlots): void {
  slots.overlays.register({ component: UpdatesOverlay, id: 'updates.overlay' });
  slots.settingsSections.register({
    component: UpdatesSettingsSection,
    description: m.updates_settings_description,
    icon: CircleArrowUpIcon,
    id: UPDATES_SETTINGS_SECTION,
    order: 95,
    title: m.updates_settings_title,
  });
  slots.commands.register({
    available: () => updates.canCheck,
    group: m.updates_command_group,
    id: 'updates.check',
    label: m.updates_check_now,
    order: 0,
    run: checkForUpdatesInSettings,
  });
  slots.commands.register({
    available: () => updates.canRestart,
    group: m.updates_command_group,
    id: 'updates.restart',
    label: m.updates_restart_and_update,
    order: 10,
    run: () => void updates.restart(),
  });
}
