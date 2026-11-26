<script lang="ts">
  import { m } from '../generated/messages.js';
  import type { VncSession } from '../state/vnc-session.svelte.js';
  import {
    type CanvasCoords,
    domButtonsToRfbMask,
    keyboardEventToKeysym,
    scaleCanvasCoords,
  } from '../utils/proxmox-keys.js';

  interface Props {
    readonly interactive: boolean;
    readonly onExitFocus: () => void;
    readonly session: VncSession;
    readonly vmid: number;
    readonly vmName: string;
  }

  let { interactive, onExitFocus, session, vmid, vmName }: Props = $props();

  let canvas = $state<HTMLCanvasElement | null>(null);

  function canvasCoordinates(event: MouseEvent): CanvasCoords | null {
    if (canvas === null) {
      return null;
    }

    const hasFramebuffer = session.width > 0 && session.height > 0;
    if (!hasFramebuffer) {
      return null;
    }

    const rect = canvas.getBoundingClientRect();
    return scaleCanvasCoords(event.clientX, event.clientY, rect, session.width, session.height);
  }

  function handlePointerMove(event: PointerEvent): void {
    if (!interactive) {
      return;
    }

    const coords = canvasCoordinates(event);
    if (coords === null) {
      return;
    }

    session.sendPointer(coords.x, coords.y, domButtonsToRfbMask(event.buttons), true);
  }

  function handlePointerDown(event: PointerEvent): void {
    if (!interactive) {
      return;
    }

    canvas?.focus();
    const coords = canvasCoordinates(event);
    if (coords === null) {
      return;
    }

    event.preventDefault();
    session.sendPointer(coords.x, coords.y, domButtonsToRfbMask(event.buttons), false);
  }

  function handlePointerUp(event: PointerEvent): void {
    if (!interactive) {
      return;
    }

    const coords = canvasCoordinates(event);
    if (coords === null) {
      return;
    }

    event.preventDefault();
    session.sendPointer(coords.x, coords.y, domButtonsToRfbMask(event.buttons), false);
  }

  function handleWheel(event: WheelEvent): void {
    if (!interactive) {
      return;
    }

    const coords = canvasCoordinates(event);
    if (coords === null) {
      return;
    }

    event.preventDefault();
    const scrollBit = event.deltaY < 0 ? 8 : 16;
    const baseMask = domButtonsToRfbMask(event.buttons);
    session.sendPointer(coords.x, coords.y, baseMask | scrollBit, false);
    session.sendPointer(coords.x, coords.y, baseMask, false);
  }

  function handleContextMenu(event: MouseEvent): void {
    if (interactive) {
      event.preventDefault();
    }
  }

  function handleKeyDown(event: KeyboardEvent): void {
    if (!interactive) {
      return;
    }

    const isEscapeChord = event.ctrlKey && event.altKey && event.key === 'Escape';
    if (isEscapeChord) {
      event.preventDefault();
      onExitFocus();
      return;
    }

    const keysym = keyboardEventToKeysym(event.key, event.location);
    if (keysym === null) {
      return;
    }

    event.preventDefault();
    session.sendKey(keysym, true);
  }

  function handleKeyUp(event: KeyboardEvent): void {
    if (!interactive) {
      return;
    }

    const keysym = keyboardEventToKeysym(event.key, event.location);
    if (keysym === null) {
      return;
    }

    event.preventDefault();
    session.sendKey(keysym, false);
  }

  $effect(() => {
    const element = canvas;
    session.attachCanvas(element);

    return () => {
      session.attachCanvas(null);
    };
  });

  $effect(() => {
    const element = canvas;
    if (element === null) {
      return;
    }

    const isFocused = document.activeElement === element;
    if (!interactive && isFocused) {
      element.blur();
    }
  });
</script>

<div class="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden rounded-md border bg-muted">
  <canvas
    bind:this={canvas}
    class="max-h-full max-w-full object-contain outline-none focus-visible:ring-2 focus-visible:ring-ring"
    aria-label={m.proxmox_panel_canvas_label({ vm: vmName, vmid: String(vmid) })}
    data-proxmox-vnc-canvas=""
    data-proxmox-vnc-interactive={interactive ? 'true' : 'false'}
    role={interactive ? 'application' : undefined}
    tabindex={interactive ? 0 : -1}
    oncontextmenu={handleContextMenu}
    onkeydown={handleKeyDown}
    onkeyup={handleKeyUp}
    onpointerdown={handlePointerDown}
    onpointermove={handlePointerMove}
    onpointerup={handlePointerUp}
    onwheel={handleWheel}
  ></canvas>
</div>
