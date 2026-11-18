import {
  batchChannel,
  call,
  type ChannelMessage,
  type PluginCallFailure,
  type SubscriptionId,
  unsubscribe,
} from '@mannie/plugin-sdk/ipc';
import { hostRefusalMessage, ipcFailureMessage } from '@mannie/plugin-sdk/messages';
import { SvelteMap } from 'svelte/reactivity';

import {
  channels,
  commands,
  type GraphicsEvent,
  type GraphicsPlacement,
  type ViewportError,
} from '../generated/bindings.js';
import { m } from '../generated/messages.js';

// Window-local edits survive navigation. They leave with the window.
const colors = new SvelteMap<string, number>();

// The host has one surface per plugin view. An older, slow open must finish (and release a
// late lease) before another project's open can reach the host and replace its surface.
let pendingOpen = Promise.resolve();

async function releaseSubscription(id: SubscriptionId): Promise<void> {
  try {
    await unsubscribe(id);
  } catch {
    // Reload or shutdown can disconnect IPC first. The host also releases this page's subscriptions.
  }
}

interface ViewportFailure {
  message: string;
  retry: boolean;
}

function graphicsFailure(code: ViewportError['code']): ViewportFailure {
  switch (code) {
    case 'unsupported_platform':
      return { message: m.designer_viewport_unsupported(), retry: false };
    case 'project_missing':
      return { message: m.designer_viewport_project_missing(), retry: false };
    case 'graphics_unavailable':
      return { message: m.designer_viewport_unavailable(), retry: true };
    case 'render_failed':
      return { message: m.designer_viewport_render_failed(), retry: true };
    case 'invalid_placement':
      return { message: m.designer_viewport_placement_failed(), retry: true };
    case 'stale_lease':
      return { message: m.designer_viewport_closed(), retry: true };
  }
}

function callFailure(failure: PluginCallFailure<ViewportError>): ViewportFailure {
  if (failure.kind === 'ipc') {
    return { message: ipcFailureMessage(), retry: true };
  }
  if (failure.error.origin === 'host') {
    return { message: hostRefusalMessage(failure.error, m.designer_plugin_name()), retry: true };
  }
  return graphicsFailure(failure.error.code);
}

/** The lifetime of a project's native surface is the lifetime of its status subscription. */
export class ProjectViewport {
  hue = $state(0);
  loading = $state(true);
  failure = $state.raw<ViewportFailure | null>(null);
  #generation = 0;
  #subscription: SubscriptionId | null = null;
  #lease: string | null = null;
  #placement: GraphicsPlacement | null = null;
  #sentPlacement: GraphicsPlacement | null = null;
  #sentHue: number | null = null;
  #updating = false;

  readonly projectId: string;

  constructor(projectId: string) {
    this.projectId = projectId;
    this.hue = colors.get(projectId) ?? 0;
  }

  changeHue(hue: number): void {
    this.hue = hue;
    colors.set(this.projectId, hue);
    void this.#update();
  }

  place(placement: GraphicsPlacement): void {
    this.#placement = placement;
    void this.#update();
  }

  /** Ends a pending open too. Its late subscription is released as soon as it arrives. */
  close(): void {
    this.#generation += 1;
    this.#lease = null;
    this.#updating = false;
    this.#sentPlacement = null;
    this.#sentHue = null;
    const subscription = this.#subscription;

    this.#subscription = null;
    if (subscription !== null) {
      const released = releaseSubscription(subscription);

      pendingOpen = Promise.all([pendingOpen, released]).then(() => undefined);
    }
  }

  open(): Promise<void> {
    this.close();
    this.loading = true;
    this.failure = null;
    const generation = this.#generation;

    pendingOpen = pendingOpen
      .then(() => this.#subscribe(generation))
      .catch(() => {
        if (this.#generation === generation) {
          this.#fail({ message: ipcFailureMessage(), retry: true });
        }
      });
    return pendingOpen;
  }

  async #subscribe(generation: number): Promise<void> {
    if (this.#generation !== generation) {
      return;
    }
    const channel = batchChannel<ChannelMessage<GraphicsEvent>>(
      (items) => {
        for (const message of items) {
          if (this.#generation !== generation) {
            return;
          }
          this.#receive(message);
        }
      },
      () => {
        if (this.#generation === generation) {
          void this.open();
        }
      },
    );
    const outcome = await call(() => channels.followViewport({ projectId: this.projectId }, channel));
    if (this.#generation !== generation) {
      if (outcome.ok) {
        // Unsubscribe before letting the next project open. A disconnected window also loses its leases.
        await releaseSubscription(outcome.data);
      }
      return;
    }
    if (outcome.ok) {
      this.#subscription = outcome.data;
    } else {
      this.#fail(callFailure(outcome.failure));
    }
  }

  #receive(message: ChannelMessage<GraphicsEvent>): void {
    if (message.kind === 'ended') {
      if (message.reason === 'lagged') {
        void this.open();
      } else {
        this.#fail(graphicsFailure('stale_lease'));
      }
      return;
    }
    if (message.item.kind === 'failed') {
      this.#fail(graphicsFailure(message.item.error.code));
      return;
    }
    this.#lease = message.item.lease;
    void this.#update();
  }

  #fail(failure: ViewportFailure): void {
    this.close();
    this.loading = false;
    this.failure = failure;
  }

  /** One in-flight command per lease, with the newest pending color and bounds. */
  async #update(): Promise<void> {
    const lease = this.#lease;
    const inactive = lease === null;
    if (inactive || this.#updating) {
      return;
    }
    this.#updating = true;
    const target = { lease, projectId: this.projectId };
    while (this.#lease === lease) {
      const placement = this.#placement;
      const hue = this.hue;
      const needPlacement = placement !== this.#sentPlacement;
      const needHue = hue !== this.#sentHue;
      // Hide promptly. On uncover or a new lease, apply the current color before presenting it.
      const placeFirst = !placement?.visible || !needHue;
      let update: () => ReturnType<typeof commands.placeViewport>;
      if (placement && needPlacement && placeFirst) {
        this.#sentPlacement = placement;
        update = () => commands.placeViewport({ ...target, placement });
      } else if (needHue) {
        this.#sentHue = hue;
        update = () => commands.setViewportColor({ ...target, hue });
      } else {
        this.loading = false;
        this.#updating = false;
        return;
      }
      const outcome = await call(update);
      if (this.#lease !== lease) {
        return;
      }
      if (!outcome.ok) {
        this.#fail(callFailure(outcome.failure));
        return;
      }
    }
  }
}
