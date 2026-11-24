<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import { Switch } from '@mannie/plugin-sdk/components/switch';
  import * as Tabs from '@mannie/plugin-sdk/components/tabs';
  import { Textarea } from '@mannie/plugin-sdk/components/textarea';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import type { AuthConfig, CollectionInput, KeyValueEntry, VariableEntry } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { networkDialogs } from '../state/network-dialogs.svelte.js';
  import { networkLibrary } from '../state/network.svelte.js';
  import type { CollectionDraft, NetworkFailure } from '../types/network.js';
  import { draftFromCollection, emptyCollectionDraft } from '../utils/network-drafts.js';
  import { networkFailureMessage } from '../utils/network-messages.js';
  import { openCollection } from '../utils/network-places.js';
  import { availableVariables } from '../utils/network-urls.js';

  import AuthEditor from './AuthEditor.svelte';
  import KeyValueEditor from './KeyValueEditor.svelte';
  import VariableInput from './VariableInput.svelte';

  let draft = $state<CollectionDraft>(emptyCollectionDraft());
  let tab = $state('general');
  let saving = $state(false);
  let failure = $state<NetworkFailure | null>(null);

  const open = $derived(networkDialogs.collectionDialog !== null);
  const isEdit = $derived(networkDialogs.collectionDialog?.mode === 'edit');
  const submitLabel = $derived.by(() => {
    if (saving) {
      return m.network_saving();
    }
    return isEdit ? m.network_save() : m.network_create();
  });
  const variableSuggestions = $derived(
    availableVariables(
      {
        environments:
          networkDialogs.collectionDialog?.mode === 'edit'
            ? networkDialogs.collectionDialog.collection.environments
            : [],
        variables: draft.variables,
      },
      networkDialogs.collectionDialog?.mode === 'edit'
        ? networkDialogs.collectionDialog.collection.activeEnvironmentId
        : null,
      networkLibrary.globalVariables,
    ),
  );

  function setBaseUrl(next: string): void {
    draft.baseUrl = next;
  }

  function setVariables(next: VariableEntry[]): void {
    draft.variables = next;
  }

  function setHeaders(next: KeyValueEntry[]): void {
    draft.headers = next;
  }

  function setQueryParams(next: KeyValueEntry[]): void {
    draft.queryParams = next;
  }

  function setAuth(next: AuthConfig): void {
    draft.auth = next;
  }

  function handleOpenChange(nextOpen: boolean): void {
    if (!nextOpen && !saving) {
      networkDialogs.closeCollectionDialog();
    }
  }

  async function submitForm(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const dialog = networkDialogs.collectionDialog;
    if (dialog === null) {
      return;
    }

    saving = true;
    failure = null;
    const payload: CollectionInput = {
      auth: draft.auth,
      autoCookies: draft.autoCookies,
      baseUrl: draft.baseUrl,
      description: draft.description,
      enabled: draft.enabled,
      headers: draft.headers,
      name: draft.name,
      protocol: 'http',
      queryParams: draft.queryParams,
      variables: draft.variables,
    };

    const outcome =
      dialog.mode === 'edit'
        ? await networkLibrary.updateCollection(dialog.collection.id, payload)
        : await networkLibrary.createCollection(payload);

    saving = false;
    if (outcome.ok) {
      networkDialogs.closeCollectionDialog();
      await openCollection(outcome.data.id);
    } else {
      failure = outcome.failure;
    }
  }

  $effect(() => {
    const dialog = networkDialogs.collectionDialog;
    if (dialog === null) {
      return;
    }
    draft = dialog.mode === 'edit' ? draftFromCollection(dialog.collection) : emptyCollectionDraft();
    tab = 'general';
    failure = null;
    saving = false;
  });
</script>

