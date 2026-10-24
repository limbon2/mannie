<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as DropdownMenu from '@mannie/plugin-sdk/components/dropdown-menu';
  import * as Item from '@mannie/plugin-sdk/components/item';
  import { Label } from '@mannie/plugin-sdk/components/label';
  import { Progress } from '@mannie/plugin-sdk/components/progress';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import { Switch } from '@mannie/plugin-sdk/components/switch';
  import EllipsisVerticalIcon from '@mannie/plugin-sdk/icons/ellipsis-vertical';
  import FileTextIcon from '@mannie/plugin-sdk/icons/file-text';

  import type { SourceView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { notebookActions } from '../state/notebook-actions.svelte.js';
  import {
    canRetry,
    sourceDetails,
    sourcePassages,
    type SourcePhase,
    sourcePhase,
    sourcePhaseLabel,
    sourceProblem,
    sourceProgress,
  } from '../utils/notebooks-labels.js';
  import { notebooksFailureMessage } from '../utils/notebooks-messages.js';
  import { openSource } from '../utils/notebooks-places.js';

  // One source on a notebook's page (plugins/notebooks/docs/README.md,
  // sources): its name, which shows its text in the viewer, what it is,
  // its state with the progress of its indexing or why it failed or waits,
  // the switch "Use in answers", "Try again" for a source whose indexing
  // failed, and "Remove…" in its menu. The switch is named by its visible
  // label and described by the source's name (WCAG 2.5.3).
  let { source }: { source: SourceView } = $props();

  let nameButton = $state<HTMLElement | null>(null);

  const phase = $derived(sourcePhase(source));
  const problem = $derived(sourceProblem(source));
  const actionFailure = $derived(notebookActions.sourceFailures[source.id] ?? null);
  const isBusy = $derived(phase === 'reading' || phase === 'indexing');
  const showsProgress = $derived(phase === 'indexing' || phase === 'paused');
  const nameId = $derived(`notebooks-source-name-${source.id}`);
  const progressId = $derived(`notebooks-source-progress-${source.id}`);
  const switchId = $derived(`notebooks-source-included-${source.id}`);

  /** How the badge marks a phase: a failure stands out, and a ready source is calm. */
  function badgeVariant(shown: SourcePhase): 'destructive' | 'outline' | 'secondary' {
    switch (shown) {
      case 'failed': {
        return 'destructive';
      }
      case 'ready': {
        return 'secondary';
      }
      case 'indexing':
      case 'paused':
      case 'reading':
      case 'waiting': {
        return 'outline';
      }
    }
  }
</script>

<div role="listitem">
  <Item.Root data-notebooks-source={source.id} data-notebooks-state={phase} variant="outline">
    <Item.Media variant="icon">
      <FileTextIcon />
    </Item.Media>
    <Item.Content class="min-w-0 gap-1.5">
      <Item.Title class="max-w-full min-w-0">
        <Button
          id={nameId}
          class="h-auto max-w-full min-w-0 justify-start p-0 text-start"
          data-notebooks-open-source=""
          title={m.notebooks_source_open()}
          variant="link"
          bind:ref={nameButton}
          onclick={() => openSource(source.id, nameButton)}
        >
          <span class="truncate" data-content dir="auto">{source.name}</span>
        </Button>
      </Item.Title>
      <Item.Description class="text-xs">{sourceDetails(source)}</Item.Description>
      <div class="flex flex-wrap items-center gap-2 text-sm">
        <Badge data-notebooks-phase={phase} variant={badgeVariant(phase)}>
          {#if isBusy}
            <Spinner aria-hidden="true" />
          {/if}
          {sourcePhaseLabel(phase)}
        </Badge>
        {#if showsProgress}
          <span id={progressId} class="text-muted-foreground">{sourceProgress(source)}</span>
        {:else if phase === 'ready'}
          <span class="text-muted-foreground">{sourcePassages(source)}</span>
        {/if}
      </div>
      {#if showsProgress}
        <Progress
          class="max-w-xs"
          aria-labelledby={progressId}
          max={Math.max(source.passages, 1)}
          value={source.embedded}
        />
      {/if}
      {#if problem !== null}
        <p class="text-sm text-destructive" data-notebooks-problem={source.failure}>{problem}</p>
      {/if}
      {#if actionFailure !== null}
        <p class="text-sm text-destructive" role="alert">{notebooksFailureMessage(actionFailure)}</p>
      {/if}
    </Item.Content>
    <Item.Actions class="flex-wrap self-start">
      {#if canRetry(source)}
        <Button data-notebooks-retry="" size="sm" variant="outline" onclick={() => void notebookActions.retry(source)}>
          {m.notebooks_source_retry()}
        </Button>
      {/if}
      <div class="flex items-center gap-2">
        <Switch
          id={switchId}
          aria-describedby={nameId}
          data-notebooks-included=""
          bind:checked={
            () => notebookActions.isIncluded(source),
            (checked: boolean) => void notebookActions.setIncluded(source, checked)
          }
        />
        <Label class="text-sm font-normal" for={switchId}>{m.notebooks_source_included()}</Label>
      </div>
      <DropdownMenu.Root>
        <DropdownMenu.Trigger>
          {#snippet child({ props })}
            <Button
              {...props}
              aria-label={m.notebooks_source_menu({ name: source.name })}
              data-notebooks-source-menu=""
              size="icon-sm"
              variant="ghost"
            >
              <EllipsisVerticalIcon />
            </Button>
          {/snippet}
        </DropdownMenu.Trigger>
        <DropdownMenu.Content align="end">
          <DropdownMenu.Group>
            <DropdownMenu.Item onSelect={() => openSource(source.id, nameButton)}>
              {m.notebooks_source_open()}
            </DropdownMenu.Item>
          </DropdownMenu.Group>
          <DropdownMenu.Separator />
          <DropdownMenu.Group>
            <DropdownMenu.Item
              data-notebooks-remove-source=""
              variant="destructive"
              onSelect={() => notebookActions.startRemove(source)}
            >
              {m.notebooks_source_remove()}
            </DropdownMenu.Item>
          </DropdownMenu.Group>
        </DropdownMenu.Content>
      </DropdownMenu.Root>
    </Item.Actions>
  </Item.Root>
</div>
