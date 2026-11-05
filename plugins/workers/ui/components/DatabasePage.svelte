<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import * as Tabs from '@mannie/plugin-sdk/components/tabs';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import { call } from '@mannie/plugin-sdk/ipc';
  import { onMount } from 'svelte';

  import { commands, type DatabaseStructure, type DatabaseView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { databaseActions } from '../state/database-actions.svelte.js';
  import { databaseList } from '../state/databases.svelte.js';
  import type { DatabasesFailure } from '../types/databases.js';
  import {
    databasesFailureMessage,
    databaseStatusLabel,
    isUsable,
    unusableMessage,
  } from '../utils/databases-messages.js';
  import { DATABASE_TABS, openDatabase, shownTab, shownTable } from '../utils/databases-places.js';

  import DatabaseDataTab from './DatabaseDataTab.svelte';
  import DatabaseSqlTab from './DatabaseSqlTab.svelte';
  import DatabaseStructureTab from './DatabaseStructureTab.svelte';
  import DatabaseTablesTab from './DatabaseTablesTab.svelte';

  // A database's page (plugins/workers/docs/databases.md, the view): its
  // name, file, machine and marks, and the tabs Tables, Structure, Data and
  // SQL, which share the database's structure. A database that cannot run
  // statements, such as one whose worker is offline (ADR-0057), says why
  // instead of the tabs, with what the owner can do. The name, the file's
  // name and a worker's name are content.
  let { database }: { database: DatabaseView } = $props();

  let structure: DatabaseStructure | null = $state(null);
  let failure: DatabasesFailure | null = $state(null);
  let loading = $state(false);
  /** Counts reads of the structure, so that a slower earlier one never covers a newer one. */
  let reads = 0;

  const unusable = $derived(unusableMessage(database));
  const replaced = $derived(database.state === 'changed' && database.reach === 'reachable');
  const where = $derived(
    database.machine.kind === 'worker'
      ? m.workers_databases_file_on_worker({ file: database.fileName, worker: database.machine.name })
      : m.workers_databases_file_on_this_computer({ file: database.fileName }),
  );
  const actionFailure = $derived(databaseActions.failures.get(database.key) ?? null);
  const tab = $derived(shownTab());
  /** The failure of the latest read of the structure, while no read runs. */
  const shownFailure = $derived(loading ? null : failure);

  /** Reads the structure again, such as after a change of the schema. */
  async function load(): Promise<void> {
    reads += 1;
    const read = reads;
    loading = true;
    failure = null;
    const outcome = await call(() => commands.databaseStructure({ key: database.key }));
    if (read !== reads) {
      return;
    }
    loading = false;
    if (outcome.ok) {
      structure = outcome.data;
    } else {
      failure = outcome.failure;
    }
  }

  function show(value: string): void {
    const chosen = DATABASE_TABS.find((known) => known === value);
    if (chosen !== undefined) {
      void openDatabase(database.key, chosen, shownTable(), { replace: true });
    }
  }

  onMount(() => {
    // The view mounts the page again when the file's state changes, so a
    // file that became usable, such as after the owner accepted a replaced
    // one, reads its structure.
    if (isUsable(database)) {
      void load();
    }
  });
</script>

<div class="flex min-h-0 flex-1 flex-col" data-database-page={database.key}>
  <header class="flex flex-col gap-1 border-b px-6 py-4">
    <div class="flex flex-wrap items-center gap-2">
      <h1 class="text-lg font-semibold wrap-anywhere" data-content dir="auto">{database.name}</h1>
      {#if database.readOnly}
        <Badge variant="secondary">{m.workers_databases_read_only()}</Badge>
      {/if}
      {#if unusable !== null}
        <Badge variant="destructive">{databaseStatusLabel(database)}</Badge>
      {/if}
    </div>
    <p class="text-sm wrap-anywhere text-muted-foreground" data-content dir="auto">{where}</p>
  </header>
  {#if actionFailure !== null}
    <p class="px-6 pt-3 text-sm text-destructive" role="alert">{databasesFailureMessage(actionFailure)}</p>
  {/if}
  {#if unusable !== null}
    <div class="p-6">
      <Alert.Root data-database-unusable="">
        <CircleAlertIcon />
        <Alert.Title>{databaseStatusLabel(database)}</Alert.Title>
        <Alert.Description>
          <p>{unusable}</p>
          <div class="mt-2 flex flex-wrap gap-2">
            {#if replaced}
              <Button size="sm" onclick={() => void databaseActions.accept(database)}>
                {m.workers_databases_accept()}
              </Button>
            {/if}
            <Button size="sm" variant="outline" onclick={() => void databaseList.refresh()}>
              {m.workers_databases_refresh()}
            </Button>
          </div>
        </Alert.Description>
      </Alert.Root>
    </div>
  {:else if structure === null}
    <div class="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-sm text-muted-foreground">
      {#if shownFailure === null}
        <span class="flex items-center gap-2"><Spinner />{m.workers_databases_structure_loading()}</span>
      {:else}
        <p class="text-destructive" role="alert">{databasesFailureMessage(shownFailure)}</p>
        <Button size="sm" variant="outline" onclick={() => void load()}>{m.workers_databases_retry()}</Button>
      {/if}
    </div>
  {:else}
    <Tabs.Root class="flex min-h-0 flex-1 flex-col gap-0" value={tab} onValueChange={show}>
      <div class="border-b px-6 py-2">
        <Tabs.List>
          <Tabs.Trigger data-database-tab="tables" value="tables">{m.workers_databases_tab_tables()}</Tabs.Trigger>
          <Tabs.Trigger data-database-tab="structure" value="structure">
            {m.workers_databases_tab_structure()}
          </Tabs.Trigger>
          <Tabs.Trigger data-database-tab="data" value="data">{m.workers_databases_tab_data()}</Tabs.Trigger>
          <Tabs.Trigger data-database-tab="sql" value="sql">{m.workers_databases_tab_sql()}</Tabs.Trigger>
        </Tabs.List>
      </div>
      <Tabs.Content class="min-h-0 flex-1 overflow-auto p-6" value="tables">
        <DatabaseTablesTab {database} {structure} />
      </Tabs.Content>
      <Tabs.Content class="min-h-0 flex-1 overflow-auto p-6" value="structure">
        <DatabaseStructureTab {database} {structure} />
      </Tabs.Content>
      <Tabs.Content class="min-h-0 flex-1 overflow-auto p-6" value="data">
        <DatabaseDataTab {database} {structure} />
      </Tabs.Content>
      <Tabs.Content class="min-h-0 flex-1 overflow-auto p-6" value="sql">
        <DatabaseSqlTab {database} onschemachange={() => void load()} />
      </Tabs.Content>
    </Tabs.Root>
  {/if}
</div>
