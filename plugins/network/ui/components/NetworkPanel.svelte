<script lang="ts">
  import * as DropdownMenu from '@mannie/plugin-sdk/components/dropdown-menu';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import * as Sidebar from '@mannie/plugin-sdk/components/sidebar';
  import CookieIcon from '@mannie/plugin-sdk/icons/cookie';
  import EllipsisVerticalIcon from '@mannie/plugin-sdk/icons/ellipsis-vertical';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import SearchIcon from '@mannie/plugin-sdk/icons/search';
  import SlidersHorizontalIcon from '@mannie/plugin-sdk/icons/sliders-horizontal';
  import UploadIcon from '@mannie/plugin-sdk/icons/upload';

  import type { CollectionView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { networkDialogs } from '../state/network-dialogs.svelte.js';
  import { networkLibrary } from '../state/network.svelte.js';

  import NetworkPanelCollection from './NetworkPanelCollection.svelte';

  let query = $state('');

  const normalizedQuery = $derived(query.trim().toLowerCase());
  const filteredCollections = $derived(
    networkLibrary.collections.filter((collection) => collectionMatches(collection, normalizedQuery)),
  );

  function collectionMatches(collection: CollectionView, needle: string): boolean {
    if (needle === '') {
      return true;
    }
    const nameMatches = collection.name.toLowerCase().includes(needle);
    const urlMatches = collection.baseUrl.toLowerCase().includes(needle);

    if (nameMatches || urlMatches) {
      return true;
    }
    if (collection.folders.some((folder) => folder.name.toLowerCase().includes(needle))) {
      return true;
    }
    return collection.requests.some((req) => {
      const matchesName = req.name.toLowerCase().includes(needle);
      const matchesUrl = req.urlTemplate.toLowerCase().includes(needle);
      const matchesMethod = req.method.toLowerCase().includes(needle);

      return matchesName || matchesUrl || matchesMethod;
    });
  }
</script>

<Sidebar.Group class="group-data-[collapsible=icon]:hidden" data-network-panel="">
  <Sidebar.GroupLabel>{m.network_panel_title()}</Sidebar.GroupLabel>

  <DropdownMenu.Root>
    <DropdownMenu.Trigger>
      {#snippet child({ props })}
        <Sidebar.GroupAction
          {...props}
          aria-label={m.network_panel_actions_menu()}
          data-network-panel-actions=""
          title={m.network_panel_actions_menu()}
        >
          <EllipsisVerticalIcon />
          <span class="sr-only">{m.network_panel_actions_menu()}</span>
        </Sidebar.GroupAction>
      {/snippet}
    </DropdownMenu.Trigger>

    <DropdownMenu.Content class="w-max max-w-80 min-w-44" align="end">
      <DropdownMenu.Item data-network-panel-new="" onclick={() => networkDialogs.startCreateCollection()}>
        <PlusIcon />
        {m.network_panel_new_collection()}
      </DropdownMenu.Item>

      <DropdownMenu.Item data-network-panel-import="" onclick={() => networkDialogs.openImport()}>
        <UploadIcon />
        {m.network_panel_import()}
      </DropdownMenu.Item>

      <DropdownMenu.Separator />

      <DropdownMenu.Item data-network-panel-globals="" onclick={() => networkDialogs.openGlobalVariables()}>
        <SlidersHorizontalIcon />
        {m.network_panel_globals()}
      </DropdownMenu.Item>

      <DropdownMenu.Item data-network-panel-cookies="" onclick={() => networkDialogs.openCookieJar()}>
        <CookieIcon />
        {m.network_panel_cookies()}
      </DropdownMenu.Item>
    </DropdownMenu.Content>
  </DropdownMenu.Root>

  <Sidebar.GroupContent class="flex flex-col gap-2">
    {#if networkLibrary.collections.length > 0}
      <div class="relative px-2">
        <SearchIcon
          class="pointer-events-none absolute inset-s-4 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground"
        />
        <Input
          class="h-7 ps-7 text-xs"
          aria-label={m.network_panel_search_label()}
          placeholder={m.network_panel_search_placeholder()}
          bind:value={query}
        />
      </div>
    {/if}

    {#if networkLibrary.status === 'loading'}
      <Sidebar.Menu aria-busy="true">
        {#each [0, 1] as placeholder (placeholder)}
          <Sidebar.MenuItem>
            <Sidebar.MenuSkeleton showIcon />
          </Sidebar.MenuItem>
        {/each}
      </Sidebar.Menu>
    {:else if networkLibrary.status === 'error'}
      <p class="px-2 text-xs text-destructive">{m.network_panel_failed()}</p>
    {:else if networkLibrary.collections.length === 0}
      <p class="px-2 text-xs text-muted-foreground">{m.network_panel_empty()}</p>
    {:else if filteredCollections.length === 0}
      <p class="px-2 text-xs text-muted-foreground">{m.network_panel_no_matches()}</p>
    {:else}
      <Sidebar.Menu>
        {#each filteredCollections as collection (collection.id)}
          <NetworkPanelCollection {collection} {query} />
        {/each}
      </Sidebar.Menu>
    {/if}
  </Sidebar.GroupContent>
</Sidebar.Group>
