<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import { Skeleton } from '@mannie/plugin-sdk/components/skeleton';
  import { formatTime } from '@mannie/plugin-sdk/format';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import { m } from '../generated/messages.js';
  import { mcpActions } from '../state/mcp-actions.svelte.js';
  import { logEventMessage, mcpFailureMessage } from '../utils/mcp-messages.js';

  // The log of a server (plugins/mcp/docs/README.md, Show log): the lines of
  // its error output, which are the server's own text and read left to
  // right, and what happened to its connections, in the owner's words. The
  // core replaced the server's secrets in both, and keeps the log in memory.
  // Only the entries scroll, so the title and the buttons stay in view.
  const request = $derived(mcpActions.log);
  const entries = $derived(request?.log?.entries ?? []);
  /** Whether the log loads for the first time, which shows a skeleton. */
  const loadsFirst = $derived(request !== null && request.loading && request.log === null);
</script>

<Dialog.Root bind:open={mcpActions.logOpen}>
  <Dialog.Content class="flex max-h-[90vh] flex-col sm:max-w-3xl" showCloseButton={false}>
    {#if request !== null}
      <div class="flex min-h-0 flex-col gap-4" data-mcp-log-dialog="">
        <Dialog.Header>
          <Dialog.Title>{m.mcp_log_title({ server: request.server.name })}</Dialog.Title>
          <Dialog.Description>{m.mcp_log_description()}</Dialog.Description>
        </Dialog.Header>
        {#if loadsFirst}
          <div class="flex flex-col gap-2" aria-busy="true">
            <span class="sr-only" role="status">{m.mcp_log_loading()}</span>
            <Skeleton class="h-6 w-full" />
            <Skeleton class="h-6 w-full" />
          </div>
        {:else if request.failure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title>{m.mcp_log_failed_title()}</Alert.Title>
            <Alert.Description>{mcpFailureMessage(request.failure)}</Alert.Description>
          </Alert.Root>
        {:else if entries.length === 0}
          <p class="text-sm text-muted-foreground">{m.mcp_log_empty()}</p>
        {:else}
          {#if request.log?.cut === true}
            <p class="text-sm text-muted-foreground">{m.mcp_log_cut()}</p>
          {/if}
          <!-- Focusable, so that the owner can scroll it from the keyboard (WCAG 2.1.1). -->
          <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
          <ol
            class="flex min-h-0 flex-col gap-1 overflow-y-auto rounded-md border p-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={m.mcp_log_entries_label()}
            tabindex="0"
          >
            {#each entries as entry, index (index)}
              <li class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3" data-mcp-log-entry={entry.kind}>
                <span class="text-muted-foreground tabular-nums">{formatTime(entry.at)}</span>
                {#if entry.kind === 'output'}
                  <span class="font-mono wrap-anywhere whitespace-pre-wrap" data-content dir="ltr">{entry.text}</span>
                {:else}
                  <span>{logEventMessage(entry.event)}</span>
                {/if}
              </li>
            {/each}
          </ol>
        {/if}
        <Dialog.Footer>
          <Button
            data-mcp-log-refresh=""
            disabled={request.loading}
            type="button"
            variant="outline"
            onclick={() => void mcpActions.loadLog(request)}
          >
            {m.mcp_log_refresh()}
          </Button>
          <Button type="button" onclick={() => (mcpActions.logOpen = false)}>{m.mcp_close()}</Button>
        </Dialog.Footer>
      </div>
    {/if}
  </Dialog.Content>
</Dialog.Root>
