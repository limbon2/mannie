// The owner's notebooks, as the plugin's channel sends them while the plugin
// is on (plugins/notebooks/docs/README.md, contracts): every notebook with its
// sources and their states, the newest first, and again after each change.
// The window follows the channel through the plugin's context, so the list
// follows only while the plugin is on (ADR-0030, decision 11).
import type { PluginContext } from '@mannie/plugin-sdk';
import { batchChannel, call, type ChannelMessage, type SubscriptionId, unsubscribe } from '@mannie/plugin-sdk/ipc';

import { channels, type NotebookList, type NotebookView, type SourceView } from '../generated/bindings.js';
import type { NotebooksFailure } from '../types/notebooks.js';

/** A notebook with one of its sources replaced, or the notebook as it is when the source is not its own. */
function withSource(notebook: NotebookView, source: SourceView): NotebookView {
  if (notebook.id !== source.notebookId) {
    return notebook;
  }
  const sources = notebook.sources.map((known) => (known.id === source.id ? source : known));
  return { ...notebook, sources };
}

/** The notebook that changed last of the two, the earlier one when they changed at once. */
function newer(latest: NotebookView | undefined, notebook: NotebookView): NotebookView {
  if (latest === undefined) {
    return notebook;
  }
  return notebook.updatedAt > latest.updatedAt ? notebook : latest;
}

/** The notebooks that the window follows. */
export class NotebookLibrary {
  /** Why the notebooks could not be followed, if they could not. */
  failure: NotebooksFailure | null = $state(null);
  /** The notebooks, the newest first. */
  notebooks: NotebookView[] = $state([]);
  /** `ready` once the channel brought the first list. */
  status: 'error' | 'loading' | 'ready' = $state('loading');
  #following = false;
  /** Counts subscriptions, so that the lists of an ended one change nothing. */
  #generation = 0;
  #subscription: SubscriptionId | null = null;

  /** The notebook with this ID, if the list has it. */
  find(id: string): NotebookView | undefined {
    return this.notebooks.find((notebook) => notebook.id === id);
  }

  /** The source with this ID, if a notebook of the list has it. */
  findSource(id: string): SourceView | undefined {
    for (const notebook of this.notebooks) {
      const source = notebook.sources.find((candidate) => candidate.id === id);
      if (source !== undefined) {
        return source;
      }
    }
    return undefined;
  }

  /** The most recently changed notebook, which the view without a notebook leads to, or `undefined` without any. */
  latest(): NotebookView | undefined {
    return this.notebooks.reduce<NotebookView | undefined>(newer, undefined);
  }

  /** Follows the notebooks while the plugin's context reports the plugin as on. Later calls change nothing. */
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

  /** Adds a notebook that a command returned, until the channel brings it. */
  keep(notebook: NotebookView): void {
    if (this.find(notebook.id) === undefined) {
      this.notebooks = [notebook, ...this.notebooks];
    }
  }

  /** Shows a source as a command returned it, until the channel brings the next list. */
  replaceSource(source: SourceView): void {
    this.notebooks = this.notebooks.map((notebook) => withSource(notebook, source));
  }

  /** Follows the notebooks again after a failure. */
  retry(): void {
    void this.#subscribe();
  }

  #apply(message: ChannelMessage<NotebookList>): void {
    switch (message.kind) {
      case 'ended': {
        // A list that fell behind follows the notebooks again. When the
        // plugin stops, the list ends as the plugin's state leaves "on".
        if (message.reason === 'lagged') {
          void this.#subscribe();
        }
        return;
      }
      case 'item': {
        this.notebooks = message.item.notebooks;
        this.status = 'ready';
      }
    }
  }

  /** Ends the subscription and forgets the notebooks, as the plugin stops. */
  #end(): void {
    this.#generation += 1;
    const previous = this.#subscription;
    this.#subscription = null;
    if (previous !== null) {
      void unsubscribe(previous);
    }
    this.notebooks = [];
    this.failure = null;
    this.status = 'loading';
  }

  async #subscribe(): Promise<void> {
    this.#generation += 1;
    const generation = this.#generation;
    const previous = this.#subscription;
    this.#subscription = null;
    if (previous !== null) {
      void unsubscribe(previous);
    }
    // A list that follows again after it fell behind keeps showing what it has.
    if (this.status === 'error') {
      this.status = 'loading';
    }
    const channel = batchChannel<ChannelMessage<NotebookList>>(
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
    const outcome = await call(() => channels.followNotebooks(channel));
    if (generation !== this.#generation) {
      // A newer subscription replaced this one while it started.
      if (outcome.ok) {
        void unsubscribe(outcome.data);
      }
      return;
    }
    if (outcome.ok) {
      this.#subscription = outcome.data;
      this.failure = null;
      return;
    }
    this.failure = outcome.failure;
    this.status = 'error';
  }
}

/** The window's notebooks. */
export const notebookLibrary = new NotebookLibrary();
