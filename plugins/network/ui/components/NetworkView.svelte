<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import { Skeleton } from '@mannie/plugin-sdk/components/skeleton';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import GlobeIcon from '@mannie/plugin-sdk/icons/globe';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import UploadIcon from '@mannie/plugin-sdk/icons/upload';

  import { m } from '../generated/messages.js';
  import { networkDialogs } from '../state/network-dialogs.svelte.js';
  import { networkLibrary } from '../state/network.svelte.js';
  import { networkFailureMessage } from '../utils/network-messages.js';
  import { openCollection, shownCollectionId, shownRequestId } from '../utils/network-places.js';

  import CollectionOverview from './CollectionOverview.svelte';
  import RequestWorkbench from './RequestWorkbench.svelte';

  const collectionId = $derived(shownCollectionId());
  const requestId = $derived(shownRequestId());
  const isBare = $derived(collectionId === null && requestId === null);
  const isReady = $derived(networkLibrary.status === 'ready');
  const isLoading = $derived(networkLibrary.status === 'loading');

  const selectedCollection = $derived(collectionId === null ? undefined : networkLibrary.findCollection(collectionId));
  const selectedRequest = $derived(
    requestId === null ? undefined : networkLibrary.findRequest(requestId, collectionId ?? undefined),
  );
  const fallbackCollection = $derived(isBare && isReady ? networkLibrary.latestCollection() : undefined);
  const waits = $derived(isLoading || fallbackCollection !== undefined);

  $effect(() => {
    if (fallbackCollection !== undefined) {
      void openCollection(fallbackCollection.id, { replace: true });
    }
  });
</script>

{#if networkLibrary.status === 'error'}
  <div class="mx-auto w-full max-w-3xl p-6">
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.network_load_failed_title()}</Alert.Title>
      <Alert.Description>
        {networkLibrary.failure === null ? '' : networkFailureMessage(networkLibrary.failure)}
      </Alert.Description>
      <Alert.Action>
        <Button size="sm" type="button" variant="outline" onclick={() => networkLibrary.retry()}>
          {m.network_try_again()}
        </Button>
      </Alert.Action>
    </Alert.Root>
  </div>
{:else if waits}
  <div class="mx-auto flex w-full max-w-3xl flex-col gap-3 p-6" aria-busy="true">
    <span class="sr-only" role="status">{m.network_loading()}</span>
    <Skeleton class="h-16 w-full" />
    <Skeleton class="h-32 w-full" />
  </div>
{:else if requestId !== null}
  {#if selectedRequest !== undefined}
    <RequestWorkbench collection={selectedRequest.collection} request={selectedRequest.request} />
  {:else}
    <Empty.Root class="flex-1">
      <Empty.Header>
        <Empty.Media variant="icon">
          <GlobeIcon />
        </Empty.Media>
        <Empty.Title>{m.network_request_missing_title()}</Empty.Title>
        <Empty.Description>{m.network_request_missing_description()}</Empty.Description>
      </Empty.Header>
    </Empty.Root>
  {/if}
{:else if collectionId !== null}
  {#if selectedCollection !== undefined}
    <CollectionOverview collection={selectedCollection} />
  {:else}
    <Empty.Root class="flex-1">
      <Empty.Header>
        <Empty.Media variant="icon">
          <GlobeIcon />
        </Empty.Media>
        <Empty.Title>{m.network_collection_missing_title()}</Empty.Title>
        <Empty.Description>{m.network_collection_missing_description()}</Empty.Description>
      </Empty.Header>
    </Empty.Root>
  {/if}
{:else}
  <Empty.Root class="flex-1" data-network-empty="">
    <Empty.Header>
      <Empty.Media variant="icon">
        <GlobeIcon />
      </Empty.Media>
      <Empty.Title>{m.network_empty_title()}</Empty.Title>
      <Empty.Description>{m.network_empty_description()}</Empty.Description>
    </Empty.Header>

    <Empty.Content class="flex flex-wrap items-center justify-center gap-2">
      <Button data-network-new-collection="" type="button" onclick={() => networkDialogs.startCreateCollection()}>
        <PlusIcon data-icon="inline-start" />
        {m.network_new_collection()}
      </Button>

      <Button
        data-network-import-openapi=""
        type="button"
        variant="outline"
        onclick={() => networkDialogs.openImport()}
      >
        <UploadIcon data-icon="inline-start" />
        {m.network_import_openapi()}
      </Button>
    </Empty.Content>
  </Empty.Root>
{/if}
