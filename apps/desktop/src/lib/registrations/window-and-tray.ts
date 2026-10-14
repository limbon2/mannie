// The window and tray feature's entries in the shell's slots (docs/features/
// window-and-tray.md; ADR-0020): the overlay with the core's questions about
// closing and quitting, which also gives the tray its labels, and the
// settings section for what closing the window does.
import AppWindowIcon from '@lucide/svelte/icons/app-window';

import WindowBehaviorSection from '#lib/components/app/window-and-tray/WindowBehaviorSection.svelte';
import WindowRequestsOverlay from '#lib/components/app/window-and-tray/WindowRequestsOverlay.svelte';
import { m } from '#lib/paraglide/messages.js';
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';

/** Registers the window and tray feature. */
export function registerWindowAndTray(slots: ShellSlots): void {
  slots.overlays.register({ component: WindowRequestsOverlay, id: 'window.requests' });
  slots.settingsSections.register({
    component: WindowBehaviorSection,
    description: m.window_settings_description,
    icon: AppWindowIcon,
    id: 'window.behavior',
    order: 30,
    title: m.window_settings_title,
  });
}