<Dialog.Root {open} onOpenChange={handleOpenChange}>
  <Dialog.Content
    class="flex h-[min(39rem,calc(100vh-2rem))] flex-col overflow-hidden sm:max-w-2xl"
    data-network-collection-dialog=""
    showCloseButton={false}
  >
    <form class="flex min-h-0 flex-1 flex-col gap-4" onsubmit={(event: SubmitEvent) => void submitForm(event)}>
      <Dialog.Header class="shrink-0">
        <Dialog.Title>
          {isEdit ? m.network_dialog_collection_edit_title() : m.network_dialog_collection_create_title()}
        </Dialog.Title>
        <Dialog.Description>{m.network_dialog_collection_description()}</Dialog.Description>
      </Dialog.Header>

      {#if failure !== null}
        <Alert.Root class="shrink-0" variant="destructive">
          <CircleAlertIcon />
          <Alert.Description>{networkFailureMessage(failure)}</Alert.Description>
        </Alert.Root>
      {/if}

      <Tabs.Root class="flex min-h-0 flex-1 flex-col" bind:value={tab}>
        <Tabs.List class="w-fit shrink-0">
          <Tabs.Trigger value="general">{m.network_dialog_collection_tab_general()}</Tabs.Trigger>
          <Tabs.Trigger value="variables">
            <span class="inline-flex items-baseline gap-1">
              <span class="font-mono text-[11px] font-semibold text-token-variable-foreground" aria-hidden="true">
                {'{{}}'}
              </span>
              <span>{m.network_dialog_collection_tab_variables()}</span>
            </span>
          </Tabs.Trigger>
          <Tabs.Trigger value="headers">{m.network_dialog_collection_tab_headers()}</Tabs.Trigger>
          <Tabs.Trigger value="query">
            <span class="inline-flex items-baseline gap-1">
              <span class="font-mono text-[11px] font-semibold text-token-query-foreground" aria-hidden="true">?</span>
              <span>{m.network_dialog_collection_tab_query()}</span>
            </span>
          </Tabs.Trigger>
          <Tabs.Trigger value="auth">{m.network_dialog_collection_tab_auth()}</Tabs.Trigger>
        </Tabs.List>

        <Tabs.Content class="mt-3 flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pe-1" value="general">
          <Field.Field>
            <Field.Label for="network-collection-name">{m.network_dialog_collection_name()}</Field.Label>
            <Input
              id="network-collection-name"
              disabled={saving}
              placeholder={m.network_dialog_collection_name_placeholder()}
              required
              bind:value={draft.name}
            />
          </Field.Field>

          <Field.Field>
            <Field.Label for="network-collection-base-url">{m.network_collection_base_url()}</Field.Label>
            <VariableInput
              id="network-collection-base-url"
              class="font-mono text-xs"
              disabled={saving}
              placeholder={m.network_collection_base_url_placeholder()}
              value={draft.baseUrl}
              variables={variableSuggestions}
              onValueChange={setBaseUrl}
            />
          </Field.Field>

          <Field.Field>
            <Field.Label>{m.network_dialog_collection_notes()}</Field.Label>
            <Textarea
              class="min-h-20 text-xs"
              disabled={saving}
              placeholder={m.network_dialog_collection_notes_placeholder()}
              bind:value={draft.description}
            />
          </Field.Field>

          <div class="flex items-center justify-between gap-3 rounded-md border border-border p-3">
            <div class="flex flex-col">
              <span class="text-xs font-medium">{m.network_collection_agents_label()}</span>
              <span class="text-xs text-muted-foreground">
                {m.network_collection_agents_description()}
              </span>
            </div>
            <Switch aria-label={m.network_collection_agents_label()} disabled={saving} bind:checked={draft.enabled} />
          </div>

          <div class="flex items-center justify-between gap-3 rounded-md border border-border p-3">
            <div class="flex flex-col">
              <span class="text-xs font-medium">{m.network_collection_auto_cookies_label()}</span>
              <span class="text-xs text-muted-foreground">
                {m.network_collection_auto_cookies_description()}
              </span>
            </div>
            <Switch
              aria-label={m.network_collection_auto_cookies_label()}
              disabled={saving}
              bind:checked={draft.autoCookies}
            />
          </div>
        </Tabs.Content>

        <Tabs.Content class="mt-3 min-h-0 flex-1 overflow-y-auto pe-1" value="variables">
          <KeyValueEditor
            disabled={saving}
            entries={draft.variables}
            keyPlaceholder={m.network_kv_variable_key_placeholder()}
            kind="variable"
            valuePlaceholder={m.network_kv_variable_value_placeholder()}
            onChange={setVariables}
          />
        </Tabs.Content>

        <Tabs.Content class="mt-3 min-h-0 flex-1 overflow-y-auto pe-1" value="headers">
          <KeyValueEditor
            disabled={saving}
            entries={draft.headers}
            keyPlaceholder={m.network_kv_header_key_placeholder()}
            valuePlaceholder={m.network_kv_header_value_placeholder()}
            variables={variableSuggestions}
            onChange={setHeaders}
          />
        </Tabs.Content>

        <Tabs.Content class="mt-3 min-h-0 flex-1 overflow-y-auto pe-1" value="query">
          <KeyValueEditor
            disabled={saving}
            entries={draft.queryParams}
            keyPlaceholder={m.network_kv_query_key_placeholder()}
            kind="query"
            valuePlaceholder={m.network_kv_query_value_placeholder()}
            variables={variableSuggestions}
            onChange={setQueryParams}
          />
        </Tabs.Content>

        <Tabs.Content class="mt-3 min-h-0 flex-1 overflow-y-auto pe-1" value="auth">
          <AuthEditor auth={draft.auth} disabled={saving} variables={variableSuggestions} onChange={setAuth} />
        </Tabs.Content>
      </Tabs.Root>

      <Dialog.Footer class="shrink-0">
        <Button
          disabled={saving}
          type="button"
          variant="outline"
          onclick={() => networkDialogs.closeCollectionDialog()}
        >
          {m.network_cancel()}
        </Button>
        <Button data-network-collection-submit="" disabled={saving || draft.name.trim() === ''} type="submit">
          {submitLabel}
        </Button>
      </Dialog.Footer>
    </form>
  </Dialog.Content>
</Dialog.Root>
