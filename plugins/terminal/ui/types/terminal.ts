// The types of the terminal plugin's UI.
import type { PluginCallFailure } from '@mannie/plugin-sdk/ipc';

import type { TerminalError } from '../generated/bindings.js';

/** Why a call of the plugin failed: the plugin's error, the host's refusal or no answer. */
export type TerminalFailure = PluginCallFailure<TerminalError>;

/** A terminal of a conversation, as the panel's argument names it. */
export interface TerminalAddress {
  readonly conversationId: string;
  readonly terminal: number;
}

/** Lines of a command that a citation or a reveal names, numbered from 1. */
export interface RevealedLines {
  readonly first: number;
  readonly last: number;
}

/** A command that a panel scrolls to and marks once it shows. */
export interface Reveal {
  /** The command's number in its terminal. */
  readonly block: number;
  /** The lines of its output to mark, or `null` for all of them. */
  readonly lines: RevealedLines | null;
}
