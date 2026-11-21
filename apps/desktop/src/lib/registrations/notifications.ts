// The notifications and tray badge feature's entries in the shell's slots
// (docs/features/notifications.md; ADR-0074): the root bridge overlay and the
// Notifications settings section.
import BellIcon from '@lucide/svelte/icons/bell';

import NotificationsBridge from '#lib/components/app/notifications/NotificationsBridge.svelte';
import NotificationsSettingsSection from '#lib/components/app/notifications/NotificationsSettingsSection.svelte';
import { m } from '#lib/paraglide/messages.js';
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';

/** The ID of the Notifications settings section. */
export const NOTIFICATIONS_SETTINGS_SECTION = 'notifications.settings';

/** Registers the notifications and tray badge feature. */
export function registerNotifications(slots: ShellSlots): void {
  slots.overlays.register({ component: NotificationsBridge, id: 'notifications.bridge' });
  slots.settingsSections.register({
    component: NotificationsSettingsSection,
    description: m.notifications_settings_description,
    icon: BellIcon,
    id: NOTIFICATIONS_SETTINGS_SECTION,
    order: 35,
    title: m.notifications_settings_title,
  });
}
