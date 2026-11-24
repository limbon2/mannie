<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import TrashIcon from '@mannie/plugin-sdk/icons/trash';

  import type { EnvironmentView, VariableEntry } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { networkDialogs } from '../state/network-dialogs.svelte.js';
  import { networkLibrary } from '../state/network.svelte.js';
  import type { NetworkFailure } from '../types/network.js';
  import { networkFailureMessage } from '../utils/network-messages.js';

  import KeyValueEditor from './KeyValueEditor.svelte';

  let editingId = $state<string | null>(null);
  let envName = $state('');
  let variables = $state<VariableEntry[]>([]);
  let saving = $state(false);
  let failure = $state<NetworkFailure | null>(null);

  const collection = $derived(
    networkDialogs.environmentsCollectionId === null
      ? undefined
      : networkLibrary.findCollection(networkDialogs.environmentsCollectionId),
  );
  const open = $derived(collection !== undefined);
  const canActivateEditing = $derived.by(() => {
    if (collection === undefined) {
      return false;
    }
    if (editingId === null) {
      return false;
    }
    return collection.activeEnvironmentId !== editingId;
  });

  function setVariables(next: VariableEntry[]): void {
    variables = next;
  }

  function selectEnvironmentForEdit(env: EnvironmentView): void {
    editingId = env.id;
    envName = env.name;
    variables = env.variables.map((entry) => ({ ...entry }));
    failure = null;
  }

  function startNewEnvironment(): void {
    editingId = null;
    envName = '';
    variables = [];
    failure = null;
  }

  function handleOpenChange(nextOpen: boolean): void {
    if (!nextOpen && !saving) {
      networkDialogs.closeEnvironments();
    }
  }

  async function saveEnvironment(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const hasCollection = collection !== undefined;
    const hasName = envName.trim() !== '';

    if (!hasCollection || !hasName) {
      return;
    }
    saving = true;
    failure = null;
    const outcome = await networkLibrary.saveEnvironment({
      collectionId: collection.id,
      environmentId: editingId,
      name: envName,
      variables,
    });

    saving = false;
    if (outcome.ok) {
      selectEnvironmentForEdit(outcome.data);
    } else {
      failure = outcome.failure;
    }
  }

  async function activateEditingEnvironment(): Promise<void> {
    if (collection === undefined) {
      return;
    }
    if (editingId === null) {
      return;
    }
    saving = true;
    failure = null;
    const outcome = await networkLibrary.setActiveEnvironment(collection.id, editingId);

    saving = false;
    if (!outcome.ok) {
      failure = outcome.failure;
    }
  }

  $effect(() => {
    if (networkDialogs.environmentsCollectionId !== null) {
      editingId = null;
      envName = '';
      variables = [];
      failure = null;
      saving = false;
    }
  });
</script>

<Dialog.Root {open} onOpenChange={handleOpenChange}>
  <Dialog.Content class="flex max-h-[calc(100vh-2rem)] flex-col overflow-hidden sm:max-w-3xl" showCloseButton={false}>
    {#if collection !== undefined}
      <div class="flex min-h-0 flex-1 flex-col gap-4">
        <Dialog.Header class="shrink-0">
          <Dialog.Title>{m.network_environments_title({ name: collection.name })}</Dialog.Title>
          <Dialog.Description>{m.network_environments_description()}</Dialog.Description>
        </Dialog.Header>

        {#if failure !== null}
          <Alert.Root class="shrink-0" variant="destructive">
            <CircleAlertIcon />
            <Alert.Description>{networkFailureMessage(failure)}</Alert.Description>
          </Alert.Root>
        {/if}

        <div class="grid min-h-0 flex-1 gap-4 overflow-y-auto pe-1 md:grid-cols-[200px_1fr]">
          <div class="flex flex-col gap-2 border-e border-border pe-3">
            <Button disabled={saving} size="sm" type="button" variant="outline" onclick={startNewEnvironment}>
              <PlusIcon class="size-3.5" data-icon="inline-start" />
              {m.network_environments_new()}
            </Button>

            {#if collection.environments.length === 0}
              <p class="text-xs text-muted-foreground">{m.network_environments_empty()}</p>
            {:else}
              <div class="flex flex-col gap-1">
                {#each collection.environments as env (env.id)}
                  <div class="flex items-center gap-1">
                    <Button
                      class="flex-1 justify-start truncate text-xs"
                      size="sm"
                      type="button"
                      variant={editingId === env.id ? 'secondary' : 'ghost'}
                      onclick={() => selectEnvironmentForEdit(env)}
                    >
                      <span class="truncate" data-content="" dir="auto">{env.name}</span>
                      {#if collection.activeEnvironmentId === env.id}
                        <Badge class="ms-auto h-4 px-1 text-[10px]" variant="default">
                          {m.network_environments_active_badge()}
                        </Badge>
                      {/if}
                    </Button>

                    <Button
                      aria-label={m.network_delete()}
                      size="icon-xs"
                      type="button"
                      variant="ghost"
                      onclick={() =>
                        networkDialogs.confirmDelete({
                          id: env.id,
                          kind: 'environment',
                          name: env.name,
                        })}
                    >
                      <TrashIcon class="size-3.5" />
                    </Button>
                  </div>
                {/each}
              </div>
            {/if}
          </div>

          <form class="flex flex-col gap-3" onsubmit={(event: SubmitEvent) => void saveEnvironment(event)}>
            <div class="flex flex-wrap items-end gap-2">
              <Field.Field class="flex-1">
                <Field.Label>{m.network_environments_name_label()}</Field.Label>
                <Input
                  disabled={saving}
                  placeholder={m.network_environments_name_placeholder()}
                  required
                  bind:value={envName}
                />
              </Field.Field>

              {#if canActivateEditing}
                <Button
                  disabled={saving}
                  size="sm"
                  type="button"
                  variant="outline"
                  onclick={() => void activateEditingEnvironment()}
                >
                  {m.network_environments_set_active()}
                </Button>
              {/if}
            </div>

            <KeyValueEditor
              disabled={saving}
              entries={variables}
              keyPlaceholder={m.network_kv_variable_key_placeholder()}
              kind="variable"
              valuePlaceholder={m.network_kv_variable_value_placeholder()}
              onChange={setVariables}
            />

            <div class="flex justify-end gap-2 pt-2">
              <Button disabled={saving || envName.trim() === ''} size="sm" type="submit">
                {saving ? m.network_saving() : m.network_save()}
              </Button>
            </div>
          </form>
        </div>

        <Dialog.Footer>
          <Button disabled={saving} type="button" variant="outline" onclick={() => networkDialogs.closeEnvironments()}>
            {m.network_close()}
          </Button>
        </Dialog.Footer>
      </div>
    {/if}
  </Dialog.Content>
</Dialog.Root>
