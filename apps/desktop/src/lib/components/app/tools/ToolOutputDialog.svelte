<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Dialog from '#lib/components/ui/dialog/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import type { ArchiveRef } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { type OutputRequest, toolsActions } from '#lib/state/tools-actions.svelte.js';
  import type { ToolsFailure } from '#lib/types/tools.js';
  import { formatBytes, toolsFailureMessage } from '#lib/utils/tools-messages.js';

  // The whole of an output that did not fit into a tool call's result
  // (docs/features/system-tools/tool-activity.md, tool calls in the reply),
  // read from the core page by page. The output is content and renders as
  // text. `toolsActions.showOutput` opens it.
  const request = $derived(toolsActions.output);
  const title = $derived(request === null ? m.tools_output_title() : archiveTitle(request.archive));
  const firstPageFailure = $derived(request === null ? null : failureBeforeOutput(request));
  const firstPageLoading = $derived(request !== null && request.totalLines === null && request.failure === null);

  /** A failure before the first page, which replaces the output. A later one shows below it. */
  function failureBeforeOutput(current: OutputRequest): ToolsFailure | null {
    return current.lines.length === 0 ? current.failure : null;
  }

  function archiveTitle(archive: ArchiveRef): string {
    switch (archive.name) {
      case 'stderr': {
        return m.tools_output_title_stderr();
      }
      case 'stdout': {
        return m.tools_output_title_stdout();
      }
      default: {
        return m.tools_output_title();
      }
    }
  }
</script>

<Dialog.Root bind:open={toolsActions.outputOpen}>
  <Dialog.Content class="sm:max-w-3xl" showCloseButton={false}>
    <Dialog.Header>
      <Dialog.Title>{title}</Dialog.Title>
      <Dialog.Description>
        {#if request !== null}
          {m.tools_output_description({ lines: request.archive.lines, size: formatBytes(request.archive.bytes) })}
        {/if}
      </Dialog.Description>
    </Dialog.Header>
    {#if request !== null}
      {#if firstPageFailure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{m.tools_output_failed_title()}</Alert.Title>
          <Alert.Description>{toolsFailureMessage(firstPageFailure)}</Alert.Description>
          <Alert.Action>
            <Button disabled={request.loading} size="sm" variant="outline" onclick={() => void request.loadMore()}>
              {m.tools_try_again()}
            </Button>
          </Alert.Action>
        </Alert.Root>
      {:else if firstPageLoading}
        <div class="flex flex-col gap-2" aria-busy="true">
          <span class="sr-only" role="status">{m.tools_loading()}</span>
          <Skeleton class="h-4 w-3/4" />
          <Skeleton class="h-4 w-full" />
          <Skeleton class="h-4 w-2/3" />
        </div>
      {:else if request.totalLines === 0}
        <p class="text-sm text-muted-foreground">{m.tools_output_empty()}</p>
      {:else}
        <!-- Focusable, so that the owner can scroll the output from the keyboard (WCAG 2.1.1). -->
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <pre
          class="max-h-[60vh] overflow-auto rounded-md bg-muted px-3 py-2 font-mono text-xs break-all whitespace-pre-wrap outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label={title}
          data-content
          role="group"
          tabindex="0">{request.lines.join('\n')}</pre>
        <div class="flex flex-wrap items-center gap-3">
          <p class="text-xs text-muted-foreground" role="status">
            {m.tools_output_shown({ shown: request.lines.length, total: request.totalLines ?? 0 })}
          </p>
          {#if request.hasMore}
            <Button disabled={request.loading} size="sm" variant="outline" onclick={() => void request.loadMore()}>
              {#if request.loading}
                <Spinner aria-label={m.tools_loading()} data-icon="inline-start" />
              {/if}
              {m.tools_output_load_more()}
            </Button>
          {/if}
        </div>
        {#if request.failure !== null}
          <p class="text-sm text-destructive" role="alert">{toolsFailureMessage(request.failure)}</p>
        {/if}
      {/if}
    {/if}
    <Dialog.Footer>
      <Button variant="outline" onclick={() => (toolsActions.outputOpen = false)}>{m.tools_close()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
