// The fixture plugin's UI (ADR-0035, decision 9): a settings section that
// adds notes and lists the notes that the plugin kept since it turned on, a
// group of the sidebar with the same notes, and a command that opens the
// section. Every entry shows only while the plugin is on, and the notes
// follow the plugin's channel only then (ADR-0030, decision 11). Its optional
// feature `counts` adds a settings section with the count of the notes,
// which shows only while the feature is on too (ADR-0055, decision 10). The
// core's tests use it in place of the plugins that ship.
import { definePlugin } from '@mannie/plugin-sdk';
import HashIcon from '@mannie/plugin-sdk/icons/hash';
import NotepadTextIcon from '@mannie/plugin-sdk/icons/notepad-text';
import { openSettings } from '@mannie/plugin-sdk/navigation';

import CountsSettingsSection from './components/CountsSettingsSection.svelte';
import NotesPanel from './components/NotesPanel.svelte';
import NotesSettingsSection from './components/NotesSettingsSection.svelte';
import { m } from './generated/messages.js';
import { COUNTS_FEATURE, COUNTS_SECTION, noteCount } from './state/counts.svelte.js';
import { noteList, SETTINGS_SECTION } from './state/notes.svelte.js';

export default definePlugin({
  register(context) {
    const { slots } = context;
    noteList.follow(context);
    slots.sidebarPanels.register({ component: NotesPanel, id: 'example.panel', order: 6 });
    slots.commands.register({
      group: m.example_command_group,
      id: 'example.add-note',
      label: m.example_command_add_note,
      order: 0,
      run: () => void openSettings(SETTINGS_SECTION),
    });
    slots.settingsSections.register({
      component: NotesSettingsSection,
      description: m.example_settings_description,
      icon: NotepadTextIcon,
      id: SETTINGS_SECTION,
      order: 80,
      title: m.example_settings_title,
    });
    const counts = context.feature(COUNTS_FEATURE);
    noteCount.follow(counts);
    counts.slots.settingsSections.register({
      component: CountsSettingsSection,
      description: m.example_counts_settings_description,
      icon: HashIcon,
      id: COUNTS_SECTION,
      order: 81,
      title: m.example_counts_settings_title,
    });
  },
});
