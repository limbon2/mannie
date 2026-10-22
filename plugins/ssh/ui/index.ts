// The SSH plugin's UI (plugins/ssh/docs/README.md; ADR-0036): the SSH
// hosts section of the settings with its dialogs, "Add SSH host…" in the
// command palette, the kind of mentions that names a host after "@", and
// the cards of its tools, which the core names by their accesses. Every
// entry shows only while the plugin is on, and the list of hosts follows the
// plugin's channel only then (ADR-0030, decision 11).
import { definePlugin } from '@mannie/plugin-sdk';
import ServerIcon from '@mannie/plugin-sdk/icons/server';
import { openSettings } from '@mannie/plugin-sdk/navigation';

import SshDialogs from './components/SshDialogs.svelte';
import SshSettingsSection from './components/SshSettingsSection.svelte';
import { m } from './generated/messages.js';
import { hostList } from './state/hosts.svelte.js';
import { SETTINGS_SECTION, sshActions } from './state/ssh-actions.svelte.js';
import {
  hostMentionRefusal,
  hostMentionSummary,
  hostMentionVariantName,
  hostMentionWarning,
  sshStartFailureMessage,
  sshToolFailure,
} from './utils/ssh-messages.js';

/** The plugin's tools, as its tool specs name them. */
const TOOLS = ['run_ssh_command', 'start_ssh_command'];

export default definePlugin({
  failure: sshStartFailureMessage,
  register(context) {
    const { slots } = context;
    hostList.follow(context);
    slots.overlays.register({ component: SshDialogs, id: 'ssh.dialogs' });
    slots.commands.register({
      group: m.ssh_command_group,
      id: 'ssh.add-host',
      label: m.ssh_command_add_host,
      order: 0,
      run: () => {
        void openSettings(SETTINGS_SECTION).then(() => {
          sshActions.startAdd();
        });
      },
    });
    slots.settingsSections.register({
      component: SshSettingsSection,
      description: m.ssh_settings_description,
      icon: ServerIcon,
      id: SETTINGS_SECTION,
      order: 30,
      title: m.ssh_settings_title,
    });
    for (const tool of TOOLS) {
      // The core names a call by its remote command; the plugin gives the
      // icon and words its failure codes.
      slots.toolKinds.register({ failure: sshToolFailure, icon: ServerIcon, id: tool, summary: () => null });
    }
    // The provider's ID is the plugin's.
    slots.mentionKinds.register({
      group: m.ssh_mention_group,
      icon: () => ServerIcon,
      id: context.id,
      order: 20,
      refusal: hostMentionRefusal,
      summary: hostMentionSummary,
      variantName: hostMentionVariantName,
      warning: hostMentionWarning,
    });
  },
});
