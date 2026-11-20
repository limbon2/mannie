import {
  batchChannel,
  call,
  type ChannelMessage,
  type PluginCallFailure,
  type SubscriptionId,
  unsubscribe,
} from '@mannie/plugin-sdk/ipc';
import { hostRefusalMessage, ipcFailureMessage } from '@mannie/plugin-sdk/messages';

import {
  channels,
  commands,
  type DocumentSnapshot,
  type GraphicsEvent,
  type GraphicsPlacement,
  type ViewportError,
} from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import { nextSelection, type SelectionInput } from '../utils/canvas-selection.js';

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

interface ViewportTarget {
  lease: string;
  projectId: string;
}

interface FailedEdit {
  elementId: string;
  hue: number;
  revision: number;
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
    case 'storage_failed':
      return { message: m.designer_document_read_failed(), retry: true };
    case 'stale_revision':
      return { message: m.designer_document_changed(), retry: true };
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
  documentError = $state<string | null>(null);
  confirmed = $state.raw<DocumentSnapshot | null>(null);
  selectedId = $state<string | null>(null);
  sliderGeneration = $state(0);
  #generation = 0;
  #subscription: SubscriptionId | null = null;
  #lease: string | null = null;
  #placement: GraphicsPlacement | null = null;
  #appliedPlacement: GraphicsPlacement | null = null;
  #sentHue: number | null = null;
  #wantedSelection: string | null = null;
  #visible = $state(false);
  #readRequested = false;
  #commits: number[] = [];
  #draft = false;
  #lastCommit: number | null = null;
  #retryEdit: FailedEdit | null = null;
  #updating = false;
  #committing = false;
  #updates = Promise.resolve();

  readonly projectId: string;

  constructor(projectId: string) {
    this.projectId = projectId;
  }

  changeHue(hue: number): void {
    if (!this.canEdit) {
      return;
    }
    this.#draft = true;
    this.hue = hue;
    this.#update();
  }

  get canEdit(): boolean {
    const ready = this.confirmed !== null && this.failure === null && this.documentError === null;
    return ready && !this.loading;
  }

  get canSelect(): boolean {
    return this.canEdit && this.#visible;
  }

  select(input: SelectionInput): void {
    if (!this.canSelect || !this.confirmed || !this.#placement) {
      return;
    }
    if (input.kind === 'point') {
      // A resize or scroll can move the DOM before the observer runs or before the host presents its crop.
      const { bounds } = this.#placement;
      const sameOrigin = input.bounds.x === bounds.x && input.bounds.y === bounds.y;
      const sameSize = input.bounds.width === bounds.width && input.bounds.height === bounds.height;
      const current = this.#appliedPlacement === this.#placement;
      if (!current || !sameOrigin || !sameSize) {
        return;
      }
    }
    this.#wantedSelection = nextSelection(this.#wantedSelection, input, this.confirmed.element, this.#placement.clip);
    this.#update();
  }

  commitHue(hue: number): void {
    if (!this.canEdit) {
      return;
    }
    this.#draft = false;
    this.hue = hue;
    this.#lastCommit = hue;
    this.#commits.push(hue);
    this.#update();
  }

  cancelPreview(): void {
    const document = this.confirmed;
    if (!this.#draft || !document) {
      return;
    }
    this.#draft = false;
    this.hue = this.#lastCommit ?? document.element.hue;
    // Release the slider's document-level drag listeners when its gesture is interrupted.
    this.sliderGeneration += 1;
    this.#update();
  }

  place(placement: GraphicsPlacement): void {
    const previous = this.#placement;

    this.#placement = placement;
    if (!placement.visible) {
      this.#visible = false;
    }
    const resized =
      previous?.bounds.width !== placement.bounds.width || previous.bounds.height !== placement.bounds.height;
    if (!placement.visible || resized) {
      this.cancelPreview();
    }
    this.#update();
  }

  retryDocument(): void {
    if (!this.documentError || !this.#lease) {
      return;
    }
    this.documentError = null;
    this.loading = true;
    this.#readRequested = true;
    this.#update();
  }

  /** A project switch cancels a draft, but finishes edits whose slider was already released. */
  dispose(): void {
    const editsPending = this.#committing || this.#commits.length > 0;
    if (!editsPending) {
      this.close();
      return;
    }
    if (this.#placement !== null) {
      this.place({ ...this.#placement, visible: false });
    }
    const generation = this.#generation;
    const finished = this.#updates.then(() => {
      if (generation === this.#generation) {
        return this.#release();
      }
    });

    pendingOpen = Promise.all([pendingOpen, finished]).then(() => undefined);
  }

  /** Ends a pending open too. Its late subscription is released as soon as it arrives. */
  close(): void {
    const released = this.#release();

    pendingOpen = Promise.all([pendingOpen, released]).then(() => undefined);
  }

