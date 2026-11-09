<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Collapsible from '@mannie/plugin-sdk/components/collapsible';
  import * as DropdownMenu from '@mannie/plugin-sdk/components/dropdown-menu';
  import * as Sidebar from '@mannie/plugin-sdk/components/sidebar';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import DatabaseIcon from '@mannie/plugin-sdk/icons/database';
  import EllipsisIcon from '@mannie/plugin-sdk/icons/ellipsis';
  import EyeIcon from '@mannie/plugin-sdk/icons/eye';
  import LockIcon from '@mannie/plugin-sdk/icons/lock';
  import TableIcon from '@mannie/plugin-sdk/icons/table';

  import type { DatabaseView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { databaseActions } from '../state/database-actions.svelte.js';
  import { databaseSidebar } from '../state/database-sidebar.svelte.js';
  import { databaseStructures } from '../state/database-structures.svelte.js';
  import {
    databasesFailureMessage,
    databaseStatusLabel,
    isUsable,
    tableKindLabel,
  } from '../utils/databases-messages.js';
  import { openDatabase, shownDatabaseKey, shownTable, tableTab } from '../utils/databases-places.js';

  // One database in the sidebar (plugins/workers/docs/databases.md, the
  // view): its name, which the owner gave and so is content, a mark when it
  // is read-only, and why it cannot run statements, such as its worker being
  // offline (ADR-0057). Below it, as a project's conversations are below the
  // project, its tables and views, each of which opens on the Data tab, or
  // on the Structure tab while that shows; shadow tables, which virtual
  // tables keep their data in, are left out. Activating the row opens the
  // database's page and shows its tables, and on the page that shows,
  // collapses or expands them. Its menu renames, marks, accepts a replaced
  // file and removes. The menu's button lies over room that the row keeps
  // free, so nothing moves when it shows. Table names are content.
  let { database }: { database: DatabaseView } = $props();

  const usable = $derived(isUsable(database));
  const replaced = $derived(database.state === 'changed' && database.reach === 'reachable');
  const isShown = $derived(shownDatabaseKey() === database.key);
  const open = $derived(databaseSidebar.isOpen(database.key, isShown));
  const read = $derived(databaseStructures.get(database.key));
  const tables = $derived((read.structure?.tables ?? []).filter((table) => table.kind !== 'shadow'));
  /** The table that the page shows: the one the address names, or the first. */
  const shownTableName = $derived(shownTable() ?? tables.at(0)?.name ?? null);

  function setOpen(value: boolean): void {
    if (isShown) {
      databaseSidebar.setOpen(database.key, value);
      return;
    }
    databaseSidebar.setOpen(database.key, true);
    void openDatabase(database.key);
  }

  $effect(() => {
    if (open && usable) {
      databaseStructures.ensure(database.key);
    }
  });
</script>

<Collapsible.Root bind:open={() => open, setOpen}>
  {#snippet child({ props })}
    <Sidebar.MenuItem {...props} data-database={database.key}>
      <div class="group/database relative">
        <Collapsible.Trigger>
          {#snippet child({ props: trigger }: { props: Record<string, unknown> })}
            {const buttonProps = $derived(usable ? trigger : { onclick: () => void openDatabase(database.key) })}
            <Sidebar.MenuButton
              {...buttonProps}
              class="pe-8 group-hover/database:bg-sidebar-accent group-hover/database:text-sidebar-accent-foreground"
              data-database-row=""
              isActive={isShown && !usable}
            >
              <DatabaseIcon />
              <span class="min-w-0 truncate" data-content dir="auto">{database.name}</span>
              {#if !usable}
                <CircleAlertIcon class="ms-auto text-destructive" />
                <span class="sr-only">{databaseStatusLabel(database)}</span>
              {:else if database.readOnly}
                <LockIcon class="ms-auto text-muted-foreground" />
                <span class="sr-only">{m.workers_databases_read_only()}</span>
              {/if}
            </Sidebar.MenuButton>
          {/snippet}
        </Collapsible.Trigger>
        <div
          class="absolute inset-y-0 inset-e-1 flex items-center group-focus-within/database:opacity-100 group-hover/database:opacity-100 has-aria-expanded:opacity-100 md:opacity-0"
        >
          <DropdownMenu.Root>
            <DropdownMenu.Trigger>
              {#snippet child({ props: menuTrigger })}
                <Button {...menuTrigger} data-database-menu="" size="icon-xs" variant="ghost">
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
                {#if replaced}
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
      {#if usable}
        <Collapsible.Content>
          <Sidebar.MenuSub data-database-tables="">
            {#if read.structure === null}
              {#if read.failure === null}
                <li class="px-2 py-1 text-xs text-muted-foreground">{m.workers_databases_structure_loading()}</li>
              {:else}
                <li class="flex flex-col items-start gap-2 px-2 py-1 text-xs" role="alert">
                  <span class="text-destructive">{databasesFailureMessage(read.failure)}</span>
                  <Button size="sm" variant="outline" onclick={() => void databaseStructures.load(database.key)}>
                    {m.workers_databases_retry()}
                  </Button>
                </li>
              {/if}
            {:else if tables.length === 0}
              <li class="px-2 py-1 text-xs text-muted-foreground" data-database-tables-empty="">
                {m.workers_databases_tables_empty()}
              </li>
            {:else}
              {#each tables as table (table.name)}
                <Sidebar.MenuSubItem data-database-table={table.name}>
                  <Sidebar.MenuSubButton isActive={isShown && shownTableName === table.name}>
                    {#snippet child({ props: button })}
                      <button
                        {...button}
                        title={tableKindLabel(table.kind)}
                        type="button"
                        onclick={() => void openDatabase(database.key, tableTab(), table.name)}
                      >
                        {#if table.kind === 'view'}
                          <EyeIcon />
                        {:else}
                          <TableIcon />
                        {/if}
                        <span class="truncate" data-content dir="auto">{table.name}</span>
                      </button>
                    {/snippet}
                  </Sidebar.MenuSubButton>
                </Sidebar.MenuSubItem>
              {/each}
            {/if}
          </Sidebar.MenuSub>
        </Collapsible.Content>
      {/if}
    </Sidebar.MenuItem>
  {/snippet}
</Collapsible.Root>
