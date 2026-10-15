// What the shell itself registers in its slots: the interface settings
// section, the commands for the settings, the side panel and the theme, and
// the confirmation of links from model output, which every view that renders
// Markdown uses.
import PaletteIcon from '@lucide/svelte/icons/palette';
import { resetMode, setMode } from 'mode-watcher';

import LinkConfirmationDialog from '#lib/components/app/markdown/LinkConfirmationDialog.svelte';
import InterfaceSettingsSection from '#lib/components/app/shell/InterfaceSettingsSection.svelte';
import { m } from '#lib/paraglide/messages.js';
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';
import { sidePanels } from '#lib/state/side-panels.svelte.js';
import { openSettings } from '#lib/utils/settings-sections.js';

/** Registers the shell's own entries. */
export function registerShell(slots: ShellSlots): void {
  slots.settingsSections.register({
    component: InterfaceSettingsSection,
    description: m.shell_settings_interface_description,
    icon: PaletteIcon,
    id: 'shell.interface',
    order: 0,
    title: m.shell_settings_interface_title,
  });
  slots.overlays.register({ component: LinkConfirmationDialog, id: 'shell.link-confirmation' });
  slots.commands.register({
    group: m.shell_command_group_general,
    id: 'shell.open-settings',
    label: m.shell_command_open_settings,
    order: 0,
    run: () => void openSettings(),
  });
  slots.commands.register({
    available: () => sidePanels.state.shown !== null,
    group: m.shell_command_group_general,
    id: 'shell.close-side-panel',
    label: m.shell_command_close_side_panel,
    order: 1,
    run: () => void sidePanels.closeShown(),
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
