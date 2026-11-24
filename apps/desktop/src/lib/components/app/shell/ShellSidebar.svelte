<script lang="ts">
  import { onDestroy, type Snippet } from 'svelte';

  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { SidebarResizeControls } from '#lib/state/sidebar-widths.svelte.js';
  import type { NavigationEntry } from '#lib/types/shell-slots.js';

  import ActivityBar from './ActivityBar.svelte';

  // The window's sidebar (ADR-0045, decision 1; ADR-0077): the activity bar and
  // beside it the content of the selected entry or of the settings, as
  // shadcn-svelte's block sidebar-09 composes an icon rail and a second sidebar
  // inside one. Collapsed, the sidebar keeps the bar alone, and the content
  // leaves the page, so that the focus cannot reach it. Resizable activities
  // add a pointer and keyboard window splitter on the content column's trailing
  // edge without editing `components/ui/`.
  interface Props {
    /** The content beside the bar. */
    children: Snippet;
    /** The entry that the bar shows as selected, if any. */
    current: NavigationEntry | undefined;
    /** Opens the command palette. */
    onFindCommand: () => void;
    /** Whether the settings page shows. */
    onSettings: boolean;
    /** Resize controls when the active sidebar allows resizing (ADR-0077). */
    resize?: SidebarResizeControls;
  }

  const KEY_STEP_PX = 16;
  const SHIFT_KEY_STEP_PX = 48;

  let { children, current, onFindCommand, onSettings, resize }: Props = $props();

  const sidebar = Sidebar.useSidebar();

  let dragging = false;
  let dragStartX = 0;
  let dragStartWidth = 0;
  let dragDirection = 1;
  let pendingWidth: number | null = null;
  let resizeFrame: number | null = null;

  const activeResize = $derived.by(() => {
    const isExpanded = sidebar.state === 'expanded';
    const isVisible = !sidebar.isMobile && isExpanded;

    return isVisible ? resize : undefined;
  });

  function horizontalDirection(): number {
    if (typeof document === 'undefined') {
      return 1;
    }
    return getComputedStyle(document.documentElement).direction === 'rtl' ? -1 : 1;
  }

  function flushPendingWidth(): void {
    resizeFrame = null;
    if (pendingWidth !== null) {
      const next = pendingWidth;
      pendingWidth = null;
      resize?.onPreview(next);
    }
  }

  function handlePointerDown(event: PointerEvent): void {
    const isPrimaryButton = event.button === 0;
    const hasResize = resize !== undefined;

    if (!isPrimaryButton || !hasResize) {
      return;
    }

    event.preventDefault();
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    dragging = true;
    dragStartX = event.clientX;
    dragStartWidth = resize.width;
    dragDirection = horizontalDirection();
    resize.onDraggingChange(true);
  }

  function handlePointerMove(event: PointerEvent): void {
    if (!dragging) {
      return;
    }

    const delta = (event.clientX - dragStartX) * dragDirection;
    pendingWidth = dragStartWidth + delta;
    resizeFrame ??= requestAnimationFrame(flushPendingWidth);
  }

  function finishDrag(event: PointerEvent): void {
    if (!dragging) {
      return;
    }

    dragging = false;
    const handle = event.currentTarget as HTMLElement;
    if (handle.hasPointerCapture(event.pointerId)) {
      handle.releasePointerCapture(event.pointerId);
    }

    const delta = (event.clientX - dragStartX) * dragDirection;
    const finalWidth = pendingWidth ?? dragStartWidth + delta;
    pendingWidth = null;

    if (resizeFrame !== null) {
      cancelAnimationFrame(resizeFrame);
      resizeFrame = null;
    }

    resize?.onCommit(finalWidth);
    resize?.onDraggingChange(false);
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (resize === undefined) {
      return;
    }

    const step = event.shiftKey ? SHIFT_KEY_STEP_PX : KEY_STEP_PX;
    const direction = horizontalDirection();

    switch (event.key) {
      case 'ArrowRight': {
        event.preventDefault();
        resize.onCommit(resize.width + step * direction);
        return;
      }
      case 'ArrowLeft': {
        event.preventDefault();
        resize.onCommit(resize.width - step * direction);
        return;
      }
      case 'Home': {
        event.preventDefault();
        resize.onCommit(resize.minWidth);
        return;
      }
      case 'End': {
        event.preventDefault();
        resize.onCommit(resize.maxWidth);
        return;
      }
      case 'Enter': {
        event.preventDefault();
        resize.onReset();
      }
    }
  }

  function handleDoubleClick(): void {
    resize?.onReset();
  }

  onDestroy(() => {
    if (resizeFrame !== null) {
      cancelAnimationFrame(resizeFrame);
      resizeFrame = null;
    }
    if (dragging) {
      dragging = false;
      resize?.onDraggingChange(false);
    }
  });
</script>

<Sidebar.Root class="overflow-hidden *:data-[sidebar=sidebar]:flex-row" collapsible="icon">
  <ActivityBar {current} {onFindCommand} {onSettings} />
  <Sidebar.Root
    class="relative min-w-0 flex-1 group-data-[collapsible=icon]:hidden"
    collapsible="none"
    data-shell-sidebar="content"
  >
    {@render children()}

    {#if activeResize !== undefined}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div
        class="group/sidebar-resize absolute inset-y-0 inset-e-0 z-20 hidden w-1.5 cursor-ew-resize touch-none items-center justify-center transition-colors after:absolute after:-inset-x-1 after:inset-y-0 hover:bg-primary/35 focus-visible:bg-primary/45 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none md:flex"
        aria-label={m.shell_sidebar_resize()}
        aria-orientation="vertical"
        aria-valuemax={activeResize.maxWidth}
        aria-valuemin={activeResize.minWidth}
        aria-valuenow={activeResize.width}
        data-shell-sidebar-resizer=""
        role="separator"
        tabindex="0"
        ondblclick={handleDoubleClick}
        onkeydown={handleKeydown}
        onpointercancel={finishDrag}
        onpointerdown={handlePointerDown}
        onpointermove={handlePointerMove}
        onpointerup={finishDrag}
      >
        <div
          class="h-8 w-0.5 rounded-full bg-transparent transition-colors group-hover/sidebar-resize:bg-primary"
        ></div>
      </div>
    {/if}
  </Sidebar.Root>
</Sidebar.Root>
