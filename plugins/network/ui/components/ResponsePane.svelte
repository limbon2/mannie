<script lang="ts">
  import { CodeEditor } from '@mannie/plugin-sdk/components';
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import * as Tabs from '@mannie/plugin-sdk/components/tabs';
  import { formatBytes, formatDuration, formatTime } from '@mannie/plugin-sdk/format';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import SquareIcon from '@mannie/plugin-sdk/icons/square';

  import type { ManualResponseView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import type { NetworkFailure } from '../types/network.js';
  import { networkFailureMessage } from '../utils/network-messages.js';
  import { inferCodeLanguage } from '../utils/network-urls.js';

  interface Props {
    failure: NetworkFailure | null;
    onCancel: () => void;
    response: ManualResponseView | null;
    running: boolean;
  }

  let { failure, onCancel, response, running }: Props = $props();

  let activeTab = $state('body');
  let prettyMode = $state(true);

  const idleResponse = $derived(running ? null : response);
  const canToggleFormat = $derived.by(() => {
    if (idleResponse === null) {
      return false;
    }
    const isBodyTab = activeTab === 'body';
    const isTextBody = !idleResponse.isBinary;

    return isBodyTab && isTextBody;
  });

  const formattedBody = $derived.by(() => {
    if (response === null) {
      return '';
    }
    if (response.body === '') {
      return '';
    }
    if (!prettyMode || response.isBinary) {
      return response.body;
    }
    try {
      const parsed: unknown = JSON.parse(response.body);
      return JSON.stringify(parsed, null, 2);
    } catch {
      return response.body;
    }
  });

  const bodyLanguage = $derived.by(() => {
    if (response === null) {
      return 'plain' as const;
    }
    if (response.isBinary) {
      return 'plain' as const;
    }
    return inferCodeLanguage(response.contentType, response.body);
  });

  const statusTone = $derived.by(() => {
    if (response === null) {
      return '';
    }
    const isSuccess = response.status >= 200 && response.status < 300;
    const isRedirect = response.status >= 300 && response.status < 400;

    if (isSuccess) {
      return 'border-emerald-600/35 bg-emerald-500/10 text-emerald-900 dark:text-emerald-300';
    }
    if (isRedirect) {
      return 'border-sky-600/35 bg-sky-500/10 text-sky-900 dark:text-sky-300';
    }
    return 'border-destructive/30 bg-destructive/10 text-destructive';
  });
</script>

<div class="flex h-full min-h-0 flex-col bg-background">
  <div class="flex flex-wrap items-center justify-between gap-2 border-b border-border px-4 py-2">
    <div class="flex flex-wrap items-center gap-2">
      <h2 class="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
        {m.network_response_heading()}
      </h2>

      {#if idleResponse !== null}
        <Badge class="rounded-sm font-mono text-xs {statusTone}" data-content="" variant="outline">
          {idleResponse.statusText ? `${idleResponse.status} ${idleResponse.statusText}` : String(idleResponse.status)}
        </Badge>
        <span class="text-xs text-muted-foreground" data-content="">
          {formatDuration(idleResponse.durationMs)}
        </span>
        <span class="text-xs text-muted-foreground" data-content="">
          {formatBytes(idleResponse.bytes)}
        </span>
        {#if idleResponse.truncated}
          <Badge class="rounded-sm" variant="secondary">{m.network_response_truncated()}</Badge>
        {/if}
      {/if}
    </div>

    {#if canToggleFormat}
      <div class="flex items-center gap-1">
        <Button
          size="xs"
          type="button"
          variant={prettyMode ? 'secondary' : 'ghost'}
          onclick={() => (prettyMode = true)}
        >
          {m.network_response_pretty()}
        </Button>
        <Button
          size="xs"
          type="button"
          variant={!prettyMode ? 'secondary' : 'ghost'}
          onclick={() => (prettyMode = false)}
        >
          {m.network_response_raw()}
        </Button>
      </div>
    {/if}
  </div>

  <div class="flex min-h-0 flex-1 flex-col overflow-hidden p-4">
    {#if running}
      <div class="flex flex-1 flex-col items-center justify-center gap-3 text-center">
        <Spinner class="size-5" aria-label={m.network_request_sending()} />
        <p class="text-sm text-muted-foreground">{m.network_request_sending()}</p>
        <Button size="sm" type="button" variant="outline" onclick={onCancel}>
          <SquareIcon class="size-3.5" data-icon="inline-start" />
          {m.network_cancel()}
        </Button>
      </div>
    {:else if failure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.network_request_send_failed()}</Alert.Title>
        <Alert.Description>{networkFailureMessage(failure)}</Alert.Description>
      </Alert.Root>
    {:else if response === null}
      <div class="flex flex-1 flex-col items-center justify-center gap-1 text-center">
        <p class="text-sm font-medium">{m.network_response_empty_title()}</p>
        <p class="max-w-md text-xs text-muted-foreground">
          {m.network_response_empty_description()}
        </p>
      </div>
    {:else}
      <Tabs.Root class="flex min-h-0 flex-1 flex-col" bind:value={activeTab}>
        <Tabs.List class="w-fit">
          <Tabs.Trigger value="body">{m.network_response_tab_body()}</Tabs.Trigger>
          <Tabs.Trigger value="headers">
            <span class="inline-flex items-baseline gap-1">
              <span>{m.network_response_tab_headers()}</span>
              <span class="text-[11px] text-muted-foreground" data-content="">({response.headers.length})</span>
            </span>
          </Tabs.Trigger>

          <Tabs.Trigger value="cookies">
            <span class="inline-flex items-baseline gap-1">
              <span>{m.network_response_tab_cookies()}</span>
              <span class="text-[11px] text-muted-foreground" data-content="">({response.cookies.length})</span>
            </span>
          </Tabs.Trigger>
        </Tabs.List>

        <Tabs.Content class="mt-3 flex min-h-0 flex-1 flex-col overflow-hidden" value="body">
          {#if formattedBody === ''}
            <p class="text-xs text-muted-foreground">{m.network_response_empty_body()}</p>
          {:else}
            <CodeEditor
              class="min-h-0 flex-1"
              ariaLabel={m.network_response_body_label()}
              language={bodyLanguage}
              readonly
              testId="network-response-body"
              value={formattedBody}
            />
          {/if}
        </Tabs.Content>

        <Tabs.Content class="mt-3 min-h-0 flex-1 overflow-auto" value="headers">
          {#if response.headers.length === 0}
            <p class="text-xs text-muted-foreground">{m.network_response_no_headers()}</p>
          {:else}
            <div class="overflow-x-auto rounded-md border border-border">
              <table class="w-full border-collapse text-xs">
                <thead>
                  <tr class="border-b border-border bg-muted/40 text-muted-foreground">
                    <th class="p-2 text-start font-medium">{m.network_kv_key()}</th>
                    <th class="p-2 text-start font-medium">{m.network_kv_value()}</th>
                  </tr>
                </thead>

                <tbody>
                  {#each response.headers as header, idx (idx)}
                    <tr class="border-b border-border last:border-b-0">
                      <td class="p-2 font-mono font-medium" data-content="" dir="ltr">
                        {header.name}
                      </td>
                      <td class="p-2 font-mono break-all" data-content="" dir="ltr">
                        {header.value}
                      </td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {/if}
        </Tabs.Content>

        <Tabs.Content class="mt-3 min-h-0 flex-1 overflow-auto" value="cookies">
          {#if response.cookies.length === 0}
            <p class="text-xs text-muted-foreground">{m.network_response_no_cookies()}</p>
          {:else}
            <div class="overflow-x-auto rounded-md border border-border">
              <table class="w-full border-collapse text-xs">
                <thead>
                  <tr class="border-b border-border bg-muted/40 text-muted-foreground">
                    <th class="p-2 text-start font-medium">{m.network_cookies_name_label()}</th>
                    <th class="p-2 text-start font-medium">{m.network_cookies_value_label()}</th>
                    <th class="p-2 text-start font-medium">{m.network_cookies_domain_label()}</th>
                    <th class="p-2 text-start font-medium">{m.network_cookies_path_label()}</th>
                    <th class="p-2 text-start font-medium">{m.network_cookies_expires_label()}</th>
                  </tr>
                </thead>

                <tbody>
                  {#each response.cookies as cookie, idx (idx)}
                    <tr class="border-b border-border last:border-b-0">
                      <td class="p-2 font-mono font-medium" data-content="" dir="ltr">
                        {cookie.name}
                      </td>
                      <td class="p-2 font-mono break-all" data-content="" dir="ltr">
                        {cookie.value}
                      </td>
                      <td class="p-2 font-mono" data-content="" dir="ltr">
                        {cookie.domain}
                        {#if cookie.hostOnly}
                          <Badge class="ms-1 h-4 rounded-sm px-1 text-[9px]" variant="outline">
                            {m.network_cookies_host_only_badge()}
                          </Badge>
                        {/if}
                      </td>
                      <td class="p-2 font-mono" data-content="" dir="ltr">{cookie.path}</td>
                      <td class="p-2 text-muted-foreground" data-content="">
                        {cookie.expiresAt === null ? m.network_cookies_session() : formatTime(cookie.expiresAt)}
                      </td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {/if}
        </Tabs.Content>
      </Tabs.Root>
    {/if}
  </div>
</div>
