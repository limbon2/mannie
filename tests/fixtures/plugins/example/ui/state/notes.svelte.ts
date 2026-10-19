// The plugin's notes: the notes that the plugin keeps while the window
// follows its channel, which it does while the plugin is on, and adding a
// note through the plugin's command. The channel brings only the notes kept
// from then on, so the list starts empty each time the plugin turns on.
import type { PluginContext } from '@mannie/plugin-sdk';
import { batchChannel, call, type ChannelMessage, type SubscriptionId, unsubscribe } from '@mannie/plugin-sdk/ipc';

import type { Note } from '../generated/bindings.js';
import { channels, commands } from '../generated/bindings.js';
import type { NotesFailure } from '../types/notes.js';

/** The ID of the plugin's settings section. */
export const SETTINGS_SECTION = 'example.settings';

/** The notes that the window follows. */
export class NoteList {
  /** Why the latest note was not added, if it was not. */
  addFailure: NotesFailure | null = $state(null);
  /** Whether the core is adding a note. */
  adding = $state(false);
  /** Why the notes could not be followed, if they could not. */
  failure: NotesFailure | null = $state(null);
  /** The notes, in the order the plugin kept them. */
  notes: Note[] = $state([]);
  status: 'error' | 'loading' | 'ready' = $state('loading');
  #following = false;
  /** Counts subscriptions, so that the notes of an ended one change nothing. */
  #generation = 0;
  #subscription: SubscriptionId | null = null;

  /** Adds a note, and returns whether the plugin kept it. */
  async add(text: string): Promise<boolean> {
    this.adding = true;
    this.addFailure = null;
    const outcome = await call(() => commands.addNote({ text }));
    this.adding = false;
    if (!outcome.ok) {
      this.addFailure = outcome.failure;
      return false;
    }
    this.#keep(outcome.data);
    return true;
  }

  /** Follows the notes while the plugin's context reports the plugin as on. Later calls change nothing. */
  follow(plugin: Pick<PluginContext, 'whileOn'>): void {
    if (this.#following) {
      return;
    }
    this.#following = true;
    plugin.whileOn(
      () => void this.#subscribe(),
      () => {
        this.#end();
      },
    );
  }

  /** Follows the notes again after a failure. */
  retry(): void {
    void this.#subscribe();
  }

  #apply(message: ChannelMessage<Note>): void {
    switch (message.kind) {
      case 'ended': {
        // A list that fell behind follows the notes again. When the plugin
        // stops, the list ends as the plugin's state leaves "on".
        if (message.reason === 'lagged') {
          void this.#subscribe();
        }
        return;
      }
      case 'item': {
        this.#keep(message.item);
      }
    }
  }

  /** Ends the subscription and forgets the notes, as the plugin stops. */
  #end(): void {
    this.#generation += 1;
    const previous = this.#subscription;
    this.#subscription = null;
    if (previous !== null) {
      void unsubscribe(previous);
    }
    this.notes = [];
    this.addFailure = null;
    this.failure = null;
    this.status = 'loading';
  }

  /** Adds a note that the command returned or the channel brought, once. */
  #keep(note: Note): void {
    const isKnown = this.notes.some((known) => known.id === note.id);
    if (!isKnown) {
      this.notes = [...this.notes, note];
    }
  }

  async #subscribe(): Promise<void> {
    this.#generation += 1;
    const generation = this.#generation;
    const previous = this.#subscription;
    this.#subscription = null;
    if (previous !== null) {
      void unsubscribe(previous);
    }
    const channel = batchChannel<ChannelMessage<Note>>(
      (items) => {
        if (generation !== this.#generation) {
          return;
        }
        for (const item of items) {
          this.#apply(item);
        }
      },
      () => {
        if (generation === this.#generation) {
          void this.#subscribe();
        }
      },
    );
    const outcome = await call(() => channels.followNotes(channel));
    if (generation !== this.#generation) {
      // A newer subscription replaced this one while it started.
      if (outcome.ok) {
        void unsubscribe(outcome.data);
      }
      return;
    }
    if (outcome.ok) {
      this.#subscription = outcome.data;
      this.status = 'ready';
      this.failure = null;
      return;
    }
    this.failure = outcome.failure;
    this.status = 'error';
  }
}

/** The window's notes. */
export const noteList = new NoteList();
