<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Collapsible from '@mannie/plugin-sdk/components/collapsible';
  import * as Item from '@mannie/plugin-sdk/components/item';
  import ChevronDownIcon from '@mannie/plugin-sdk/icons/chevron-down';

  import type { WorkerView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { workersActions } from '../state/workers-actions.svelte.js';
  import { ceilingModeLabel, workerStatusLabel } from '../utils/workers-messages.js';

  import WorkerStatusIcon from './WorkerStatusIcon.svelte';

  // One worker in the Workers settings (plugins/workers/docs/
  // worker-activity.md, the Workers settings): its name, status, address,
  // version and key, and on demand its purpose, host facts, ceiling and task
  // limit. Everything the worker said about itself is content. A worker that
  // needs an update says how the owner updates it on its host.
  interface Props {
    /** Whether the details start open, as when "Details" opened the settings for this worker. */
    open?: boolean;
    worker: WorkerView;
  }

  let { open = $bindable(false), worker }: Props = $props();

  // The worker leaves out what it does not know; older workers know less.
  const details = $derived(worker.details);
  const purpose = $derived(details.purpose ?? '');
  const facts = $derived(details.facts ?? null);
  const ceiling = $derived(details.ceiling ?? null);
  const version = $derived(details.version ?? null);
  const taskLimit = $derived(details.maxParallelTasks ?? 0);
  const numbers = new Intl.NumberFormat();
  const list = new Intl.ListFormat(undefined, { type: 'unit' });

  /** Names as a list, or "None". The names are content. */
  function listed(names: readonly string[]): string {
    return names.length === 0 ? m.workers_settings_none() : list.format(names);
  }
</script>

<Item.Root data-worker-details={worker.id} role="listitem" variant="outline">
  <Item.Content class="min-w-0 gap-3">
    <div class="flex min-w-0 flex-wrap items-center gap-2">
      <Item.Title class="min-w-0 wrap-anywhere" data-content dir="auto">{worker.name}</Item.Title>
      <span class="flex items-center gap-1.5 text-sm text-muted-foreground">
        <WorkerStatusIcon status={worker.status} />
        <span aria-hidden="true">{workerStatusLabel(worker.status)}</span>
      </span>
    </div>
    <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-sm">
      <dt class="text-muted-foreground">{m.workers_settings_address()}</dt>
      <dd class="font-mono wrap-anywhere" data-content dir="ltr">{worker.address}</dd>
      <dt class="text-muted-foreground">{m.workers_settings_version()}</dt>
      {#if version === null}
        <dd>{m.workers_settings_unknown()}</dd>
      {:else}
        <dd class="wrap-anywhere" data-content>{version}</dd>
      {/if}
      <dt class="text-muted-foreground">{m.workers_settings_key()}</dt>
      <dd class="font-mono wrap-anywhere" data-content dir="ltr">{worker.keyPin}</dd>
    </dl>
    {#if worker.status === 'incompatible'}
      <p class="text-sm text-destructive">{m.workers_settings_update_hint()}</p>
    {/if}
    <Collapsible.Root bind:open>
      <Collapsible.Trigger>
        {#snippet child({ props })}
          <Button {...props} class="-ms-2" data-worker-more="" size="sm" variant="ghost">
            {m.workers_settings_more()}
            <ChevronDownIcon class={['transition-transform', open && 'rotate-180']} data-icon="inline-end" />
          </Button>
        {/snippet}
      </Collapsible.Trigger>
      <Collapsible.Content>
        <dl class="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-sm">
          <dt class="text-muted-foreground">{m.workers_settings_purpose()}</dt>
          {#if purpose === ''}
            <dd>{m.workers_settings_unknown()}</dd>
          {:else}
            <dd class="wrap-anywhere whitespace-pre-wrap" data-content dir="auto">{purpose}</dd>
          {/if}
          <dt class="text-muted-foreground">{m.workers_settings_system()}</dt>
          {#if facts === null}
            <dd>{m.workers_settings_unknown()}</dd>
          {:else}
            <dd class="wrap-anywhere" data-content dir="auto">{facts.system}</dd>
            <dt class="text-muted-foreground">{m.workers_settings_shells()}</dt>
            <dd class="wrap-anywhere" data-content dir="auto">{listed(facts.shells ?? [])}</dd>
            <dt class="text-muted-foreground">{m.workers_settings_tools()}</dt>
            <dd class="wrap-anywhere" data-content dir="auto">{listed(facts.tools ?? [])}</dd>
          {/if}
          <dt class="text-muted-foreground">{m.workers_settings_ceiling()}</dt>
          {#if ceiling === null}
            <dd>{m.workers_settings_unknown()}</dd>
          {:else}
            <dd>{ceilingModeLabel(ceiling.mode)}</dd>
            <dt class="text-muted-foreground">{m.workers_settings_allowed_folders()}</dt>
            <dd class="font-mono wrap-anywhere" data-content dir="auto">{listed(ceiling.allowedFolders ?? [])}</dd>
            <dt class="text-muted-foreground">{m.workers_settings_rules()}</dt>
            <dd class="font-mono wrap-anywhere" data-content dir="auto">{listed(ceiling.rules ?? [])}</dd>
            <dt class="text-muted-foreground">{m.workers_settings_master_full_access()}</dt>
            <dd>
              {(ceiling.acceptsMasterFullAccess ?? true)
                ? m.workers_settings_master_full_access_answers()
                : m.workers_settings_master_full_access_refused()}
            </dd>
          {/if}
          <dt class="text-muted-foreground">{m.workers_settings_tasks_at_once()}</dt>
          <dd>{taskLimit === 0 ? m.workers_settings_unknown() : numbers.format(taskLimit)}</dd>
        </dl>
      </Collapsible.Content>
    </Collapsible.Root>
  </Item.Content>
  <Item.Actions class="self-start">
    <Button data-worker-rename="" size="sm" variant="outline" onclick={() => workersActions.startRename(worker)}>
      {m.workers_rename()}
    </Button>
    <Button data-worker-remove="" size="sm" variant="outline" onclick={() => workersActions.startRemove(worker)}>
      {m.workers_remove()}
    </Button>
  </Item.Actions>
</Item.Root>
