// The count of the notes that the plugin kept since it started, which the
// plugin's optional feature `counts` answers while the owner keeps it on
// (ADR-0055). The window reads the count when the feature turns on and again
// on request, and forgets it when the feature turns off.
import type { FeatureContext } from '@mannie/plugin-sdk';
import { call } from '@mannie/plugin-sdk/ipc';

import { commands } from '../generated/bindings.js';
import type { NotesFailure } from '../types/notes.js';

/** The ID of the plugin's optional feature that counts the notes. */
export const COUNTS_FEATURE = 'counts';

/** The ID of the feature's settings section. */
export const COUNTS_SECTION = 'example.counts';

/** The count of the notes, while the feature is on. */
export class NoteCount {
  /** How many notes the plugin kept since it started, once read. */
  count: number | null = $state(null);
  /** Why the count could not be read, if it could not. */
  failure: NotesFailure | null = $state(null);
  status: 'error' | 'loading' | 'ready' = $state('loading');
  #following = false;
  /** Counts the reads and the turns off, so that an older read's answer changes nothing. */
  #generation = 0;

  /** Reads the count while the feature's context reports the feature as on. Later calls change nothing. */
  follow(feature: Pick<FeatureContext, 'whileOn'>): void {
    if (this.#following) {
      return;
    }
    this.#following = true;
    feature.whileOn(
      () => void this.refresh(),
      () => {
        this.#forget();
      },
    );
  }

  /** Reads the count again. */
  async refresh(): Promise<void> {
    this.#generation += 1;
    const generation = this.#generation;
    this.status = 'loading';
    const outcome = await call(() => commands.countNotes());
    if (generation !== this.#generation) {
      return;
    }
    if (outcome.ok) {
      this.count = outcome.data;
      this.failure = null;
      this.status = 'ready';
      return;
    }
    this.failure = outcome.failure;
    this.status = 'error';
  }

  /** Forgets the count, as the feature turns off. */
  #forget(): void {
    this.#generation += 1;
    this.count = null;
    this.failure = null;
    this.status = 'loading';
  }
}

/** The window's count of the notes. */
export const noteCount = new NoteCount();
