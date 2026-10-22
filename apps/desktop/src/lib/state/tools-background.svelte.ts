// What the panel of a background command's output follows (docs/features/
// system-tools/background-commands.md, interface): the command, its output
// as it arrives, kept to the last lines that the view holds, and the owner's
// Stop. After a gap in the channel it subscribes again from its last line.
import {
  type BackgroundCommandView,
  type CommandOutputUpdate,
  commands,
  type ConversationId,
  type SubscriptionId,
} from '#lib/generated/bindings.js';
import type { ToolsFailure } from '#lib/types/tools.js';
import { batchChannel, call } from '#lib/utils/calls.js';

/** The most lines that the panel holds; older ones leave the view. */
export const MAX_SHOWN_LINES = 2_000;

/** One entry of the shown output: a line, or the lines that the log let go. */
export type OutputEntry =
  | { readonly first: number; readonly kind: 'left_out'; readonly last: number }
  | { readonly kind: 'line'; readonly number: number; readonly text: string };

/** Adds an update's lines to the entries, keeping the last ones. */
export function withUpdate(entries: readonly OutputEntry[], update: CommandOutputUpdate): OutputEntry[] {
  let added: OutputEntry[];
  switch (update.kind) {
    case 'lines': {
      added = update.lines.map((text, index) => ({ kind: 'line', number: update.firstLine + index, text }));
      break;
    }
    case 'left_out': {
      added = [{ first: update.firstLine, kind: 'left_out', last: update.lastLine }];
      break;
    }
    case 'command':
    case 'ended': {
      return [...entries];
    }
  }
  const all = [...entries, ...added];
  return all.length > MAX_SHOWN_LINES ? all.slice(all.length - MAX_SHOWN_LINES) : all;
}

/** The last line that the entries hold, or `null` for none. */
export function lastLine(entries: readonly OutputEntry[]): number | null {
  const last = entries.at(-1);
  if (last === undefined) {
    return null;
  }
  return last.kind === 'line' ? last.number : last.last;
}

/** One panel's view of a command's output, from its first subscription until the panel closes. */
export class CommandOutputView {
  /** The command as the latest update showed it. */
  command: BackgroundCommandView | null = $state(null);
  readonly conversationId: ConversationId;
  /** The shown lines, the oldest first. */
  entries: OutputEntry[] = $state([]);
  /** Why the output could not be followed, if it could not. */
  failure: ToolsFailure | null = $state(null);
  readonly number: number;
  /** Whether the owner's Stop is on its way. */
  stopping = $state(false);
  /** Why the owner's latest Stop failed, if it failed. */
  stopFailure: ToolsFailure | null = $state(null);
  /** Whether the subscription has not delivered the command yet. */
  loading = $state(true);
  #closed = false;
  #generation = 0;
  #subscription: SubscriptionId | null = null;

  constructor(conversationId: ConversationId, number: number) {
    this.conversationId = conversationId;
    this.number = number;
  }

  /** Whether the command still runs. */
  get running(): boolean {
    const runs = this.command !== null && this.command.ending === null;
    return runs;
  }

  /** Follows the output, from the last line shown, or from the last lines of the output. */
  async follow(): Promise<void> {
    if (this.#closed) {
      return;
    }
    this.#generation += 1;
    const generation = this.#generation;
    this.#unsubscribe();
    this.failure = null;
    const channel = batchChannel<CommandOutputUpdate>(
      (updates) => {
        if (generation === this.#generation) {
          this.#apply(updates);
        }
      },
      () => {
        if (generation === this.#generation) {
          void this.follow();
        }
      },
    );
    const afterLine = lastLine(this.entries);
    const outcome = await call(() =>
      commands.subscribeCommandOutput({ afterLine, conversationId: this.conversationId, number: this.number }, channel),
    );
    if (generation !== this.#generation) {
      // A newer subscription replaced this one while it started.
      if (outcome.ok) {
        void commands.unsubscribe({ subscriptionId: outcome.data });
      }
      return;
    }
    if (outcome.ok) {
      this.#subscription = outcome.data;
      return;
    }
    this.loading = false;
    this.failure = outcome.failure;
  }

  /** Stops the command at the owner's request. */
  async stop(): Promise<void> {
    if (this.stopping) {
      return;
    }
    this.stopping = true;
    this.stopFailure = null;
    const outcome = await call(() =>
      commands.stopBackgroundCommand({ conversationId: this.conversationId, number: this.number }),
    );
    this.stopping = false;
    if (outcome.ok) {
      this.command = outcome.data;
      return;
    }
    this.stopFailure = outcome.failure;
  }

  /** Ends the view: the panel closed. */
  close(): void {
    this.#closed = true;
    this.#generation += 1;
    this.#unsubscribe();
  }

  #apply(updates: readonly CommandOutputUpdate[]): void {
    let entries = this.entries;
    for (const update of updates) {
      switch (update.kind) {
        case 'command':
        case 'ended': {
          this.command = update.command;
          this.loading = false;
          break;
        }
        case 'left_out':
        case 'lines': {
          entries = withUpdate(entries, update);
          break;
        }
      }
    }
    if (entries !== this.entries) {
      this.entries = entries;
    }
  }

  #unsubscribe(): void {
    const subscription = this.#subscription;
    this.#subscription = null;
    if (subscription !== null) {
      void commands.unsubscribe({ subscriptionId: subscription });
    }
  }
}

/**
 * Stops a command from the card of the call that started it, or from the
 * toolbar's menu, and returns why the core refused, if it did.
 */
export async function stopCommand(conversationId: ConversationId, number: number): Promise<ToolsFailure | null> {
  const outcome = await call(() => commands.stopBackgroundCommand({ conversationId, number }));
  return outcome.ok ? null : outcome.failure;
}
