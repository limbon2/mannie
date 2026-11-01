// What the window knows of the plugin's terminals (plugins/terminal/docs/
// README.md): where terminals open, the terminals of each conversation that a
// control shows, the settings, and the commands that panels scroll to. The
// hosts and the settings follow the plugin while it is on (ADR-0030,
// decision 11); a conversation's terminals are followed while a component
// shows them.
import type { PluginContext } from '@mannie/plugin-sdk';
import { projectFolder } from '@mannie/plugin-sdk/conversations';
import { call } from '@mannie/plugin-sdk/ipc';
import { openSidePanel } from '@mannie/plugin-sdk/navigation';
import { untrack } from 'svelte';
import { SvelteMap } from 'svelte/reactivity';

import {
  channels,
  commands,
  type HostChoice,
  type HostsView,
  type TerminalSettings,
  type TerminalSummary,
} from '../generated/bindings.js';
import type { Reveal, TerminalFailure } from '../types/terminal.js';
import { panelArgument, TERMINAL_PANEL } from '../utils/terminal-panel.js';

import { Followed } from './followed.svelte.js';

/** The settings until the plugin's are read. */
const DEFAULT_SETTINGS: TerminalSettings = {
  cursorBlink: false,
  defaultShell: null,
  fontSize: 13,
  renderer: 'automatic',
  scrollback: 2000,
  screenReader: false,
};

/** The size of a terminal before its view measures itself. */
const FIRST_SIZE = { columns: 120, rows: 30 };

/** The terminals of one conversation, while a component shows them. */
class ConversationTerminals {
  terminals: TerminalSummary[] = $state([]);
  readonly followed: Followed<{ terminals: TerminalSummary[] }>;
  /** How many components show the list. */
  watchers = 0;

  constructor(conversationId: string) {
    this.followed = new Followed(
      (channel) => channels.followTerminals({ conversationId }, channel),
      (update) => {
        this.terminals = update.terminals;
      },
    );
  }
}

/** The plugin's terminals, as the window knows them. */
export class TerminalsState {
  /** Where terminals open, once the core said. */
  hosts: HostsView | null = $state(null);
  /** The settings, the defaults until they are read. */
  settings: TerminalSettings = $state(DEFAULT_SETTINGS);
  /** Why the settings could not be read or stored, if they could not. */
  settingsFailure: TerminalFailure | null = $state(null);
  readonly #conversations = new SvelteMap<string, ConversationTerminals>();
  #following = false;
  readonly #hosts = new Followed<HostsView>(
    (channel) => channels.followHosts(channel),
    (hosts) => {
      this.hosts = hosts;
    },
  );
  /** The commands that panels scroll to once they show, by panel argument. */
  readonly #reveals = new SvelteMap<string, Reveal>();

  /** Follows the hosts and reads the settings while the plugin is on. Later calls change nothing. */
  follow(plugin: Pick<PluginContext, 'whileOn'>): void {
    if (this.#following) {
      return;
    }
    this.#following = true;
    plugin.whileOn(
      () => {
        this.#hosts.start();
        void this.loadSettings();
        for (const conversation of this.#conversations.values()) {
          conversation.followed.start();
        }
      },
      () => {
        this.#hosts.stop();
        this.hosts = null;
        for (const conversation of this.#conversations.values()) {
          conversation.followed.stop();
          conversation.terminals = [];
        }
      },
    );
  }

  /** A conversation's terminals, by number, while a component watches them. */
  of(conversationId: string): readonly TerminalSummary[] {
    return this.#conversations.get(conversationId)?.terminals ?? [];
  }

  /**
   * Follows a conversation's terminals until the function returned is called.
   * Components call it from an effect, so it reads and changes the map
   * untracked: an effect that tracked the entry that it adds would run again,
   * drop the entry and follow the channel anew, without end.
   */
  watch(conversationId: string): () => void {
    return untrack(() => {
      let conversation = this.#conversations.get(conversationId);
      if (conversation === undefined) {
        conversation = new ConversationTerminals(conversationId);
        this.#conversations.set(conversationId, conversation);
      }
      conversation.watchers += 1;
      if (conversation.watchers === 1) {
        conversation.followed.start();
      }
      const watched = conversation;
      return () => {
        watched.watchers -= 1;
        if (watched.watchers === 0) {
          watched.followed.stop();
          this.#conversations.delete(conversationId);
        }
      };
    });
  }

  /** Reads the settings. */
  async loadSettings(): Promise<void> {
    const outcome = await call(() => commands.getSettings());
    if (outcome.ok) {
      this.settings = outcome.data;
      this.settingsFailure = null;
    } else {
      this.settingsFailure = outcome.failure;
    }
  }

  /** Stores settings, and says why it failed, or `null`. */
  async updateSettings(settings: TerminalSettings): Promise<TerminalFailure | null> {
    const outcome = await call(() => commands.updateSettings(settings));
    if (!outcome.ok) {
      this.settingsFailure = outcome.failure;
      return outcome.failure;
    }
    this.settings = outcome.data;
    this.settingsFailure = null;
    return null;
  }

  /**
   * Opens a terminal of a conversation where `host` says, in the
   * conversation's project folder on this computer, and shows its panel.
   * Resolves to why it failed, or `null`.
   */
  async open(conversationId: string, host: HostChoice, opener: HTMLElement | null): Promise<TerminalFailure | null> {
    const folder = host.kind === 'this_computer' ? projectFolder(conversationId) : null;
    const outcome = await call(() => commands.openTerminal({ conversationId, folder, host, ...FIRST_SIZE }));
    if (!outcome.ok) {
      return outcome.failure;
    }
    await openSidePanel(TERMINAL_PANEL, panelArgument({ conversationId, terminal: outcome.data.number }), {
      opener,
    });
    return null;
  }

  /** Shows a terminal's panel and scrolls it to a command once it shows. */
  async reveal(conversationId: string, terminal: number, reveal: Reveal): Promise<void> {
    const argument = panelArgument({ conversationId, terminal });
    this.#reveals.set(argument, reveal);
    await openSidePanel(TERMINAL_PANEL, argument);
  }

  /** Takes the command that a panel should scroll to, if one waits. */
  takeReveal(argument: string): Reveal | undefined {
    const reveal = this.#reveals.get(argument);
    this.#reveals.delete(argument);
    return reveal;
  }

  /** Whether a panel has a command to scroll to, which it follows to take it. */
  hasReveal(argument: string): boolean {
    return this.#reveals.has(argument);
  }
}

/** The window's terminals. */
export const terminals = new TerminalsState();
