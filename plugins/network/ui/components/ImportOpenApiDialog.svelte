<script lang="ts">
  import type { CodeEditorLanguage } from '@mannie/plugin-sdk/code-editor';
  import { CodeEditor } from '@mannie/plugin-sdk/components';
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import { Switch } from '@mannie/plugin-sdk/components/switch';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import UploadIcon from '@mannie/plugin-sdk/icons/upload';

  import type { OpenApiPreview } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { networkDialogs } from '../state/network-dialogs.svelte.js';
  import { networkLibrary } from '../state/network.svelte.js';
  import type { NetworkFailure } from '../types/network.js';
  import { formatImportWarning, networkFailureMessage } from '../utils/network-messages.js';
  import { openCollection } from '../utils/network-places.js';
  import { activeImportWarnings, availableVariables } from '../utils/network-urls.js';

  import MethodBadge from './MethodBadge.svelte';
  import TokenizedUrl from './TokenizedUrl.svelte';
  import VariableInput from './VariableInput.svelte';

  interface ImportNoteItem {
    readonly localized: boolean;
    readonly text: string;
  }

  let pastedText = $state('');
  let preview = $state<OpenApiPreview | null>(null);
  let customName = $state('');
  let customBaseUrl = $state('');
  let enableAfterImport = $state(true);
  let busy = $state(false);
  let importing = $state(false);
  let failure = $state<NetworkFailure | null>(null);

  const specLanguage = $derived<CodeEditorLanguage>(pastedText.trimStart().startsWith('{') ? 'json' : 'yaml');
  const globalVariableSuggestions = $derived(availableVariables(null, null, networkLibrary.globalVariables));
  const unresolvedWarnings = $derived.by((): ImportNoteItem[] => {
    if (preview === null) {
      return [];
    }
    const items = preview.warningItems ?? [];
    if (items.length > 0) {
      return activeImportWarnings(items, {
        baseUrl: customBaseUrl,
        cookies: networkLibrary.cookies,
      }).map((warning) => ({ localized: true, text: formatImportWarning(warning) }));
    }
    return preview.warnings.map((text) => ({ localized: false, text }));
  });

  function setCustomBaseUrl(next: string): void {
    customBaseUrl = next;
  }

  function applyPreview(next: OpenApiPreview): void {
    preview = next;
    customName = next.title;
    customBaseUrl = next.baseUrl;
    failure = null;
  }

  function handleOpenChange(nextOpen: boolean): void {
    if (!nextOpen && !importing) {
      networkDialogs.closeImport();
    }
  }

  async function pickFile(): Promise<void> {
    busy = true;
    failure = null;
    const outcome = await networkLibrary.pickOpenApiFile(m.network_import_file_filter());

    busy = false;
    if (!outcome.ok) {
      failure = outcome.failure;
      return;
    }
    if (outcome.data !== null) {
      applyPreview(outcome.data);
    }
  }

  async function parsePasted(): Promise<void> {
    if (pastedText.trim() === '') {
      return;
    }
    busy = true;
    failure = null;
    const outcome = await networkLibrary.previewOpenApiText(pastedText);

    busy = false;
    if (outcome.ok) {
      applyPreview(outcome.data);
    } else {
      failure = outcome.failure;
    }
  }

  async function confirmImport(): Promise<void> {
    if (preview === null) {
      return;
    }
    importing = true;
    failure = null;
    const outcome = await networkLibrary.confirmOpenApiImport({
      baseUrl: customBaseUrl.trim() || null,
      enabled: enableAfterImport,
      name: customName.trim() || null,
      previewId: preview.previewId,
    });

    importing = false;
    if (outcome.ok) {
      networkDialogs.closeImport();
      await openCollection(outcome.data.id);
    } else {
      failure = outcome.failure;
    }
  }

  $effect(() => {
    if (networkDialogs.importOpen) {
      pastedText = '';
      preview = null;
      customName = '';
      customBaseUrl = '';
      enableAfterImport = true;
      busy = false;
      importing = false;
      failure = null;
    }
  });
</script>

