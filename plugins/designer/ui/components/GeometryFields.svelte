<script lang="ts">
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import { formatCount } from '@mannie/plugin-sdk/format';

  import { m } from '../generated/messages.js';
  import type { ProjectViewport } from '../state/viewport.svelte.js';
  import { DOCUMENT_LIMIT, type GeometryField } from '../utils/canvas-geometry.js';

  interface Props {
    viewport: ProjectViewport;
  }

  type GeometryInputEvent = Event & { currentTarget: HTMLInputElement };
  type GeometryFocusEvent = FocusEvent & { currentTarget: HTMLInputElement };

  const fields = ['x', 'y', 'width', 'height'] as const;
  const labels = {
    x: m.designer_geometry_x,
    y: m.designer_geometry_y,
    width: m.designer_geometry_width,
    height: m.designer_geometry_height,
  };

  let { viewport }: Props = $props();

  const id = $props.id();

  let group = $state<HTMLFieldSetElement | null>(null);

  const anyInvalid = $derived(fields.some((field) => viewport.geometryInvalid(field)));

  function change(field: GeometryField, event: GeometryInputEvent): void {
    viewport.changeGeometry(field, event.currentTarget.value);
  }

  function cancelBeforeNavigation(event: PointerEvent): void {
    const primary = event.button === 0 && event.isPrimary;
    const target = event.target;
    const hasElement = target instanceof Element;
    if (!primary || !group?.contains(document.activeElement) || !hasElement) {
      return;
    }

    // Pointer-down transfers focus before a project or dialog button handles its click.
    const targetProject = target.closest<HTMLButtonElement>('button[data-designer-project]')?.dataset.designerProject;
    const switchesProject = targetProject !== undefined && targetProject !== viewport.projectId;
    const opensDialog = target.closest('[data-designer-projects-add], [aria-haspopup="dialog"]') !== null;
    if (switchesProject || opensDialog) {
      viewport.cancelPreview();
    }
  }

  function leave(field: GeometryField, event: GeometryFocusEvent): void {
    const target = event.relatedTarget;
    const dialog = target instanceof Element && target.closest('[role="dialog"], [role="alertdialog"]') !== null;
    const inactive = event.currentTarget.closest('[inert]') !== null || !document.hasFocus();
    if (dialog || inactive) {
      viewport.cancelGeometry(field);
      return;
    }

    viewport.commitGeometry(field);
  }

  function handleKey(field: GeometryField, event: KeyboardEvent): void {
    if (event.isComposing) {
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      viewport.commitGeometry(field);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      viewport.cancelGeometry(field);
    }
  }
</script>

<svelte:window onpointerdowncapture={cancelBeforeNavigation} />

<Field.Set class="shrink-0 gap-1" bind:ref={group}>
  <Field.Legend class="sr-only">{m.designer_geometry_label()}</Field.Legend>

  <Field.Group class="grid max-w-lg grid-cols-4 gap-2">
    {#each fields as field (field)}
      {const invalid = $derived(viewport.geometryInvalid(field))}
      <Field.Field class="min-w-0 gap-1" data-disabled={!viewport.canEditGeometry} data-invalid={invalid}>
        <Field.Label for={`${id}-${field}`}>{labels[field]()}</Field.Label>
        <Input
          id={`${id}-${field}`}
          aria-describedby={invalid ? `${id}-error` : undefined}
          aria-invalid={invalid}
          autocomplete="off"
          data-designer-geometry={field}
          disabled={!viewport.canEditGeometry}
          inputmode="decimal"
          type="text"
          value={viewport.geometryText(field)}
          onblur={(event: GeometryFocusEvent) => leave(field, event)}
          oninput={(event: GeometryInputEvent) => change(field, event)}
          onkeydown={(event: KeyboardEvent) => handleKey(field, event)}
        />
      </Field.Field>
    {/each}
  </Field.Group>

  <!-- Reserve the message's height. Resizing the native surface cancels an active preview. -->
  <Field.Error id={`${id}-error`} class={anyInvalid ? undefined : 'invisible'} aria-hidden={!anyInvalid}>
    {m.designer_geometry_invalid({
      min: formatCount(-DOCUMENT_LIMIT),
      max: formatCount(DOCUMENT_LIMIT),
      size: formatCount(1),
    })}
  </Field.Error>
</Field.Set>
