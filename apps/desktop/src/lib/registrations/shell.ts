// What the shell itself registers in its slots: the interface settings
// section and the commands for the settings and the theme.
import { resetMode, setMode } from 'mode-watcher';

import { goto } from '$app/navigation';
import { resolve } from '$app/paths';

import InterfaceSettingsSection from '#lib/components/app/shell/InterfaceSettingsSection.svelte';
import { m } from '#lib/paraglide/messages.js';
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';

/** Registers the shell's own entries. */
export function registerShell(slots: ShellSlots): void {
  slots.settingsSections.register({
    component: InterfaceSettingsSection,
    description: m.shell_settings_interface_description,
    id: 'shell.interface',
    order: 0,
    title: m.shell_settings_interface_title,
  });
  slots.commands.register({
    group: m.shell_command_group_general,
    id: 'shell.open-settings',
    label: m.shell_command_open_settings,
    order: 0,
    run: () => void goto(resolve('/settings')),
  });
  slots.commands.register({
    group: m.shell_command_group_appearance,
    id: 'shell.theme-light',
    label: m.shell_command_theme_light,
    order: 0,
    run: () => setMode('light'),
  });
  slots.commands.register({
    group: m.shell_command_group_appearance,
    id: 'shell.theme-dark',
    label: m.shell_command_theme_dark,
    order: 1,
    run: () => setMode('dark'),
  });
  slots.commands.register({
    group: m.shell_command_group_appearance,
    id: 'shell.theme-system',
    label: m.shell_command_theme_system,
    order: 2,
    run: resetMode,
  });
}
