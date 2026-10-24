<script lang="ts">
  import FileOutputIcon from '@lucide/svelte/icons/file-output';
  import ImageIcon from '@lucide/svelte/icons/image';

  import { Button } from '#lib/components/ui/button/index.js';
  import type { ArchiveRef, ConversationId, PartId, ToolCallView, ToolImageRef } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { toolsActions } from '#lib/state/tools-actions.svelte.js';
  import type { ToolKind } from '#lib/types/shell-slots.js';
  import { toolKindCall } from '#lib/utils/tool-kinds.js';
  import { decisionMessage } from '#lib/utils/tools-messages.js';

  import ToolAccessList from './ToolAccessList.svelte';
  import ToolImageDialog from './ToolImageDialog.svelte';

  // The details of a tool call (docs/features/system-tools/tool-activity.md,
  // tool calls in the reply): what it touches, the details that its tool
  // kind shows of a plugin's call (ADR-0039), its arguments as the model
  // wrote them, what allowed it, and the result the model read, with the
  // outputs that did not fit and the images that the model saw (ADR-0044).
  // Everything the model or the system wrote is content, shown as plain
  // text (T-3).
  interface Props {
    call: ToolCallView;
    conversationId: ConversationId;
    /** The tool kind of a plugin's call, which its own plugin registered. */
    kind?: ToolKind | undefined;
    /** The call's part, which holds the images of its result. */
    partId: PartId;
  }

  let { call, conversationId, kind, partId }: Props = $props();

  const id = $props.id();

  /** The image that the viewer shows, once the owner opened one. */
  let shownImage: ToolImageRef | null = $state(null);
  let viewerOpen = $state(false);

  const shownArguments = $derived(formatArguments(call));
  const archives = $derived(call.result?.archives ?? []);
  const images = $derived(call.result?.images ?? []);
  const KindDetails = $derived(kind?.details ?? null);

  /** The arguments, indented when they are complete JSON, and nothing for a call without any. */
  function formatArguments(current: ToolCallView): string {
    const text = current.arguments.trim();
    const isEmpty = text === '' || text === '{}';
    if (isEmpty) {
      return '';
    }
    if (current.argumentsCut) {
      return text;
    }
    try {
      return JSON.stringify(JSON.parse(text), null, 2);
    } catch {
      return text;
    }
  }

  function showImage(image: ToolImageRef): void {
    shownImage = image;
    viewerOpen = true;
  }

  function archiveLabel(archive: ArchiveRef): string {
    switch (archive.name) {
      case 'stderr': {
        return m.tools_output_show_stderr();
      }
      case 'stdout': {
        return m.tools_output_show_stdout();
      }
      default: {
        return m.tools_output_show();
      }
    }
  }
</script>

<div class="flex min-w-0 flex-col gap-3 text-sm">
  {#if call.accesses.length > 0}
    <div class="flex flex-col gap-1.5">
      <p class="text-xs font-medium text-muted-foreground">{m.tools_details_accesses()}</p>
      <ToolAccessList accesses={call.accesses} />
    </div>
  {/if}
  {#if KindDetails !== null}
    <div class="flex min-w-0 flex-col gap-1.5" data-tool-kind-details="">
      <KindDetails call={toolKindCall(call)} {conversationId} />
    </div>
  {/if}
  {#if shownArguments !== ''}
    <div class="flex flex-col gap-1.5">
      <p id="{id}-arguments" class="text-xs font-medium text-muted-foreground">{m.tools_details_arguments()}</p>
      <!-- Focusable, so that the owner can scroll it from the keyboard (WCAG 2.1.1). -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <pre
        class="max-h-64 overflow-auto rounded-md bg-muted px-2 py-1.5 font-mono text-xs break-all whitespace-pre-wrap outline-none focus-visible:ring-2 focus-visible:ring-ring"
        aria-labelledby="{id}-arguments"
        data-content
        role="group"
        tabindex="0">{shownArguments}</pre>
      {#if call.argumentsCut}
        <p class="text-xs text-muted-foreground">{m.tools_details_arguments_cut()}</p>
      {/if}
    </div>
  {/if}
  {#if call.decision !== null}
    <p class="text-xs text-muted-foreground">{decisionMessage(call.decision)}</p>
  {/if}
  {#if call.result !== null}
    <div class="flex flex-col gap-1.5">
      <p id="{id}-result" class="text-xs font-medium text-muted-foreground">{m.tools_details_result()}</p>
      <!-- Focusable, so that the owner can scroll it from the keyboard (WCAG 2.1.1). -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <pre
        class="max-h-80 overflow-auto rounded-md bg-muted px-2 py-1.5 font-mono text-xs break-all whitespace-pre-wrap outline-none focus-visible:ring-2 focus-visible:ring-ring"
        aria-labelledby="{id}-result"
        data-content
        role="group"
        tabindex="0">{call.result.output}</pre>
    </div>
    {#if images.length > 0}
      <div class="flex flex-col gap-1.5" data-tool-images="">
        <p class="text-xs font-medium text-muted-foreground">{m.tools_details_images()}</p>
        <div class="flex flex-wrap gap-2">
          {#each images as image (image.position)}
            <Button size="sm" variant="outline" onclick={() => showImage(image)}>
              <ImageIcon data-icon="inline-start" />
              {m.tools_image_show({ number: image.position + 1 })}
            </Button>
          {/each}
        </div>
      </div>
    {/if}
    {#if archives.length > 0}
      <div class="flex flex-wrap gap-2">
        {#each archives as archive (archive.id)}
          <Button size="sm" variant="outline" onclick={() => toolsActions.showOutput(conversationId, archive)}>
            <FileOutputIcon data-icon="inline-start" />
            {archiveLabel(archive)}
          </Button>
        {/each}
      </div>
    {/if}
  {/if}
</div>
{#if shownImage !== null}
  <ToolImageDialog {conversationId} image={shownImage} {partId} bind:open={viewerOpen} />
{/if}
