// The owner's projects, as the plugin's channel sends them while the plugin is
// on (plugins/designer/docs/README.md, contracts): every project, the newest
// first, and again after each change. The window follows the channel through
// the plugin's context, so the list follows only while the plugin is on
// (ADR-0030, decision 11).
import type { PluginContext } from '@mannie/plugin-sdk';
import { batchChannel, call, type ChannelMessage, type SubscriptionId, unsubscribe } from '@mannie/plugin-sdk/ipc';

import { channels, type ProjectList, type ProjectView } from '../generated/bindings.js';
import type { DesignerFailure } from '../types/designer.js';

/** The projects that the window follows. */
export class ProjectLibrary {
  /** Why the projects could not be followed, if they could not. */
  failure: DesignerFailure | null = $state(null);
  /** The projects, the newest first. */
  projects: ProjectView[] = $state([]);
  /** `ready` once the channel brought the first list. */
  status: 'error' | 'loading' | 'ready' = $state('loading');
  #following = false;
  /** Counts subscriptions, so that the lists of an ended one change nothing. */
  #generation = 0;
  #subscription: SubscriptionId | null = null;

  /** The project with this ID, if the list has it. */
  find(id: string): ProjectView | undefined {
    return this.projects.find((project) => project.id === id);
  }

  /** The newest project, which the view shows while the owner has selected none, or `undefined` without any. */
  latest(): ProjectView | undefined {
    return this.projects.at(0);
  }

  /** Follows the projects while the plugin's context reports the plugin as on. Later calls change nothing. */
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

  /** Adds a project that a command returned, until the channel brings it. */
  keep(project: ProjectView): void {
    if (this.find(project.id) === undefined) {
      this.projects = [project, ...this.projects];
    }
  }

  /** Follows the projects again after a failure. */
  retry(): void {
    void this.#subscribe();
  }

  #apply(message: ChannelMessage<ProjectList>): void {
    switch (message.kind) {
      case 'ended': {
        // A list that fell behind follows the projects again. When the plugin
        // stops, the list ends as the plugin's state leaves "on".
        if (message.reason === 'lagged') {
          void this.#subscribe();
        }
        return;
      }
      case 'item': {
        this.projects = message.item.projects;
        this.status = 'ready';
      }
    }
  }

  /** Ends the subscription and forgets the projects, as the plugin stops. */
  #end(): void {
    this.#generation += 1;
    const previous = this.#subscription;
    this.#subscription = null;
    if (previous !== null) {
      void unsubscribe(previous);
    }
    this.projects = [];
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
    const channel = batchChannel<ChannelMessage<ProjectList>>(
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
    const outcome = await call(() => channels.followProjects(channel));
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

/** The window's projects. */
export const projectLibrary = new ProjectLibrary();