<Dialog.Root open={networkDialogs.importOpen} onOpenChange={handleOpenChange}>
  <Dialog.Content
    class="flex max-h-[calc(100vh-2rem)] flex-col overflow-hidden sm:max-w-2xl"
    data-network-import-dialog=""
    showCloseButton={false}
  >
    <div class="flex min-h-0 flex-1 flex-col gap-4">
      <Dialog.Header class="shrink-0">
        <Dialog.Title>{m.network_import_title()}</Dialog.Title>
        <Dialog.Description>{m.network_import_description()}</Dialog.Description>
      </Dialog.Header>

      <div class="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pe-1">
        {#if failure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Description>{networkFailureMessage(failure)}</Alert.Description>
          </Alert.Root>
        {/if}

        <div class="flex flex-col gap-3">
          <div>
            <Button
              data-network-import-choose-file=""
              disabled={busy || importing}
              type="button"
              variant="outline"
              onclick={() => void pickFile()}
            >
              <UploadIcon data-icon="inline-start" />
              {m.network_import_choose_file()}
            </Button>
          </div>

          <Field.Field>
            <Field.Label>{m.network_import_paste_label()}</Field.Label>
            <CodeEditor
              class="h-48"
              ariaLabel={m.network_import_paste_label()}
              language={specLanguage}
              placeholder={m.network_import_paste_placeholder()}
              readonly={busy || importing}
              testId="network-import-editor"
              bind:value={pastedText}
            />
          </Field.Field>

          <div>
            <Button
              data-network-import-preview=""
              disabled={busy || importing || pastedText.trim() === ''}
              size="sm"
              type="button"
              variant="secondary"
              onclick={() => void parsePasted()}
            >
              {busy ? m.network_import_previewing() : m.network_import_preview_button()}
            </Button>
          </div>
        </div>

        {#if preview !== null}
          <div class="flex flex-col gap-3 rounded-lg border border-border bg-muted/20 p-3">
            <div class="flex flex-wrap items-center gap-2">
              <Badge class="rounded-sm" variant="secondary">
                {m.network_import_version({ version: preview.specVersion })}
              </Badge>
              <Badge class="rounded-sm" variant="outline">
                {m.network_import_endpoints_count({ count: preview.endpoints.length })}
              </Badge>
            </div>

            <div class="grid gap-3 sm:grid-cols-2">
              <Field.Field>
                <Field.Label>{m.network_import_collection_name()}</Field.Label>
                <Input
                  disabled={importing}
                  placeholder={m.network_dialog_collection_name_placeholder()}
                  bind:value={customName}
                />
              </Field.Field>

              <Field.Field>
                <Field.Label for="network-import-base-url">{m.network_import_base_url()}</Field.Label>
                <VariableInput
                  id="network-import-base-url"
                  class="font-mono text-xs"
                  disabled={importing}
                  placeholder={m.network_collection_base_url_placeholder()}
                  value={customBaseUrl}
                  variables={globalVariableSuggestions}
                  onValueChange={setCustomBaseUrl}
                />
              </Field.Field>
            </div>

            <div class="flex items-center justify-between gap-2">
              <span class="text-xs font-medium">{m.network_import_enable_now()}</span>
              <Switch
                aria-label={m.network_import_enable_now()}
                disabled={importing}
                bind:checked={enableAfterImport}
              />
            </div>

            {#if preview.environments.length > 0}
              <div class="flex flex-wrap items-center gap-1.5 text-xs">
                <span class="font-medium text-muted-foreground">
                  {m.network_import_environments_label()}:
                </span>
                {#each preview.environments as envName (envName)}
                  <Badge class="rounded-sm" data-content="" variant="outline">{envName}</Badge>
                {/each}
              </div>
            {/if}

            {#if preview.folders.length > 0}
              <div class="flex flex-wrap items-center gap-1.5 text-xs">
                <span class="font-medium text-muted-foreground">
                  {m.network_import_folders_label()}:
                </span>
                {#each preview.folders as folderName (folderName)}
                  <Badge class="rounded-sm" data-content="" variant="outline">{folderName}</Badge>
                {/each}
              </div>
            {/if}

            {#if preview.securitySchemes.length > 0}
              <div class="flex flex-wrap items-center gap-1.5 text-xs">
                <span class="font-medium text-muted-foreground">
                  {m.network_import_security_label()}:
                </span>
                {#each preview.securitySchemes as scheme (scheme)}
                  <Badge class="rounded-sm" data-content="" variant="secondary">{scheme}</Badge>
                {/each}
              </div>
            {/if}

            {#if unresolvedWarnings.length > 0}
              <Alert.Root data-network-import-notes="">
                <CircleAlertIcon />
                <Alert.Title>{m.network_import_warnings_title()}</Alert.Title>
                <Alert.Description>
                  <ul class="list-disc ps-4 text-xs">
                    {#each unresolvedWarnings as warning, idx (idx)}
                      {#if warning.localized}
                        <li>{warning.text}</li>
                      {:else}
                        <li data-content="">{warning.text}</li>
                      {/if}
                    {/each}
                  </ul>
                </Alert.Description>
              </Alert.Root>
            {/if}

            {#if preview.endpoints.length > 0}
              <div
                class="max-h-48 divide-y divide-border overflow-y-auto rounded-md border border-border bg-background"
              >
                {#each preview.endpoints as ep, idx (idx)}
                  <div class="flex items-center gap-2 px-2.5 py-1.5 text-xs" title={ep.name}>
                    <MethodBadge class="px-1 py-0 text-[9px]" method={ep.method} />
                    <TokenizedUrl class="truncate text-[11px]" value={ep.path} />
                    <span class="ms-auto truncate text-muted-foreground" data-content="" dir="auto">
                      {ep.name}
                    </span>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {/if}
      </div>

      <Dialog.Footer class="shrink-0">
        <Button disabled={importing} type="button" variant="outline" onclick={() => networkDialogs.closeImport()}>
          {m.network_cancel()}
        </Button>
        <Button
          data-network-import-confirm=""
          disabled={preview === null || importing}
          type="button"
          onclick={() => void confirmImport()}
        >
          {importing ? m.network_import_importing() : m.network_import_confirm_button()}
        </Button>
      </Dialog.Footer>
    </div>
  </Dialog.Content>
</Dialog.Root>
