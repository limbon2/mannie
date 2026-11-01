<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as DropdownMenu from '@mannie/plugin-sdk/components/dropdown-menu';
  import * as Sidebar from '@mannie/plugin-sdk/components/sidebar';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import DatabaseIcon from '@mannie/plugin-sdk/icons/database';
  import EllipsisIcon from '@mannie/plugin-sdk/icons/ellipsis';
  import LockIcon from '@mannie/plugin-sdk/icons/lock';

  import type { DatabaseView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { databaseActions } from '../state/database-actions.svelte.js';
  import { databaseStateLabel } from '../utils/databases-messages.js';
  import { openDatabase, shownDatabaseKey } from '../utils/databases-places.js';

  // One database in the sidebar (plugins/workers/docs/databases.md, the
  // view): its name, which the owner gave and so is content, a mark when it
  // is read-only, and its state when it cannot run statements. The row opens
  // the database's page; its menu renames, marks, accepts a replaced file
  // and removes. The menu's button lies over room that the row keeps free,
  // so nothing moves when it shows.
  let { database }: { database: DatabaseView } = $props();

  const usable = $derived(database.state === 'available');
</script>

<Sidebar.MenuItem data-database={database.key}>
  <div class="group/database relative">
    <Sidebar.MenuButton
      class="pe-8 group-hover/database:bg-sidebar-accent group-hover/database:text-sidebar-accent-foreground"
      data-database-row=""
      isActive={shownDatabaseKey() === database.key}
      onclick={() => void openDatabase(database.key)}
    >
      <DatabaseIcon />
      <span class="min-w-0 truncate" data-content dir="auto">{database.name}</span>
      {#if !usable}
        <CircleAlertIcon class="ms-auto text-destructive" />
        <span class="sr-only">{databaseStateLabel(database.state)}</span>
      {:else if database.readOnly}
        <LockIcon class="ms-auto text-muted-foreground" />
        <span class="sr-only">{m.workers_databases_read_only()}</span>
      {/if}
    </Sidebar.MenuButton>
    <div
      class="absolute inset-y-0 inset-e-1 flex items-center group-focus-within/database:opacity-100 group-hover/database:opacity-100 has-aria-expanded:opacity-100 md:opacity-0"
    >
      <DropdownMenu.Root>
        <DropdownMenu.Trigger>
          {#snippet child({ props: trigger })}
            <Button {...trigger} data-database-menu="" size="icon-xs" variant="ghost">
              <EllipsisIcon class="size-4" />
              <span class="sr-only">{m.workers_databases_row_actions({ name: database.name })}</span>
            </Button>
          {/snippet}
        </DropdownMenu.Trigger>
        <DropdownMenu.Content align="start" side="right">
          <DropdownMenu.Group>
            <DropdownMenu.Item onSelect={() => databaseActions.startRename(database)}>
              {m.workers_databases_rename()}
            </DropdownMenu.Item>
            <DropdownMenu.Item onSelect={() => void databaseActions.mark(database, !database.readOnly)}>
              {database.readOnly ? m.workers_databases_allow_changes() : m.workers_databases_mark_read_only()}
            </DropdownMenu.Item>
            {#if database.state === 'changed'}
              <DropdownMenu.Item onSelect={() => void databaseActions.accept(database)}>
                {m.workers_databases_accept()}
              </DropdownMenu.Item>
            {/if}
            <DropdownMenu.Item variant="destructive" onSelect={() => databaseActions.startRemove(database)}>
              {m.workers_databases_remove()}
            </DropdownMenu.Item>
          </DropdownMenu.Group>
        </DropdownMenu.Content>
      </DropdownMenu.Root>
    </div>
  </div>
</Sidebar.MenuItem>
