<script lang="ts">
  import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
  import ServerIcon from '@lucide/svelte/icons/server';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import type { WorkerView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { workersActions } from '#lib/state/workers-actions.svelte.js';

  import WorkerStatusIcon from './WorkerStatusIcon.svelte';

  // One worker in the sidebar (docs/features/workers/worker-activity.md,
  // workers in the sidebar): its name, which the worker or the owner gave and
  // so is content, and its status. The row opens the worker's details in the
  // Workers settings; its menu offers "Details", "Rename…" and "Remove…". The
  // menu's button lies over room that the row keeps free, so nothing moves
  // when it shows.
  let { worker }: { worker: WorkerView } = $props();
</script>

<Sidebar.MenuItem data-worker={worker.id}>
  <div class="group/worker relative">
    <Sidebar.MenuButton
      class="pe-8 group-hover/worker:bg-sidebar-accent group-hover/worker:text-sidebar-accent-foreground"
      data-worker-row=""
      onclick={() => void workersActions.showDetails(worker)}
    >
      <ServerIcon />
      <span class="min-w-0 truncate" data-content dir="auto">{worker.name}</span>
      <WorkerStatusIcon class="ms-auto" status={worker.status} />
    </Sidebar.MenuButton>
    <div
      class="absolute inset-y-0 inset-e-1 flex items-center group-focus-within/worker:opacity-100 group-hover/worker:opacity-100 has-aria-expanded:opacity-100 md:opacity-0"
    >
      <DropdownMenu.Root>
        <DropdownMenu.Trigger>
          {#snippet child({ props: trigger })}
            <Button {...trigger} data-worker-menu="" size="icon-xs" variant="ghost">
              <EllipsisIcon class="size-4" />
              <span class="sr-only">{m.workers_row_actions()}</span>
            </Button>
          {/snippet}
        </DropdownMenu.Trigger>
        <DropdownMenu.Content align="start" side="right">
          <DropdownMenu.Group>
            <DropdownMenu.Item onSelect={() => void workersActions.showDetails(worker)}>
              {m.workers_details()}
            </DropdownMenu.Item>
            <DropdownMenu.Item onSelect={() => workersActions.startRename(worker)}>
              {m.workers_rename()}
            </DropdownMenu.Item>
            <DropdownMenu.Item variant="destructive" onSelect={() => workersActions.startRemove(worker)}>
              {m.workers_remove()}
            </DropdownMenu.Item>
          </DropdownMenu.Group>
        </DropdownMenu.Content>
      </DropdownMenu.Root>
    </div>
  </div>
</Sidebar.MenuItem>
