<script lang="ts">
  import type { CodeCompletionItem } from '@mannie/plugin-sdk/code-editor';
  import { CodeEditor } from '@mannie/plugin-sdk/components';
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import * as Select from '@mannie/plugin-sdk/components/select';
  import { Switch } from '@mannie/plugin-sdk/components/switch';
  import * as Tabs from '@mannie/plugin-sdk/components/tabs';
  import { Textarea } from '@mannie/plugin-sdk/components/textarea';
  import BracesIcon from '@mannie/plugin-sdk/icons/braces';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import PlayIcon from '@mannie/plugin-sdk/icons/play';
  import SaveIcon from '@mannie/plugin-sdk/icons/save';
  import SquareIcon from '@mannie/plugin-sdk/icons/square';
  import { onDestroy, untrack } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';

  import type {
    AuthConfig,
    CollectionView,
    HttpMethod,
    KeyValueEntry,
    RequestInput,
    RequestView,
  } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { networkLibrary } from '../state/network.svelte.js';
  import { type BodyKind, HTTP_METHODS, type NetworkFailure, type RequestDraft } from '../types/network.js';
  import { draftFromRequest } from '../utils/network-drafts.js';
  import { bodyKindLabel, networkFailureMessage } from '../utils/network-messages.js';
  import {
    availableVariables,
    composeUrlWithQuery,
    inferCodeLanguage,
    stripQueryFromUrl,
    syncPathParamsFromUrl,
    syncQueryParamsFromUrl,
    toCodeCompletions,
  } from '../utils/network-urls.js';

  import AuthEditor from './AuthEditor.svelte';
  import KeyValueEditor from './KeyValueEditor.svelte';
  import ResponsePane from './ResponsePane.svelte';
  import TokenizedUrl from './TokenizedUrl.svelte';

  interface Props {
    collection: CollectionView;
    request: RequestView;
  }

  const NO_ENVIRONMENT = '__none__';
  const ROOT_FOLDER = '__root__';
  const SPLIT_STORAGE_KEY = 'mannie.network.workbench-split-v2';
  const DEFAULT_SPLIT_PERCENT = 52;
  const MIN_SPLIT_PERCENT = 20;
  const MAX_SPLIT_PERCENT = 80;
  const BODY_KINDS: readonly BodyKind[] = ['none', 'json', 'text', 'form_url_encoded'];

  let { collection, request }: Props = $props();

  const rememberedPathParams = new SvelteMap<string, KeyValueEntry>();

  let loadedRequestId = untrack(() => request.id);
  let draggingSplit = false;
  let pendingSplitPercent: number | null = null;
  let splitFrame: number | null = null;

  let draft = $state<RequestDraft>(untrack(() => buildHydratedDraft(request)));
  let dirty = $state(false);
  let saving = $state(false);
  let saveFailure = $state<NetworkFailure | null>(null);
  let environmentFailure = $state<NetworkFailure | null>(null);
  let switchingEnvironment = $state(false);
  let selectedEnvironmentId = $state<string | null>(untrack(() => collection.activeEnvironmentId));
  let jsonError = $state(false);
  let requestTab = $state('query');

  // Lightweight pointer-capture vertical split state (Note 155): avoids
  // PaneForge's unthrottled mousemove DOM queries and inline `<style>` tags.
  let splitContainer = $state<HTMLDivElement | null>(null);
  let splitPercent = $state<number>(loadInitialSplit());

  const targetKey = $derived(`${collection.id}:${request.id}`);
  const isRunning = $derived(networkLibrary.isRunning(targetKey));
  const response = $derived(networkLibrary.responses[targetKey] ?? null);
  const sendFailure = $derived(networkLibrary.sendFailures[targetKey] ?? null);
  const activeEnvLabel = $derived(
    collection.environments.find((env) => env.id === selectedEnvironmentId)?.name ??
      m.network_collection_no_environment(),
  );
  const folderLabel = $derived(
    collection.folders.find((folder) => folder.id === draft.folderId)?.name ?? m.network_settings_folder_root(),
  );
  const variableSuggestions = $derived(
    availableVariables(collection, selectedEnvironmentId, networkLibrary.globalVariables),
  );
  const textBodyLanguage = $derived(
    draft.body.kind === 'text' ? inferCodeLanguage(draft.body.contentType, draft.body.content) : ('plain' as const),
  );

  function buildHydratedDraft(source: RequestView): RequestDraft {
    const base = draftFromRequest(source);
    const urlWithQuery = composeUrlWithQuery(base.urlTemplate, base.queryParams);
    const syncedPathParams = syncPathParamsFromUrl(urlWithQuery, base.pathParams, rememberedPathParams);
    return {
      ...base,
      pathParams: syncedPathParams,
      urlTemplate: urlWithQuery,
    };
  }

  function loadInitialSplit(): number {
    if (typeof localStorage === 'undefined') {
      return DEFAULT_SPLIT_PERCENT;
    }
    try {
      const raw = localStorage.getItem(SPLIT_STORAGE_KEY);
      if (raw === null) {
        return DEFAULT_SPLIT_PERCENT;
      }
      const parsed = Number.parseFloat(raw);
      if (!Number.isFinite(parsed)) {
        return DEFAULT_SPLIT_PERCENT;
      }
      return Math.min(MAX_SPLIT_PERCENT, Math.max(MIN_SPLIT_PERCENT, parsed));
    } catch {
      return DEFAULT_SPLIT_PERCENT;
    }
  }

  function persistSplit(percent: number): void {
    if (typeof localStorage === 'undefined') {
      return;
    }
    try {
      localStorage.setItem(SPLIT_STORAGE_KEY, percent.toFixed(1));
    } catch {
      // Ignore storage quota or sandbox restrictions.
    }
  }

  function getBodyCompletions(): readonly CodeCompletionItem[] {
    return toCodeCompletions(variableSuggestions, {
      collection: m.network_variable_scope_collection(),
      environment: m.network_variable_scope_environment(),
      global: m.network_variable_scope_global(),
      redacted: m.network_variable_redacted_preview(),
    });
  }

  function clearResizingCursor(): void {
    if (typeof document !== 'undefined') {
      delete document.documentElement.dataset.paneResizing;
    }
  }

  function clampSplit(percent: number): number {
    return Math.min(MAX_SPLIT_PERCENT, Math.max(MIN_SPLIT_PERCENT, Math.round(percent * 10) / 10));
  }

  function flushPendingSplit(): void {
    splitFrame = null;
    if (pendingSplitPercent !== null) {
      splitPercent = pendingSplitPercent;
      pendingSplitPercent = null;
    }
  }

  function handleSplitterPointerDown(event: PointerEvent): void {
    const isPrimaryButton = event.button === 0;
    const hasContainer = splitContainer !== null;

    if (!isPrimaryButton || !hasContainer) {
      return;
    }
    event.preventDefault();
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    draggingSplit = true;
    document.documentElement.dataset.paneResizing = 'vertical';
  }

  function handleSplitterPointerMove(event: PointerEvent): void {
    if (!draggingSplit) {
      return;
    }
    if (splitContainer === null) {
      return;
    }
    const rect = splitContainer.getBoundingClientRect();
    if (rect.height <= 0) {
      return;
    }
    pendingSplitPercent = clampSplit(((event.clientY - rect.top) / rect.height) * 100);
    splitFrame ??= requestAnimationFrame(flushPendingSplit);
  }

  function finishSplitterDrag(event: PointerEvent): void {
    if (!draggingSplit) {
      return;
    }
    draggingSplit = false;
    const handle = event.currentTarget as HTMLElement;
    if (handle.hasPointerCapture(event.pointerId)) {
      handle.releasePointerCapture(event.pointerId);
    }
    if (splitFrame !== null) {
      cancelAnimationFrame(splitFrame);
      flushPendingSplit();
    }
    clearResizingCursor();
    persistSplit(splitPercent);
  }

  function handleSplitterKeydown(event: KeyboardEvent): void {
    let next: number | null = null;
    if (event.key === 'ArrowUp') {
      next = clampSplit(splitPercent - 4);
    } else if (event.key === 'ArrowDown') {
      next = clampSplit(splitPercent + 4);
    } else if (event.key === 'Home') {
      next = MIN_SPLIT_PERCENT;
    } else if (event.key === 'End') {
      next = MAX_SPLIT_PERCENT;
    }
    if (next !== null) {
      event.preventDefault();
      splitPercent = next;
      persistSplit(next);
    }
  }

  function resetSplitter(): void {
    splitPercent = DEFAULT_SPLIT_PERCENT;
    persistSplit(DEFAULT_SPLIT_PERCENT);
  }

  function existingTextBody(fallback: string): string {
    if (draft.body.kind === 'json') {
      return draft.body.content;
    }
    if (draft.body.kind === 'text') {
      return draft.body.content;
    }
    return fallback;
  }

  function toRequestInput(): RequestInput {
    // Strip the synchronized query string from `urlTemplate` before persisting
    // or sending so query parameters live solely in `queryParams` (preventing
    // duplicates on Send and keeping secret query params out of SQLite).
    return {
      auth: draft.auth,
      body: draft.body,
      description: draft.description,
      folderId: draft.folderId,
      headers: draft.headers,
      method: draft.method,
      name: draft.name,
      pathParams: draft.pathParams,
      protocol: 'http',
      queryParams: draft.queryParams,
      timeoutSeconds: Number.isFinite(draft.timeoutSeconds)
        ? Math.min(300, Math.max(1, Math.round(draft.timeoutSeconds)))
        : 30,
      urlTemplate: stripQueryFromUrl(draft.urlTemplate),
      useCookies: draft.useCookies,
    };
  }

  function markDirty(): void {
    dirty = true;
    saveFailure = null;
  }

  function updateName(event: Event): void {
    draft.name = (event.currentTarget as HTMLInputElement).value;
    markDirty();
  }

  function updateUrlTemplate(nextUrl: string): void {
    draft.urlTemplate = nextUrl;
    draft.pathParams = syncPathParamsFromUrl(nextUrl, draft.pathParams, rememberedPathParams);
    draft.queryParams = syncQueryParamsFromUrl(nextUrl, draft.queryParams);
    markDirty();
  }

  function updatePathParams(next: KeyValueEntry[]): void {
    draft.pathParams = next;
    for (const entry of next) {
      const key = entry.key.trim();
      if (key !== '') {
        rememberedPathParams.set(key, { ...entry, key });
      }
    }
    markDirty();
  }

  function updateQueryParams(next: KeyValueEntry[]): void {
    draft.queryParams = next;
    draft.urlTemplate = composeUrlWithQuery(draft.urlTemplate, next);
    markDirty();
  }

  function updateHeaders(next: KeyValueEntry[]): void {
    draft.headers = next;
    markDirty();
  }

  function updateAuth(next: AuthConfig): void {
    draft.auth = next;
    markDirty();
  }

  function updateFormEntries(next: KeyValueEntry[]): void {
    draft.body = { entries: next, kind: 'form_url_encoded' };
    markDirty();
  }

  function updateDescription(event: Event): void {
    draft.description = (event.currentTarget as HTMLTextAreaElement).value;
    markDirty();
  }

  function updateFolder(value: string): void {
    draft.folderId = value === ROOT_FOLDER ? null : value;
    markDirty();
  }

  function updateUseCookies(checked: boolean): void {
    draft.useCookies = checked;
    markDirty();
  }

  function changeMethod(raw: string): void {
    if (HTTP_METHODS.includes(raw as HttpMethod)) {
      draft.method = raw as HttpMethod;
      markDirty();
    }
  }

  function changeBodyKind(raw: string): void {
    jsonError = false;
    switch (raw) {
      case 'none': {
        draft.body = { kind: 'none' };
        break;
      }
      case 'json': {
        draft.body = {
          content: existingTextBody('{}'),
          kind: 'json',
        };
        break;
      }
      case 'text': {
        draft.body = {
          content: existingTextBody(''),
          contentType: draft.body.kind === 'text' ? draft.body.contentType : 'text/plain',
          kind: 'text',
        };
        break;
      }
      case 'form_url_encoded': {
        draft.body = {
          entries: draft.body.kind === 'form_url_encoded' ? draft.body.entries : [],
          kind: 'form_url_encoded',
        };
        break;
      }
    }
    markDirty();
  }

  function formatJsonBody(): void {
    if (draft.body.kind !== 'json') {
      return;
    }
    try {
      const parsed: unknown = JSON.parse(draft.body.content);

      draft.body = { content: JSON.stringify(parsed, null, 2), kind: 'json' };
      jsonError = false;
      markDirty();
    } catch {
      jsonError = true;
    }
  }

  function updateJsonContent(content: string): void {
    draft.body = { content, kind: 'json' };
    jsonError = false;
    markDirty();
  }

  function updateTextContent(content: string): void {
    if (draft.body.kind !== 'text') {
      return;
    }
    draft.body = { content, contentType: draft.body.contentType, kind: 'text' };
    markDirty();
  }

  function updateTextContentType(event: Event): void {
    if (draft.body.kind !== 'text') {
      return;
    }
    const contentType = (event.currentTarget as HTMLInputElement).value;

    draft.body = { content: draft.body.content, contentType, kind: 'text' };
    markDirty();
  }

  function updateTimeout(event: Event): void {
    const parsed = Number.parseInt((event.currentTarget as HTMLInputElement).value, 10);

    draft.timeoutSeconds = Number.isFinite(parsed) ? parsed : 30;
    markDirty();
  }

  async function selectEnvironment(raw: string): Promise<void> {
    const nextEnvId = raw === NO_ENVIRONMENT ? null : raw;
    const previousEnvId = selectedEnvironmentId;

    selectedEnvironmentId = nextEnvId;
    switchingEnvironment = true;
    environmentFailure = null;

    const outcome = await networkLibrary.setActiveEnvironment(collection.id, nextEnvId);
    switchingEnvironment = false;

    if (!outcome.ok) {
      selectedEnvironmentId = previousEnvId;
      environmentFailure = outcome.failure;
    }
  }

  async function saveRequest(): Promise<void> {
    saving = true;
    saveFailure = null;
    const outcome = await networkLibrary.updateRequest(request.id, toRequestInput());

    saving = false;
    if (outcome.ok) {
      draft = buildHydratedDraft(outcome.data);
      dirty = false;
    } else {
      saveFailure = outcome.failure;
    }
  }

  async function sendNow(): Promise<void> {
    if (switchingEnvironment) {
      return;
    }
    await networkLibrary.sendOwnerRequest(targetKey, {
      collectionId: collection.id,
      draft: toRequestInput(),
      environmentId: selectedEnvironmentId,
      requestId: request.id,
    });
  }

  function cancelActiveRequest(): void {
    void networkLibrary.cancelRequest(targetKey);
  }

  $effect(() => {
    if (request.id !== loadedRequestId) {
      loadedRequestId = request.id;
      rememberedPathParams.clear();
      draft = buildHydratedDraft(request);
      dirty = false;
      saving = false;
      saveFailure = null;
      environmentFailure = null;
      jsonError = false;
    }
  });

  $effect(() => {
    if (!switchingEnvironment) {
      selectedEnvironmentId = collection.activeEnvironmentId;
    }
  });

  onDestroy(() => {
    if (splitFrame !== null) {
      cancelAnimationFrame(splitFrame);
      splitFrame = null;
    }
    if (draggingSplit) {
      draggingSplit = false;
      clearResizingCursor();
    }
  });
