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
  import { watchViewport } from '../utils/viewport-placement.js';

  interface Props {
    project: ProjectView;
  }

  let { project }: Props = $props();

  // DesignerView keys this component by project ID. A refreshed project snapshot keeps the lease.
  const viewport = new ProjectViewport(untrack(() => project.id));
  const labelId = $props.id();

  const formattedHue = $derived(m.designer_hue_degrees({ value: formatCount(viewport.hue) }));
  const sceneReady = $derived(!viewport.loading && viewport.failure === null);

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

  onMount(() => {
    void viewport.open();

    return () => viewport.close();
  });
</script>

<div class="flex min-h-0 flex-1 flex-col gap-4 p-4" data-designer-project={project.id}>
  <h2 class="shrink-0 truncate text-base font-medium" data-content="">{project.name}</h2>
  <Field.Field class="max-w-sm shrink-0">
    <div class="flex items-baseline justify-between gap-4">
      <Field.Title id={labelId}>{m.designer_rectangle_color()}</Field.Title>
      <span class="text-sm text-muted-foreground tabular-nums">{formattedHue}</span>
    </div>

    <div {@attach labelThumb} data-designer-hue-track="">
      <Slider
        class="min-h-6"
        data-designer-hue=""
        disabled={viewport.loading || viewport.failure !== null}
        max={360}
        min={0}
        step={1}
        type="single"
        value={viewport.hue}
        onValueChange={changeHue}
      />
    </div>
  </Field.Field>

  <div
    class="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden border bg-muted"
    data-designer-viewport=""
  >
    <div
      class="absolute inset-0"
      {@attach followBounds}
      aria-label={sceneReady ? m.designer_viewport_scene({ hue: formattedHue }) : undefined}
      role={sceneReady ? 'img' : undefined}
    ></div>
    {#if viewport.failure !== null}
      <Alert.Root class="relative mx-4 max-w-md" data-designer-viewport-error="">
        <Alert.Description>
          <p>{viewport.failure.message}</p>
          {#if viewport.failure.retry}
            <Button size="sm" variant="outline" onclick={() => void viewport.open()}>{m.designer_try_again()}</Button>
          {/if}
        </Alert.Description>
      </Alert.Root>
    {:else if viewport.loading}
      <p class="relative text-sm text-muted-foreground" role="status">{m.designer_viewport_loading()}</p>
    {/if}
  </div>
</div>

<style>
  [data-designer-hue-track] :global([data-slot='slider-track']) {
    background: linear-gradient(to right, red, yellow, lime, cyan, blue, magenta, red);
  }

  [data-designer-hue-track] :global([data-slot='slider-range']) {
    background: transparent;
  }
</style>
