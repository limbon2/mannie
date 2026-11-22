<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Select from '@mannie/plugin-sdk/components/select';
  import { Switch } from '@mannie/plugin-sdk/components/switch';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import LayersIcon from '@mannie/plugin-sdk/icons/layers';
  import PencilIcon from '@mannie/plugin-sdk/icons/pencil';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import TrashIcon from '@mannie/plugin-sdk/icons/trash';

  import type { CollectionView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { networkDialogs } from '../state/network-dialogs.svelte.js';
  import { networkLibrary } from '../state/network.svelte.js';
  import type { NetworkFailure } from '../types/network.js';
  import { authKindLabel, networkFailureMessage } from '../utils/network-messages.js';
  import { openRequest } from '../utils/network-places.js';

  import MethodBadge from './MethodBadge.svelte';
  import TokenizedUrl from './TokenizedUrl.svelte';

  interface Props {
    collection: CollectionView;
  }

  const NO_ENVIRONMENT = '__none__';

  let { collection }: Props = $props();

  let actionFailure = $state<NetworkFailure | null>(null);
  let busy = $state(false);

  const activeEnvLabel = $derived(
    collection.environments.find((env) => env.id === collection.activeEnvironmentId)?.name ??
      m.network_collection_no_environment(),
  );

  async function toggleEnabled(enabled: boolean): Promise<void> {
    busy = true;
    actionFailure = null;
    const outcome = await networkLibrary.setCollectionEnabled(collection.id, enabled);

    busy = false;
    if (!outcome.ok) {
      actionFailure = outcome.failure;
    }
  }

  async function selectEnvironment(raw: string): Promise<void> {
    const environmentId = raw === NO_ENVIRONMENT ? null : raw;

    busy = true;
    actionFailure = null;
    const outcome = await networkLibrary.setActiveEnvironment(collection.id, environmentId);

    busy = false;
    if (!outcome.ok) {
      actionFailure = outcome.failure;
    }
  }

  async function createNewRequest(): Promise<void> {
    busy = true;
    actionFailure = null;
    const outcome = await networkLibrary.createRequest(collection.id, {
      auth: { kind: 'inherit' },
      body: { kind: 'none' },
      description: '',
      folderId: null,
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

    busy = false;
    if (outcome.ok) {
      await openRequest(collection.id, outcome.data.id);
    } else {
      actionFailure = outcome.failure;
    }
  }
</script>

<div class="mx-auto flex w-full max-w-5xl flex-col gap-6 p-6" data-network-collection={collection.id}>
  <div class="flex flex-wrap items-start justify-between gap-4">
    <div class="flex min-w-0 flex-col gap-1">
      <div class="flex flex-wrap items-center gap-2">
        <h1 class="truncate text-xl font-semibold" data-content="" dir="auto" title={collection.name}>
          {collection.name}
        </h1>
        <Badge class="rounded-sm font-mono text-[10px] uppercase" variant="secondary">
          {m.network_protocol_http()}
        </Badge>
        <Badge variant={collection.enabled ? 'default' : 'outline'}>
          {collection.enabled ? m.network_enabled() : m.network_disabled()}
        </Badge>
      </div>

      {#if collection.description.trim() !== ''}
        <p class="text-sm text-muted-foreground" data-content="" dir="auto">
          {collection.description}
        </p>
      {/if}
    </div>

    <div class="flex flex-wrap items-center gap-2">
      <Button disabled={busy} size="sm" type="button" onclick={() => void createNewRequest()}>
        <PlusIcon data-icon="inline-start" />
        {m.network_panel_new_request()}
      </Button>

      <Button size="sm" type="button" variant="outline" onclick={() => networkDialogs.openEnvironments(collection.id)}>
        <LayersIcon data-icon="inline-start" />
        {m.network_panel_environments()}
      </Button>

      <Button
        data-network-collection-edit=""
        size="sm"
        type="button"
        variant="outline"
        onclick={() => networkDialogs.startEditCollection(collection)}
      >
        <PencilIcon data-icon="inline-start" />
        {m.network_edit()}
      </Button>

      <Button
        size="sm"
        type="button"
        variant="outline"
        onclick={() =>
          networkDialogs.confirmDelete({
            id: collection.id,
            kind: 'collection',
            name: collection.name,
          })}
      >
        <TrashIcon data-icon="inline-start" />
        {m.network_delete()}
      </Button>
    </div>
  </div>

  {#if actionFailure !== null}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Description>{networkFailureMessage(actionFailure)}</Alert.Description>
    </Alert.Root>
  {/if}

  <div class="grid gap-4 md:grid-cols-2">
    <section class="flex flex-col gap-3 rounded-lg border border-border bg-card p-4 text-sm">
      <h2 class="text-sm font-semibold">{m.network_collection_defaults_heading()}</h2>
      <div class="flex flex-col gap-1">
        <span class="text-xs font-medium text-muted-foreground">
          {m.network_collection_base_url()}
        </span>
        {#if collection.baseUrl.trim() !== ''}
          <TokenizedUrl class="text-xs break-all" value={collection.baseUrl} />
        {:else}
          <span class="font-mono text-xs text-muted-foreground">{m.network_none()}</span>
        {/if}
      </div>

      <div class="flex flex-col gap-1">
        <span class="text-xs font-medium text-muted-foreground">
          {m.network_collection_active_environment()}
        </span>
        <Select.Root
          disabled={busy}
          type="single"
          value={collection.activeEnvironmentId ?? NO_ENVIRONMENT}
          onValueChange={(value: string) => void selectEnvironment(value)}
        >
          <Select.Trigger class="h-8 w-full max-w-xs text-xs">
            {activeEnvLabel}
          </Select.Trigger>
          <Select.Content>
            <Select.Item label={m.network_collection_no_environment()} value={NO_ENVIRONMENT}>
              {m.network_collection_no_environment()}
            </Select.Item>
            {#each collection.environments as env (env.id)}
              <Select.Item label={env.name} value={env.id}>
                <span data-content="" dir="auto">{env.name}</span>
              </Select.Item>
            {/each}
          </Select.Content>
        </Select.Root>
      </div>

      <div class="flex items-center justify-between gap-3 pt-1">
        <div class="flex flex-col">
          <span class="text-xs font-medium">{m.network_collection_agents_label()}</span>
          <span class="text-xs text-muted-foreground">
            {m.network_collection_agents_description()}
          </span>
        </div>
        <Switch
          aria-label={m.network_collection_agents_label()}
          checked={collection.enabled}
          disabled={busy}
          onCheckedChange={(checked: boolean) => void toggleEnabled(checked)}
        />
      </div>
    </section>

    <section class="flex flex-col gap-3 rounded-lg border border-border bg-card p-4">
      <h2 class="text-sm font-semibold">{m.network_collection_summary_auth()}</h2>
      <div class="grid grid-cols-2 gap-3 text-xs">
        <div class="flex flex-col gap-0.5">
          <span class="text-muted-foreground">{m.network_collection_summary_auth()}</span>
          <span class="font-medium">{authKindLabel(collection.auth.kind)}</span>
        </div>

        <div class="flex flex-col gap-0.5">
          <span class="text-muted-foreground">{m.network_collection_summary_variables()}</span>
          <span class="font-medium" data-content="">{collection.variables.length}</span>
        </div>

        <div class="flex flex-col gap-0.5">
          <span class="text-muted-foreground">{m.network_collection_summary_headers()}</span>
          <span class="font-medium" data-content="">{collection.headers.length}</span>
        </div>

        <div class="flex flex-col gap-0.5">
          <span class="text-muted-foreground">{m.network_collection_summary_query()}</span>
          <span class="font-medium" data-content="">{collection.queryParams.length}</span>
        </div>
      </div>
    </section>
  </div>

  <div class="flex flex-col gap-3">
    <h2 class="text-sm font-semibold">{m.network_collection_requests_heading()}</h2>
    {#if collection.requests.length === 0}
      <p class="text-sm text-muted-foreground">{m.network_collection_requests_empty()}</p>
    {:else}
      <div class="divide-y divide-border rounded-lg border border-border bg-card">
        {#each collection.requests as req (req.id)}
          <button
            class="flex w-full items-center gap-3 px-4 py-2.5 text-start transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            title={req.name}
            type="button"
            onclick={() => void openRequest(collection.id, req.id)}
          >
            <MethodBadge method={req.method} />
            <span class="min-w-0 flex-1 truncate text-sm font-medium" data-content="" dir="auto">
              {req.name}
            </span>
            <TokenizedUrl class="max-w-md truncate text-xs text-muted-foreground" value={req.urlTemplate} />
          </button>
        {/each}
      </div>
    {/if}
  </div>
</div>