</script>

<div class="flex h-full min-h-0 flex-col" data-network-workbench={request.id}>
  <div class="flex flex-col gap-3 border-b border-border bg-card/50 px-4 py-3">
    <div class="flex flex-wrap items-center justify-between gap-2">
      <div class="flex min-w-0 flex-1 items-center gap-2">
        <Badge class="shrink-0 rounded-sm font-mono text-[10px] uppercase" variant="secondary">
          {m.network_protocol_http()}
        </Badge>
        <Input
          id="network-request-name"
          class="h-8 min-w-0 flex-1 font-medium"
          aria-label={m.network_request_name_label()}
          data-network-request-name=""
          placeholder={m.network_request_name_placeholder()}
          value={draft.name}
          oninput={updateName}
        />
        {#if dirty}
          <Badge class="shrink-0 rounded-sm" variant="outline">{m.network_request_unsaved()}</Badge>
        {/if}
      </div>

      <div class="flex shrink-0 flex-wrap items-center gap-2">
        <Select.Root
          disabled={switchingEnvironment}
          type="single"
          value={selectedEnvironmentId ?? NO_ENVIRONMENT}
          onValueChange={(value: string) => void selectEnvironment(value)}
        >
          <Select.Trigger
            class="h-8 w-44 text-xs"
            aria-label={`${m.network_collection_active_environment()}: ${activeEnvLabel}`}
          >
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

        <Button
          data-network-save=""
          disabled={!dirty || saving}
          size="sm"
          type="button"
          variant="outline"
          onclick={() => void saveRequest()}
        >
          <SaveIcon class="size-3.5" data-icon="inline-start" />
          {saving ? m.network_saving() : m.network_save()}
        </Button>
      </div>
    </div>

    <div class="flex items-center gap-2">
      <Select.Root type="single" value={draft.method} onValueChange={changeMethod}>
        <Select.Trigger
          class="h-9 w-28 shrink-0 font-mono text-xs font-semibold"
          aria-label={`${m.network_request_method_label()}: ${draft.method}`}
        >
          {draft.method}
        </Select.Trigger>
        <Select.Content>
          {#each HTTP_METHODS as method (method)}
            <Select.Item label={method} value={method}>
              <span class="font-mono text-xs font-semibold">{method}</span>
            </Select.Item>
          {/each}
        </Select.Content>
      </Select.Root>

      <CodeEditor
        id="network-request-url"
        class="min-w-0 flex-1"
        ariaLabel={m.network_request_url_label()}
        completions={getBodyCompletions}
        highlightTokens="url"
        language="plain"
        lineNumbers={false}
        placeholder={m.network_request_url_placeholder()}
        singleLine
        testId="network-request-url"
        value={draft.urlTemplate}
        wrapLines={false}
        onModEnter={() => void sendNow()}
        onchange={updateUrlTemplate}
      />

      {#if isRunning}
        <Button data-network-cancel="" size="sm" type="button" variant="destructive" onclick={cancelActiveRequest}>
          <SquareIcon class="size-3.5" data-icon="inline-start" />
          {m.network_cancel()}
        </Button>
      {:else}
        <Button
          data-network-send=""
          disabled={switchingEnvironment}
          size="sm"
          type="button"
          onclick={() => void sendNow()}
        >
          <PlayIcon class="size-3.5" data-icon="inline-start" />
          {m.network_request_send()}
        </Button>
      {/if}
    </div>

    <div class="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
      {#if collection.baseUrl.trim() !== ''}
        <p
          class="flex min-w-0 items-center gap-1.5 truncate"
          title={m.network_request_resolved_base({ baseUrl: collection.baseUrl })}
        >
          <span class="shrink-0">{m.network_collection_base_url()}:</span>
          <TokenizedUrl class="truncate" value={collection.baseUrl} />
        </p>
      {/if}

      <div class="ms-auto flex flex-wrap items-center gap-3" data-network-token-legend="">
        <span class="inline-flex items-center gap-1">
          <span
            class="rounded-[3px] bg-token-variable px-1 font-mono text-[10px] font-medium text-token-variable-foreground ring-1 ring-token-variable-border/55 ring-inset"
            aria-hidden="true"
            data-content=""
          >
            {'{{}}'}
          </span>
          <span>{m.network_token_variable_label()}</span>
        </span>

        <span class="inline-flex items-center gap-1">
          <span
            class="rounded-[3px] bg-token-path px-1 font-mono text-[10px] font-medium text-token-path-foreground ring-1 ring-token-path-border/55 ring-inset"
            aria-hidden="true"
            data-content=""
          >
            {'{}'}
          </span>
          <span>{m.network_token_path_label()}</span>
        </span>

        <span class="inline-flex items-center gap-1">
          <span
            class="rounded-[3px] bg-token-query px-1 font-mono text-[10px] font-medium text-token-query-foreground ring-1 ring-token-query-border/55 ring-inset"
            aria-hidden="true"
            data-content=""
          >
            ?
          </span>
          <span>{m.network_token_query_label()}</span>
        </span>
      </div>
    </div>

    {#if saveFailure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.network_request_save_failed()}</Alert.Title>
        <Alert.Description>{networkFailureMessage(saveFailure)}</Alert.Description>
      </Alert.Root>
    {/if}

    {#if environmentFailure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.network_environment_switch_failed()}</Alert.Title>
        <Alert.Description>{networkFailureMessage(environmentFailure)}</Alert.Description>
      </Alert.Root>
    {/if}
  </div>

  <div bind:this={splitContainer} class="flex min-h-0 flex-1 flex-col overflow-hidden">
    <div class="flex min-h-0 shrink-0 grow-0 flex-col overflow-hidden p-4" style:flex-basis={`${splitPercent}%`}>
      <Tabs.Root class="flex min-h-0 flex-1 flex-col" bind:value={requestTab}>
        <Tabs.List class="w-fit">
          <Tabs.Trigger data-network-tab="query" value="query">
            <span class="inline-flex items-baseline gap-1">
              <span class="font-mono text-[11px] font-semibold text-token-query-foreground" aria-hidden="true">?</span>
              <span>{m.network_tab_query()}</span>
              {#if draft.queryParams.length > 0}
                <span class="text-[11px] text-token-query-foreground" data-content="">
                  ({draft.queryParams.length})
                </span>
              {/if}
            </span>
          </Tabs.Trigger>

          <Tabs.Trigger data-network-tab="path" value="path">
            <span class="inline-flex items-baseline gap-1">
              <span class="font-mono text-[11px] font-semibold text-token-path-foreground" aria-hidden="true">
                {'{}'}
              </span>
              <span>{m.network_tab_path()}</span>
              {#if draft.pathParams.length > 0}
                <span class="text-[11px] text-token-path-foreground" data-content="">
                  ({draft.pathParams.length})
                </span>
              {/if}
            </span>
          </Tabs.Trigger>

          <Tabs.Trigger data-network-tab="headers" value="headers">
            <span class="inline-flex items-baseline gap-1">
              <span>{m.network_tab_headers()}</span>
              {#if draft.headers.length > 0}
                <span class="text-[11px] text-muted-foreground" data-content="">({draft.headers.length})</span>
              {/if}
            </span>
          </Tabs.Trigger>

          <Tabs.Trigger data-network-tab="auth" value="auth">{m.network_tab_auth()}</Tabs.Trigger>
          <Tabs.Trigger data-network-tab="body" value="body">{m.network_tab_body()}</Tabs.Trigger>
          <Tabs.Trigger data-network-tab="settings" value="settings">{m.network_tab_settings()}</Tabs.Trigger>
        </Tabs.List>

        <Tabs.Content class="mt-3 min-h-0 flex-1 overflow-auto" value="query">
          <KeyValueEditor
            entries={draft.queryParams}
            keyPlaceholder={m.network_kv_query_key_placeholder()}
            kind="query"
            valuePlaceholder={m.network_kv_query_value_placeholder()}
            variables={variableSuggestions}
            onChange={updateQueryParams}
          />
        </Tabs.Content>

        <Tabs.Content class="mt-3 min-h-0 flex-1 overflow-auto" value="path">
          <KeyValueEditor
            allowAddRemove={false}
            emptyMessage={m.network_params_path_empty()}
            entries={draft.pathParams}
            kind="path"
            lockKeys
            showEnabled={false}
            valuePlaceholder={m.network_kv_path_value_placeholder()}
            variables={variableSuggestions}
            onChange={updatePathParams}
          />
        </Tabs.Content>

        <Tabs.Content class="mt-3 min-h-0 flex-1 overflow-auto" value="headers">
          <KeyValueEditor
            entries={draft.headers}
            keyPlaceholder={m.network_kv_header_key_placeholder()}
            valuePlaceholder={m.network_kv_header_value_placeholder()}
            variables={variableSuggestions}
            onChange={updateHeaders}
          />
        </Tabs.Content>

        <Tabs.Content class="mt-3 min-h-0 flex-1 overflow-auto" value="auth">
          <AuthEditor allowInherit auth={draft.auth} variables={variableSuggestions} onChange={updateAuth} />
        </Tabs.Content>

        <Tabs.Content class="mt-3 flex min-h-0 flex-1 flex-col overflow-auto" value="body">
          <div class="flex min-h-0 flex-1 flex-col gap-3">
            <div class="flex flex-wrap items-center justify-between gap-2">
              <Select.Root type="single" value={draft.body.kind} onValueChange={changeBodyKind}>
                <Select.Trigger
                  class="h-8 w-52 text-xs"
                  aria-label={`${m.network_body_type_label()}: ${bodyKindLabel(draft.body.kind)}`}
                >
                  {bodyKindLabel(draft.body.kind)}
                </Select.Trigger>
                <Select.Content>
                  {#each BODY_KINDS as kind (kind)}
                    <Select.Item label={bodyKindLabel(kind)} value={kind}>
                      {bodyKindLabel(kind)}
                    </Select.Item>
                  {/each}
                </Select.Content>
              </Select.Root>

              {#if draft.body.kind === 'json'}
                <Button size="sm" type="button" variant="outline" onclick={formatJsonBody}>
                  <BracesIcon class="size-3.5" data-icon="inline-start" />
                  {m.network_body_format_json()}
                </Button>
              {/if}
            </div>

            {#if draft.body.kind === 'json'}
              <CodeEditor
                class="min-h-40 flex-1"
                ariaLabel={m.network_body_content_label()}
                completions={getBodyCompletions}
                highlightTokens="variables"
                language="json"
                placeholder={m.network_body_json_placeholder()}
                testId="network-body-editor"
                value={draft.body.content}
                onModEnter={() => void sendNow()}
                onchange={updateJsonContent}
              />
              {#if jsonError}
                <p class="text-xs text-destructive">{m.network_body_invalid_json()}</p>
              {/if}
            {:else if draft.body.kind === 'text'}
              <Field.Field>
                <Field.Label>{m.network_body_content_type()}</Field.Label>
                <Input
                  class="max-w-xs font-mono text-xs"
                  placeholder={m.network_body_content_type_placeholder()}
                  value={draft.body.contentType}
                  oninput={updateTextContentType}
                />
              </Field.Field>
              <CodeEditor
                class="min-h-36 flex-1"
                ariaLabel={m.network_body_content_label()}
                completions={getBodyCompletions}
                highlightTokens="variables"
                language={textBodyLanguage}
                placeholder={m.network_body_text_placeholder()}
                testId="network-body-editor"
                value={draft.body.content}
                onModEnter={() => void sendNow()}
                onchange={updateTextContent}
              />
            {:else if draft.body.kind === 'form_url_encoded'}
              <KeyValueEditor
                entries={draft.body.entries}
                keyPlaceholder={m.network_kv_form_key_placeholder()}
                valuePlaceholder={m.network_kv_form_value_placeholder()}
                variables={variableSuggestions}
                onChange={updateFormEntries}
              />
            {/if}
          </div>
        </Tabs.Content>

        <Tabs.Content class="mt-3 min-h-0 flex-1 overflow-auto" value="settings">
          <div class="flex max-w-xl flex-col gap-4">
            <Field.Field>
              <Field.Label>{m.network_settings_description_label()}</Field.Label>
              <Textarea
                class="min-h-20 text-xs"
                placeholder={m.network_settings_description_placeholder()}
                value={draft.description}
                oninput={updateDescription}
              />
            </Field.Field>

            <div class="grid gap-4 sm:grid-cols-2">
              <Field.Field>
                <Field.Label>{m.network_settings_folder_label()}</Field.Label>
                <Select.Root type="single" value={draft.folderId ?? ROOT_FOLDER} onValueChange={updateFolder}>
                  <Select.Trigger class="w-full text-xs">{folderLabel}</Select.Trigger>
                  <Select.Content>
                    <Select.Item label={m.network_settings_folder_root()} value={ROOT_FOLDER}>
                      {m.network_settings_folder_root()}
                    </Select.Item>
                    {#each collection.folders as folder (folder.id)}
                      <Select.Item label={folder.name} value={folder.id}>
                        <span data-content="" dir="auto">{folder.name}</span>
                      </Select.Item>
                    {/each}
                  </Select.Content>
                </Select.Root>
              </Field.Field>

              <Field.Field>
                <Field.Label>{m.network_settings_timeout_label()}</Field.Label>
                <Input
                  max={300}
                  min={1}
                  placeholder={m.network_settings_timeout_placeholder()}
                  type="number"
                  value={String(draft.timeoutSeconds)}
                  oninput={updateTimeout}
                />
              </Field.Field>
            </div>

            <div class="flex items-center justify-between gap-3 rounded-md border border-border p-3">
              <div class="flex flex-col">
                <span class="text-xs font-medium">{m.network_settings_use_cookies_label()}</span>
                <span class="text-xs text-muted-foreground">
                  {m.network_settings_use_cookies_description()}
                </span>
              </div>
              <Switch
                aria-label={m.network_settings_use_cookies_label()}
                checked={draft.useCookies}
                onCheckedChange={updateUseCookies}
              />
            </div>
          </div>
        </Tabs.Content>
      </Tabs.Root>
    </div>

    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="group/split relative flex h-1.5 shrink-0 cursor-ns-resize touch-none items-center justify-center bg-border/60 transition-colors hover:bg-primary/40 focus-visible:bg-primary/50 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      aria-label={m.network_split_handle_label()}
      aria-orientation="horizontal"
      aria-valuemax={MAX_SPLIT_PERCENT}
      aria-valuemin={MIN_SPLIT_PERCENT}
      aria-valuenow={Math.round(splitPercent)}
      data-network-splitter=""
      role="separator"
      tabindex="0"
      ondblclick={resetSplitter}
      onkeydown={handleSplitterKeydown}
      onpointercancel={finishSplitterDrag}
      onpointerdown={handleSplitterPointerDown}
      onpointermove={handleSplitterPointerMove}
      onpointerup={finishSplitterDrag}
    >
      <div class="h-0.5 w-8 rounded-full bg-muted-foreground/40 group-hover/split:bg-primary"></div>
    </div>

    <div class="flex min-h-0 flex-1 flex-col overflow-hidden">
      <ResponsePane failure={sendFailure} {response} running={isRunning} onCancel={cancelActiveRequest} />
    </div>
  </div>
</div>
