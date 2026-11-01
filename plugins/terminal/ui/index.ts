// The terminal plugin's UI (plugins/terminal/docs/README.md; ADR-0053): the
// Terminal button in a conversation's toolbar, a terminal's side panel,
// "Open a terminal" in the command palette with Ctrl+`, the Terminal section
// of the settings, the cards of its tools, the kind of mentions that names
// commands after "@", and the kind of context whose citations show a
// command's output and open the terminal at it (ADR-0054, decision 3). Every
// entry shows only while the plugin is on (ADR-0030, decision 11).
import { definePlugin } from '@mannie/plugin-sdk';
import ListIcon from '@mannie/plugin-sdk/icons/list';
import SquareDashedTextIcon from '@mannie/plugin-sdk/icons/square-dashed-text';
import SquareTerminalIcon from '@mannie/plugin-sdk/icons/square-terminal';
import { openConversationId, openSidePanel } from '@mannie/plugin-sdk/navigation';
import type { Component } from 'svelte';

import RunInTerminalDetails from './components/RunInTerminalDetails.svelte';
import TerminalControl from './components/TerminalControl.svelte';
import TerminalPanel from './components/TerminalPanel.svelte';
import TerminalSettingsSection from './components/TerminalSettingsSection.svelte';
import { m } from './generated/messages.js';
import { terminals } from './state/terminals.svelte.js';
import {
  mentionRefusal,
  mentionSummary,
  mentionVariantName,
  runSummary,
  terminalOffQuestion,
  terminalStartFailureMessage,
  toolFailure,
} from './utils/terminal-messages.js';
import { panelArgument, parseCiteKey, SETTINGS_SECTION, TERMINAL_PANEL } from './utils/terminal-panel.js';

/** The icon of an entry of the "@" list, by its variant. */
function mentionIcon(variant: string): Component {
  return variant === 'selection' ? SquareDashedTextIcon : SquareTerminalIcon;
}

/** Opens the open conversation's latest terminal, or a new one on this computer. */
async function openTerminal(): Promise<void> {
  const conversationId = openConversationId();
  if (conversationId === null) {
    return;
  }
  const latest = terminals.of(conversationId).findLast((terminal) => terminal.phase !== 'ended');
  if (latest === undefined) {
    await terminals.open(conversationId, { kind: 'this_computer', shell: null }, null);
    return;
  }
  await openSidePanel(TERMINAL_PANEL, panelArgument({ conversationId, terminal: latest.number }));
}

/** Shows the command that a cite link names, in its terminal, while the terminal exists. */
function showCited(key: string): void {
  const conversationId = openConversationId();
  const cited = parseCiteKey(key);
  if (conversationId === null) {
    return;
  }
  if (cited === null) {
    return;
  }
  const terminal = terminals.of(conversationId).find((summary) => summary.session === cited.session);
  if (terminal !== undefined) {
    void terminals.reveal(conversationId, terminal.number, cited.reveal);
  }
}

export default definePlugin({
  failure: terminalStartFailureMessage,
  offQuestion: terminalOffQuestion,
  register(context) {
    const { slots } = context;
    terminals.follow(context);
    slots.sidePanels.register({
      component: TerminalPanel,
      defaultLabel: m.terminal_panel_title,
      icon: SquareTerminalIcon,
      id: TERMINAL_PANEL,
    });
    slots.conversationControls.register({ component: TerminalControl, id: 'terminal.control', order: 40 });
    slots.commands.register({
      available: () => openConversationId() !== null,
      group: m.terminal_command_group,
      id: 'terminal.open',
      key: { code: 'Backquote', label: m.terminal_key },
      label: m.terminal_command_open,
      order: 0,
      run: () => void openTerminal(),
    });
    slots.settingsSections.register({
      component: TerminalSettingsSection,
      description: m.terminal_settings_description,
      icon: SquareTerminalIcon,
      id: SETTINGS_SECTION,
      order: 70,
      title: m.terminal_settings_title,
    });
    slots.toolKinds.register({
      details: RunInTerminalDetails,
      failure: toolFailure,
      icon: SquareTerminalIcon,
      id: 'run_in_terminal',
      summary: runSummary,
    });
    slots.toolKinds.register({
      failure: () => null,
      icon: ListIcon,
      id: 'list_terminals',
      summary: () => m.terminal_tool_list(),
    });
    // The provider's ID is the plugin's.
    slots.mentionKinds.register({
      group: m.terminal_mention_group,
      icon: mentionIcon,
      id: context.id,
      order: 40,
      refusal: mentionRefusal,
      summary: mentionSummary,
      variantName: mentionVariantName,
      // The provider warns about no entry.
      warning: () => null,
    });
    // Citations only: no context provider has this ID (ADR-0054, decision 3).
    slots.contextKinds.register({
      failure: () => null,
      found: (_labels, count) => m.terminal_context_found({ count }),
      gathering: m.terminal_context_gathering,
      icon: SquareTerminalIcon,
      id: context.id,
      noun: m.terminal_context_noun,
      open: { label: m.terminal_show_in_terminal, run: showCited },
    });
  },
});