  #release(): Promise<void> {
    this.#generation += 1;
    this.#lease = null;
    this.#updating = false;
    this.#committing = false;
    this.#appliedPlacement = null;
    this.#sentHue = null;
    this.#wantedSelection = null;
    this.selectedId = null;
    this.#visible = false;
    this.#readRequested = false;
    this.#commits = [];
    this.#lastCommit = null;
    this.#retryEdit = null;
    this.#draft = false;
    const subscription = this.#subscription;

    this.#subscription = null;
    return subscription === null ? Promise.resolve() : releaseSubscription(subscription);
  }

  open(): Promise<void> {
    this.close();
    this.loading = true;
    this.failure = null;
    this.documentError = null;
    this.confirmed = null;
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
    this.#readRequested = true;
    this.#update();
  }

  #fail(failure: ViewportFailure): void {
    this.close();
    this.loading = false;
    this.documentError = null;
    this.failure = failure;
  }

  #documentFailed(failure: PluginCallFailure<ViewportError>, edit: FailedEdit | null): void {
    const error = failure.kind === 'core' ? failure.error : null;
    if (error?.code === 'storage_failed') {
      this.confirmed = error.confirmed;
      this.documentError = edit === null ? m.designer_document_read_failed() : m.designer_document_save_failed();
      this.#retryEdit = edit ?? this.#retryEdit;
    } else if (error?.code === 'stale_revision') {
      this.documentError = m.designer_document_changed();
      this.#retryEdit = null;
    } else {
      this.#fail(callFailure(failure));
      return;
    }
    this.#commits = [];
    if (this.#draft) {
      this.sliderGeneration += 1;
    }
    this.#draft = false;
    this.#lastCommit = null;
    this.hue = this.confirmed?.element.hue ?? 0;
    this.#sentHue = this.hue;
    this.loading = false;
  }

  async #read(target: ViewportTarget): Promise<void> {
    this.#readRequested = false;
    const outcome = await call(() => commands.readViewportDocument(target));
    if (this.#lease !== target.lease) {
      return;
    }
    if (!outcome.ok) {
      this.#documentFailed(outcome.failure, null);
      return;
    }
    this.confirmed = outcome.data;
    if (this.selectedId !== outcome.data.element.id) {
      this.selectedId = null;
    }
    if (this.#wantedSelection !== outcome.data.element.id) {
      this.#wantedSelection = null;
    }
    this.hue = outcome.data.element.hue;
    this.#sentHue = this.hue;
    const retry = this.#retryEdit;

    this.#retryEdit = null;
    const stillCurrent = retry?.revision === outcome.data.revision && retry.elementId === outcome.data.element.id;
    if (stillCurrent) {
      this.#commits.push(retry.hue);
      this.hue = retry.hue;
    }
  }

  async #commit(target: ViewportTarget, document: DocumentSnapshot, hue: number): Promise<void> {
    if (hue === document.element.hue) {
      return;
    }
    const edit = { elementId: document.element.id, hue, revision: document.revision };
    const outcome = await call(() => commands.commitViewportColor({ ...target, ...edit }));
    if (this.#lease !== target.lease) {
      return;
    }
    if (!outcome.ok) {
      this.#documentFailed(outcome.failure, edit);
      return;
    }
    this.confirmed = outcome.data;
    this.#sentHue = outcome.data.element.hue;
  }

  /** Read before showing the surface. Keep completed edits in order, coalescing only previews and placement. */
  #update(): void {
    const lease = this.#lease;
    const inactive = lease === null;
    if (inactive || this.#updating) {
      return;
    }
    this.#updating = true;
    this.#updates = this.#send({ lease, projectId: this.projectId });
  }

  async #send(target: ViewportTarget): Promise<void> {
    const { lease } = target;
    while (this.#lease === lease) {
      const placement = this.#placement;
      const needPlacement = placement !== this.#appliedPlacement;
      const hideFirst = needPlacement && placement?.visible === false;
      if (this.#readRequested && !hideFirst) {
        await this.#read(target);
        continue;
      }
      const nextCommit = hideFirst ? undefined : this.#commits.shift();
      const hasCommit = nextCommit !== undefined;
      if (hasCommit && this.confirmed) {
        this.#committing = true;

        await this.#commit(target, this.confirmed, nextCommit);
        if (this.#lease === lease) {
          this.#committing = false;
        }
        continue;
      }
      const hue = this.hue;
      const needHue = this.confirmed !== null && this.documentError === null && hue !== this.#sentHue;
      const selection = this.#wantedSelection;
      const needSelection = this.canSelect && selection !== this.selectedId;
      const place = placement && needPlacement && (!placement.visible || !needHue);
      let update: () => ReturnType<typeof commands.placeViewport>;
      if (place) {
        update = () => commands.placeViewport({ ...target, placement });
      } else if (needHue) {
        this.#sentHue = hue;
        update = () => commands.setViewportColor({ ...target, hue });
      } else if (needSelection) {
        update = () => commands.setViewportSelection({ ...target, elementId: selection });
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
      if (place) {
        this.#appliedPlacement = placement;
        if (placement === this.#placement) {
          this.#visible = placement.visible;
        }
      } else if (needSelection && !needHue) {
        this.selectedId = selection;
      }
    }
  }
}
