// Live VNC console session state for one (conversation, server, vmid) side
// panel (ADR-0080): attaches/opens the Rust RFB session, streams throttled
// PNG snapshots and dirty-rect patches onto an HTML canvas, enforces takeover
// control toggles, and forwards owner input only while in owner control.
import { batchChannel, call, type ChannelMessage, type SubscriptionId, unsubscribe } from '@mannie/plugin-sdk/ipc';

import { channels, commands, type VncSessionSummary, type VncSessionUpdate } from '../generated/bindings.js';
import type { ProxmoxFailure } from '../types/proxmox.js';
import { labelVncPanel, type ProxmoxVncTarget } from '../utils/proxmox-panel.js';

/** Live VNC panel connection status. */
export type VncPanelStatus = 'closed' | 'connected' | 'connecting' | 'error';

interface QueuedPointerMove {
  readonly buttonMask: number;
  readonly x: number;
  readonly y: number;
}

function decodeBase64Png(base64: string): Blob {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return new Blob([bytes], { type: 'image/png' });
}

type DecodedImage = HTMLImageElement | ImageBitmap;

async function decodePngImage(base64: string): Promise<DecodedImage | null> {
  if (base64 === '') {
    return null;
  }

  const blob = decodeBase64Png(base64);
  if (typeof createImageBitmap === 'function') {
    return createImageBitmap(blob);
  }

  const url = URL.createObjectURL(blob);
  return new Promise<HTMLImageElement | null>((resolve) => {
    const img = new Image();

    img.onload = () => {
      URL.revokeObjectURL(url);
      resolve(img);
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      resolve(null);
    };
    img.src = url;
  });
}

function closeDrawable(image: DecodedImage): void {
  if ('close' in image) {
    image.close();
  }
}

/** Manages one live Proxmox VNC console stream and canvas renderer. */
export class VncSession {
  busy = $state(false);
  closedReason: string | null = $state(null);
  failure: ProxmoxFailure | null = $state(null);
  height = $state(0);
  status: VncPanelStatus = $state('connecting');
  summary: VncSessionSummary | null = $state(null);
  readonly target: ProxmoxVncTarget;
  width = $state(0);

  #canvas: HTMLCanvasElement | null = null;
  #drawQueue: Promise<void> = Promise.resolve();
  #generation = 0;
  #lastSnapshotPng = '';
  #moveInFlight = false;
  #pendingMove: QueuedPointerMove | null = null;
  #subscription: SubscriptionId | null = null;

  constructor(target: ProxmoxVncTarget) {
    this.target = target;
  }

