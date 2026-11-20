<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Slider } from '@mannie/plugin-sdk/components/slider';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import { onMount, untrack } from 'svelte';

  import type { ProjectView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { ProjectViewport } from '../state/viewport.svelte.js';
  import { formatGeometryValue } from '../utils/canvas-geometry.js';
  import { hasGeometry } from '../utils/canvas-selection.js';
  import { watchViewport } from '../utils/viewport-placement.js';

  import GeometryFields from './GeometryFields.svelte';

  interface Props {
    project: ProjectView;
  }

  type CanvasClick = MouseEvent & { currentTarget: HTMLElement };

  let { project }: Props = $props();

  // DesignerView keys this component by project ID. A refreshed project snapshot keeps the lease.
  const viewport = new ProjectViewport(untrack(() => project.id));
  const labelId = $props.id();

  const formattedHue = $derived(m.designer_hue_degrees({ value: formatCount(viewport.hue) }));
  const sceneReady = $derived(viewport.confirmed !== null && viewport.failure === null);
  const selectionText = $derived.by(() => {
    const element = viewport.confirmed?.element;
    const selected = element?.id === viewport.selectedId;
    if (!selected || !hasGeometry(element)) {
      return m.designer_selection_empty();
    }
    return m.designer_selection_rectangle({
      height: formatGeometryValue(element.height),
      width: formatGeometryValue(element.width),
      x: formatGeometryValue(element.x),
      y: formatGeometryValue(element.y),
    });
  });

  function followBounds(element: HTMLElement): () => void {
    // Initial placement can read the selected hue while sending IPC. A hue edit must keep this observer.
    return untrack(() => watchViewport(element, (placement) => viewport.place(placement)));
  }

  // The registry slider forwards props to its group. Its thumb owns keyboard focus and ARIA values.
  function labelThumb(element: HTMLElement): void {
    const thumb = element.querySelector('[role="slider"]');
    if (thumb === null) {
      return;
    }
    thumb.setAttribute('aria-labelledby', labelId);
    $effect(() => {
      thumb.ariaValueText = formattedHue;
    });
  }

  function changeHue(hue: number): void {
    viewport.changeHue(hue);
  }

  function commitHue(hue: number): void {
    viewport.commitHue(hue);
  }

  function cancelOnEscape(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      viewport.cancelPreview();
    }
  }

  function activateCanvas(event: CanvasClick): void {
    // Enter, Space and assistive technology activate a native button without pointer coordinates.
    if (event.detail === 0) {
      viewport.select({ kind: 'select' });
      return;
    }
    const primary = event.button === 0;
    const placeholder = event.currentTarget.parentElement;
    if (primary && placeholder) {
      const bounds = placeholder.getBoundingClientRect();
      viewport.select({ kind: 'point', client: { x: event.clientX, y: event.clientY }, bounds });
    }
  }

  function clearOnEscape(event: KeyboardEvent): void {
    const escape = event.key === 'Escape';
    if (escape && viewport.canSelect) {
      event.preventDefault();
      event.stopPropagation();
      viewport.select({ kind: 'clear' });
    }
  }

  onMount(() => {
    void viewport.open();

    return () => viewport.dispose();
  });
</script>

<svelte:window
  onblur={() => viewport.cancelPreview()}
  onkeydown={cancelOnEscape}
  onpointercancel={() => viewport.cancelPreview()}
/>

<div class="flex min-h-0 flex-1 flex-col gap-4 p-4" data-designer-project={project.id}>
  <h2 class="shrink-0 truncate text-base font-medium" data-content="">{project.name}</h2>
  <Field.Field class="max-w-sm shrink-0">
    <div class="flex items-baseline justify-between gap-4">
      <Field.Title id={labelId}>{m.designer_rectangle_color()}</Field.Title>
      <span class="text-sm text-muted-foreground tabular-nums">{formattedHue}</span>
    </div>

    {#key viewport.sliderGeneration}
      <div {@attach labelThumb} data-designer-hue-track="">
        <Slider
          class="min-h-6"
          data-designer-hue=""
          disabled={!viewport.canEdit}
          max={360}
          min={0}
          step={1}
          type="single"
          value={viewport.hue}
          onValueChange={changeHue}
          onValueCommit={commitHue}
        />
      </div>
    {/key}
  </Field.Field>

  {#if viewport.documentError !== null}
    <Alert.Root class="max-w-md shrink-0" data-designer-document-error="">
      <Alert.Description>
        <p>{viewport.documentError}</p>
        <Button size="sm" variant="outline" onclick={() => viewport.retryDocument()}>{m.designer_try_again()}</Button>
      </Alert.Description>
    </Alert.Root>
  {/if}

  <div
    class="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden border bg-muted"
    data-designer-viewport=""
  >
    <div class="absolute inset-0" {@attach followBounds}>
      <Button
        class="size-full rounded-none"
        aria-describedby={`${labelId}-instructions ${labelId}-selection`}
        aria-label={m.designer_canvas_name()}
        data-designer-canvas=""
        disabled={!viewport.canSelect}
        variant="ghost"
        onclick={activateCanvas}
        onkeydown={clearOnEscape}
      />
    </div>

    {#if viewport.failure !== null}
      <Alert.Root class="relative mx-4 max-w-md" data-designer-viewport-error="">
        <Alert.Description>
          <p>{viewport.failure.message}</p>
          {#if viewport.failure.retry}
            <Button size="sm" variant="outline" onclick={() => void viewport.open()}>{m.designer_try_again()}</Button>
          {/if}
        </Alert.Description>
      </Alert.Root>
    {:else if viewport.loading && !sceneReady}
      <p class="relative text-sm text-muted-foreground" role="status">{m.designer_viewport_loading()}</p>
    {/if}
  </div>

  {#if sceneReady}
    <GeometryFields {viewport} />
  {/if}

  <div class="sr-only">
    <p id={`${labelId}-instructions`}>
      {m.designer_canvas_instructions()}
      {m.designer_viewport_scene({ hue: formattedHue })}
    </p>

    <p id={`${labelId}-selection`} data-designer-selection="" role="status">{sceneReady ? selectionText : ''}</p>
  </div>
</div>

<style>
  /* The native child covers the placeholder. Keep keyboard focus visible outside its bounds. */
  [data-designer-viewport]:has(:global([data-designer-canvas]:focus-visible)) {
    outline: 2px solid var(--ring);
    outline-offset: 2px;
  }

  [data-designer-hue-track] :global([data-slot='slider-track']) {
    background: linear-gradient(to right, red, yellow, lime, cyan, blue, magenta, red);
  }

  [data-designer-hue-track] :global([data-slot='slider-range']) {
    background: transparent;
  }
</style>
