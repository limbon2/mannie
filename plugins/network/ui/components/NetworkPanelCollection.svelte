<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Collapsible from '@mannie/plugin-sdk/components/collapsible';
  import * as DropdownMenu from '@mannie/plugin-sdk/components/dropdown-menu';
  import * as Sidebar from '@mannie/plugin-sdk/components/sidebar';
  import ChevronDownIcon from '@mannie/plugin-sdk/icons/chevron-down';
  import CopyIcon from '@mannie/plugin-sdk/icons/copy';
  import EllipsisIcon from '@mannie/plugin-sdk/icons/ellipsis';
  import FolderIcon from '@mannie/plugin-sdk/icons/folder';
  import FolderPlusIcon from '@mannie/plugin-sdk/icons/folder-plus';
  import GlobeIcon from '@mannie/plugin-sdk/icons/globe';
  import LayersIcon from '@mannie/plugin-sdk/icons/layers';
  import PencilIcon from '@mannie/plugin-sdk/icons/pencil';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import TrashIcon from '@mannie/plugin-sdk/icons/trash';

  import type { CollectionView, FolderView, RequestView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { networkDialogs } from '../state/network-dialogs.svelte.js';
  import { networkLibrary } from '../state/network.svelte.js';
  import { openCollection, openRequest, shownCollectionId, shownRequestId } from '../utils/network-places.js';

  import MethodBadge from './MethodBadge.svelte';

  interface Props {
    collection: CollectionView;
    query: string;
  }

  let { collection, query }: Props = $props();

  const normalizedQuery = $derived(query.trim().toLowerCase());
  const isCollectionActive = $derived(shownCollectionId() === collection.id && shownRequestId() === null);

  const groupedRequests = $derived.by(() => {
    const root: RequestView[] = [];
    const byFolder: Partial<Record<string, RequestView[]>> = {};

    for (const req of collection.requests) {
      if (!matchesRequest(req, normalizedQuery)) {
        continue;
      }
      if (req.folderId === null) {
        root.push(req);
      } else {
        const list = byFolder[req.folderId];
        if (list === undefined) {
          byFolder[req.folderId] = [req];
        } else {
          list.push(req);
        }
      }
    }

    return { byFolder, root };
  });

  const rootRequests = $derived(groupedRequests.root);
  const visibleFolders = $derived.by(() => {
    if (normalizedQuery === '') {
      return collection.folders;
    }
    return collection.folders.filter((folder) => folderMatches(folder, normalizedQuery));
  });

  function matchesRequest(req: RequestView, needle: string): boolean {
    if (needle === '') {
      return true;
    }
    return (
      req.name.toLowerCase().includes(needle) ||
      req.urlTemplate.toLowerCase().includes(needle) ||
      req.method.toLowerCase().includes(needle)
    );
  }

  function folderRequests(folderId: string): RequestView[] {
    return groupedRequests.byFolder[folderId] ?? [];
  }

  function folderMatches(folder: FolderView, needle: string): boolean {
    if (folder.name.toLowerCase().includes(needle)) {
      return true;
    }
    return folderRequests(folder.id).length > 0;
  }

  async function createQuickRequest(folderId: string | null): Promise<void> {
    const outcome = await networkLibrary.createRequest(collection.id, {
      auth: { kind: 'inherit' },
      body: { kind: 'none' },
      description: '',
      folderId,
      headers: [],
      method: 'GET',
      name: m.network_panel_new_request(),
      pathParams: [],
      protocol: 'http',
      queryParams: [],
      timeoutSeconds: 30,
      urlTemplate: '/',
      useCookies: true,
    });
    if (outcome.ok) {
      networkDialogs.setNodeOpen(collection.id, true);
      if (folderId !== null) {
        networkDialogs.setNodeOpen(folderId, true);
      }
      await openRequest(collection.id, outcome.data.id);
    }
  }

  async function duplicateItem(request: RequestView): Promise<void> {
    const outcome = await networkLibrary.duplicateRequest(request.id);
    if (outcome.ok) {
      await openRequest(collection.id, outcome.data.id);
    }
  }

  async function toggleEnabled(): Promise<void> {
    await networkLibrary.setCollectionEnabled(collection.id, !collection.enabled);
  }

  function folderKey(folder: FolderView): string {
    return `folder:${folder.id}`;
  }
</script>

<Collapsible.Root
  bind:open={
    () => networkDialogs.isNodeOpen(collection.id, true),
    (open: boolean) => networkDialogs.setNodeOpen(collection.id, open)
  }
>
  {#snippet child({ props })}
    <Sidebar.MenuItem {...props} data-network-panel-collection={collection.id}>
      <div class="group/col relative flex items-center">
        <Sidebar.MenuButton
          class="pe-14"
          isActive={isCollectionActive}
          title={collection.name}
          onclick={() => void openCollection(collection.id)}
        >
          <GlobeIcon class={collection.enabled ? '' : 'opacity-45'} />
          <span class="truncate" data-content="" dir="auto">{collection.name}</span>
          {#if !collection.enabled}
            <Badge class="ms-auto h-4 px-1 text-[10px]" variant="secondary">
              {m.network_disabled()}
            </Badge>
          {/if}
        </Sidebar.MenuButton>

        <div class="absolute inset-y-0 inset-e-1 flex items-center gap-0.5">
          <DropdownMenu.Root>
            <DropdownMenu.Trigger>
              {#snippet child({ props: trigger })}
                <Button
                  {...trigger}
                  aria-label={m.network_panel_collection_menu({ name: collection.name })}
                  size="icon-xs"
                  type="button"
                  variant="ghost"
                >
                  <EllipsisIcon class="size-3.5" />
                </Button>
              {/snippet}
            </DropdownMenu.Trigger>

            <DropdownMenu.Content class="w-max max-w-80 min-w-44" align="end">
              <DropdownMenu.Item onclick={() => void createQuickRequest(null)}>
                <PlusIcon />
                {m.network_panel_new_request()}
              </DropdownMenu.Item>

              <DropdownMenu.Item onclick={() => networkDialogs.startCreateFolder(collection.id)}>
                <FolderPlusIcon />
                {m.network_panel_new_folder()}
              </DropdownMenu.Item>

              <DropdownMenu.Item onclick={() => networkDialogs.openEnvironments(collection.id)}>
                <LayersIcon />
                {m.network_panel_environments()}
              </DropdownMenu.Item>
              <DropdownMenu.Separator />
              <DropdownMenu.Item onclick={() => networkDialogs.startEditCollection(collection)}>
                <PencilIcon />
                {m.network_edit()}
              </DropdownMenu.Item>
              <DropdownMenu.Item onclick={() => void toggleEnabled()}>
                {collection.enabled ? m.network_panel_disable_for_agents() : m.network_panel_enable_for_agents()}
              </DropdownMenu.Item>
              <DropdownMenu.Separator />
              <DropdownMenu.Item
                variant="destructive"
                onclick={() =>
                  networkDialogs.confirmDelete({
                    id: collection.id,
                    kind: 'collection',
                    name: collection.name,
                  })}
              >
                <TrashIcon />
                {m.network_delete()}
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Root>

          <Collapsible.Trigger>
            {#snippet child({ props: trigger })}
              <Button
                {...trigger}
                class="group/toggle"
                aria-label={m.network_panel_toggle_collection({ name: collection.name })}
                size="icon-xs"
                type="button"
                variant="ghost"
              >
                <ChevronDownIcon
                  class="size-3.5 transition-transform group-data-[state=closed]/toggle:-rotate-90 rtl:group-data-[state=closed]/toggle:rotate-90"
                />
              </Button>
            {/snippet}
          </Collapsible.Trigger>
        </div>
      </div>

      <Collapsible.Content>
        <Sidebar.MenuSub>
          {#each visibleFolders as folder (folder.id)}
            {const items = $derived(folderRequests(folder.id))}
            <Collapsible.Root
              bind:open={
                () => networkDialogs.isNodeOpen(folderKey(folder), true),
                (open: boolean) => networkDialogs.setNodeOpen(folderKey(folder), open)
              }
            >
              {#snippet child({ props: folderProps })}
                <Sidebar.MenuSubItem {...folderProps}>
                  <div class="group/folder relative flex items-center">
                    <Collapsible.Trigger>
                      {#snippet child({ props: trigger })}
                        <button
                          {...trigger}
                          class="group/ftoggle flex w-full items-center gap-1.5 rounded-md px-2 py-1 pe-8 text-xs font-medium text-sidebar-foreground/80 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
                          aria-label={m.network_panel_toggle_folder({ name: folder.name })}
                          title={folder.name}
                          type="button"
                        >
                          <ChevronDownIcon
                            class="size-3 shrink-0 transition-transform group-data-[state=closed]/ftoggle:-rotate-90 rtl:group-data-[state=closed]/ftoggle:rotate-90"
                          />
                          <FolderIcon class="size-3.5 shrink-0" />
                          <span class="truncate" data-content="" dir="auto">{folder.name}</span>
                        </button>
                      {/snippet}
                    </Collapsible.Trigger>

                    <div class="absolute inset-y-0 inset-e-0.5 flex items-center">
                      <DropdownMenu.Root>
                        <DropdownMenu.Trigger>
                          {#snippet child({ props: trigger })}
                            <Button
                              {...trigger}
                              aria-label={m.network_panel_folder_menu({ name: folder.name })}
                              size="icon-xs"
                              type="button"
                              variant="ghost"
                            >
                              <EllipsisIcon class="size-3" />
                            </Button>
                          {/snippet}
                        </DropdownMenu.Trigger>

                        <DropdownMenu.Content class="w-max max-w-80 min-w-44" align="end">
                          <DropdownMenu.Item onclick={() => void createQuickRequest(folder.id)}>
                            <PlusIcon />
                            {m.network_panel_new_request()}
                          </DropdownMenu.Item>

                          <DropdownMenu.Item onclick={() => networkDialogs.startRenameFolder(folder)}>
                            <PencilIcon />
                            {m.network_rename()}
                          </DropdownMenu.Item>
                          <DropdownMenu.Separator />
                          <DropdownMenu.Item
                            variant="destructive"
                            onclick={() =>
                              networkDialogs.confirmDelete({
                                id: folder.id,
                                kind: 'folder',
                                name: folder.name,
                              })}
                          >
                            <TrashIcon />
                            {m.network_delete()}
                          </DropdownMenu.Item>
                        </DropdownMenu.Content>
                      </DropdownMenu.Root>
                    </div>
                  </div>

                  <Collapsible.Content>
                    <div class="ms-3 flex flex-col gap-0.5 border-s border-sidebar-border ps-1.5">
                      {#each items as request (request.id)}
                        <div class="group/req relative flex items-center">
                          <Sidebar.MenuSubButton class="w-full pe-7" isActive={shownRequestId() === request.id}>
                            {#snippet child({ props: button })}
                              <button
                                {...button}
                                data-network-panel-request={request.id}
                                title={request.name}
                                type="button"
                                onclick={() => void openRequest(collection.id, request.id)}
                              >
                                <MethodBadge class="px-1 py-0 text-[9px]" method={request.method} />
                                <span class="truncate" data-content="" dir="auto">
                                  {request.name}
                                </span>
                              </button>
                            {/snippet}
                          </Sidebar.MenuSubButton>

                          <div
                            class="absolute inset-y-0 inset-e-0.5 flex items-center group-focus-within/req:opacity-100 group-hover/req:opacity-100 md:opacity-0"
                          >
                            <DropdownMenu.Root>
                              <DropdownMenu.Trigger>
                                {#snippet child({ props: trigger })}
                                  <Button
                                    {...trigger}
                                    aria-label={m.network_panel_request_menu({ name: request.name })}
                                    size="icon-xs"
                                    type="button"
                                    variant="ghost"
                                  >
                                    <EllipsisIcon class="size-3" />
                                  </Button>
                                {/snippet}
                              </DropdownMenu.Trigger>

                              <DropdownMenu.Content class="w-max max-w-80 min-w-44" align="end">
                                <DropdownMenu.Item onclick={() => void duplicateItem(request)}>
                                  <CopyIcon />
                                  {m.network_duplicate()}
                                </DropdownMenu.Item>

                                <DropdownMenu.Item
                                  variant="destructive"
                                  onclick={() =>
                                    networkDialogs.confirmDelete({
                                      id: request.id,
                                      kind: 'request',
                                      name: request.name,
                                    })}
                                >
                                  <TrashIcon />
                                  {m.network_delete()}
                                </DropdownMenu.Item>
                              </DropdownMenu.Content>
                            </DropdownMenu.Root>
                          </div>
                        </div>
                      {/each}
                    </div>
                  </Collapsible.Content>
                </Sidebar.MenuSubItem>
              {/snippet}
            </Collapsible.Root>
          {/each}

          {#each rootRequests as request (request.id)}
            <Sidebar.MenuSubItem class="group/req relative">
              <Sidebar.MenuSubButton class="w-full pe-7" isActive={shownRequestId() === request.id}>
                {#snippet child({ props: button })}
                  <button
                    {...button}
                    data-network-panel-request={request.id}
                    title={request.name}
                    type="button"
                    onclick={() => void openRequest(collection.id, request.id)}
                  >
                    <MethodBadge class="px-1 py-0 text-[9px]" method={request.method} />
                    <span class="truncate" data-content="" dir="auto">{request.name}</span>
                  </button>
                {/snippet}
              </Sidebar.MenuSubButton>

              <div
                class="absolute inset-y-0 inset-e-0.5 flex items-center group-focus-within/req:opacity-100 group-hover/req:opacity-100 md:opacity-0"
              >
                <DropdownMenu.Root>
                  <DropdownMenu.Trigger>
                    {#snippet child({ props: trigger })}
                      <Button
                        {...trigger}
                        aria-label={m.network_panel_request_menu({ name: request.name })}
                        size="icon-xs"
                        type="button"
                        variant="ghost"
                      >
                        <EllipsisIcon class="size-3" />
                      </Button>
                    {/snippet}
                  </DropdownMenu.Trigger>

                  <DropdownMenu.Content class="w-max max-w-80 min-w-44" align="end">
                    <DropdownMenu.Item onclick={() => void duplicateItem(request)}>
                      <CopyIcon />
                      {m.network_duplicate()}
                    </DropdownMenu.Item>

                    <DropdownMenu.Item
                      variant="destructive"
                      onclick={() =>
                        networkDialogs.confirmDelete({
                          id: request.id,
                          kind: 'request',
                          name: request.name,
                        })}
                    >
                      <TrashIcon />
                      {m.network_delete()}
                    </DropdownMenu.Item>
                  </DropdownMenu.Content>
                </DropdownMenu.Root>
              </div>
            </Sidebar.MenuSubItem>
          {/each}
        </Sidebar.MenuSub>
      </Collapsible.Content>
    </Sidebar.MenuItem>
  {/snippet}
</Collapsible.Root>