  /** Attaches or detaches the display canvas element. */
  attachCanvas(canvas: HTMLCanvasElement | null): void {
    this.#canvas = canvas;
    const hasCanvas = canvas !== null;
    const hasSnapshot = this.#lastSnapshotPng !== '';
    if (hasCanvas && hasSnapshot) {
      const png = this.#lastSnapshotPng;
      const w = this.width;
      const h = this.height;
      this.#enqueueDraw(async () => {
        await this.#paintFull(png, w, h);
      });
    }
  }

  /** Closes the underlying live VNC session on the owner's behalf. */
  async closeSession(): Promise<void> {
    if (this.busy) {
      return;
    }

    this.busy = true;
    this.failure = null;
    this.stop();
    const outcome = await call(() => commands.closeVncSession(this.target));

    this.busy = false;
    if (!outcome.ok) {
      this.failure = outcome.failure;
      return;
    }

    this.closedReason = 'owner_closed';
    this.status = 'closed';
  }

  /** Opens or attaches to the live VNC session and starts streaming frames. */
  async connect(): Promise<void> {
    this.#generation += 1;
    const generation = this.#generation;
    this.#dropSubscription();
    this.status = 'connecting';
    this.failure = null;
    this.closedReason = null;

    const opened = await call(() => commands.openVncSession(this.target));
    if (generation !== this.#generation) {
      return;
    }
    if (!opened.ok) {
      this.failure = opened.failure;
      this.status = 'error';
      return;
    }
    this.#applySummary(opened.data);
    await this.#subscribe(generation);
  }

  /** Hands control back to the agent so agent VNC tools can run again. */
  async resumeAgent(): Promise<void> {
    await this.#setControl('agent');
  }

  /** Sends Ctrl+Alt+Delete to the VM (requires owner takeover). */
  async sendCtrlAltDel(): Promise<void> {
    const ownerControls = this.summary?.control === 'owner';
    if (this.busy || !ownerControls) {
      return;
    }

    this.busy = true;
    this.failure = null;
    const outcome = await call(() => commands.vncSendCtrlAltDel(this.target));

    this.busy = false;
    if (!outcome.ok) {
      this.failure = outcome.failure;
    }
  }

  /** Sends an owner key press or release (requires owner takeover). */
  sendKey(keysym: number, down: boolean): void {
    if (this.summary?.control !== 'owner') {
      return;
    }
    void call(() =>
      commands.vncKeyboard({
        ...this.target,
        down,
        keysym,
      }),
    );
  }

  /** Sends an owner pointer button/scroll/move event (requires owner takeover). */
  sendPointer(x: number, y: number, buttonMask: number, coalesce = false): void {
    if (this.summary?.control !== 'owner') {
      return;
    }
    if (coalesce) {
      this.#pendingMove = { buttonMask, x, y };
      if (!this.#moveInFlight) {
        void this.#flushMoves();
      }
      return;
    }
    this.#pendingMove = null;
    void call(() =>
      commands.vncPointer({
        ...this.target,
        buttonMask,
        x,
        y,
      }),
    );
  }

  /** Detaches the panel stream without closing the underlying VNC session. */
  stop(): void {
    this.#generation += 1;
    this.#pendingMove = null;
    this.#dropSubscription();
  }

  /** Takes over live console control, blocking all agent VNC tools in Rust. */
  async takeOver(): Promise<void> {
    await this.#setControl('owner');
  }

  #applyMessage(message: ChannelMessage<VncSessionUpdate>, generation: number): void {
    if (generation !== this.#generation) {
      return;
    }
    if (message.kind === 'ended') {
      if (message.reason === 'lagged') {
        void this.#subscribe(generation);
      }
      return;
    }
    const update = message.item;
    switch (update.kind) {
      case 'snapshot': {
        this.#applySummary(update.session);
        this.status = 'connected';
        if (update.pngBase64 !== '') {
          this.#lastSnapshotPng = update.pngBase64;
          const png = update.pngBase64;
          const w = update.session.width;
          const h = update.session.height;
          this.#enqueueDraw(async () => {
            await this.#paintFull(png, w, h);
          });
        }
        return;
      }
      case 'patch': {
        this.status = 'connected';
        if (update.pngBase64 !== '') {
          const { height, pngBase64, width, x, y } = update;
          this.#enqueueDraw(async () => {
            await this.#paintPatch(pngBase64, x, y, width, height);
          });
        }
        return;
      }
      case 'status': {
        this.#applySummary(update.session);
        if (update.session.state === 'connected') {
          this.status = 'connected';
        }
        return;
      }
      case 'closed': {
        this.closedReason = update.reason;
        this.status = 'closed';
        this.#dropSubscription();
      }
    }
  }

  #applySummary(summary: VncSessionSummary): void {
    this.summary = summary;
    if (summary.width > 0) {
      this.width = summary.width;
    }
    if (summary.height > 0) {
      this.height = summary.height;
    }
    labelVncPanel(this.target, summary.vmName);
  }

  #dropSubscription(): void {
    const previous = this.#subscription;

    this.#subscription = null;
    if (previous !== null) {
      void unsubscribe(previous);
    }
  }

  #enqueueDraw(work: () => Promise<void>): void {
    this.#drawQueue = this.#drawQueue.then(work).catch(() => {
      // Ignore stale or aborted frame decode errors when the panel closes.
    });
  }

  async #flushMoves(): Promise<void> {
    this.#moveInFlight = true;

    try {
      while (this.#pendingMove !== null) {
        const next = this.#pendingMove;

        this.#pendingMove = null;

        await call(() =>
          commands.vncPointer({
            ...this.target,
            buttonMask: next.buttonMask,
            x: next.x,
            y: next.y,
          }),
        );
      }
    } finally {
      this.#moveInFlight = false;
    }
  }

  async #paintFull(pngBase64: string, width: number, height: number): Promise<void> {
    const canvas = this.#canvas;
    if (canvas === null) {
      return;
    }
    const image = await decodePngImage(pngBase64);
    if (image === null) {
      return;
    }
    try {
      const targetWidth = width > 0 ? width : image.width;
      const targetHeight = height > 0 ? height : image.height;
      const sizeChanged = canvas.width !== targetWidth || canvas.height !== targetHeight;
      if (sizeChanged) {
        canvas.width = targetWidth;
        canvas.height = targetHeight;
      }

      const ctx = canvas.getContext('2d');
      ctx?.drawImage(image, 0, 0);
    } finally {
      closeDrawable(image);
    }
  }

  async #paintPatch(pngBase64: string, x: number, y: number, width: number, height: number): Promise<void> {
    const canvas = this.#canvas;
    if (canvas === null) {
      return;
    }
    const image = await decodePngImage(pngBase64);
    if (image === null) {
      return;
    }
    try {
      const ctx = canvas.getContext('2d');
      ctx?.drawImage(image, x, y, width, height);
    } finally {
      closeDrawable(image);
    }
  }

  async #setControl(control: 'agent' | 'owner'): Promise<void> {
    if (this.busy) {
      return;
    }
    this.busy = true;
    this.failure = null;
    const outcome = await call(() => commands.setVncControl({ ...this.target, control }));

    this.busy = false;
    if (!outcome.ok) {
      this.failure = outcome.failure;
      return;
    }
    this.#applySummary(outcome.data);
  }

  async #subscribe(generation: number): Promise<void> {
    this.#dropSubscription();
    const channel = batchChannel<ChannelMessage<VncSessionUpdate>>(
      (items) => {
        for (const item of items) {
          this.#applyMessage(item, generation);
        }
      },
      () => {
        const isCurrent = generation === this.#generation;
        const isOpen = this.status !== 'closed';
        if (isCurrent && isOpen) {
          void this.#subscribe(generation);
        }
      },
    );
    const outcome = await call(() => channels.followVncSession(this.target, channel));
    if (generation !== this.#generation) {
      if (outcome.ok) {
        void unsubscribe(outcome.data);
      }
      return;
    }
    if (outcome.ok) {
      this.#subscription = outcome.data;
      return;
    }
    this.failure = outcome.failure;
    this.status = 'error';
  }
}
