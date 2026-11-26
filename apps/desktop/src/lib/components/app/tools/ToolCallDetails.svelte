<script lang="ts">
  import CodeIcon from '@lucide/svelte/icons/code';
  import FileOutputIcon from '@lucide/svelte/icons/file-output';
  import ImageIcon from '@lucide/svelte/icons/image';

  import { Badge } from '#lib/components/ui/badge/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as CodeBlock from '#lib/components/ui/code-block/index.js';
  import * as Trace from '#lib/components/ui/trace/index.js';
  import type {
    Access,
    ArchiveRef,
    ConversationId,
    PartId,
    ToolCallView,
    ToolImageRef,
  } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { toolsActions } from '#lib/state/tools-actions.svelte.js';
  import type { ToolKind } from '#lib/types/shell-slots.js';
  import { toolKindCall } from '#lib/utils/tool-kinds.js';
  import { decisionMessage } from '#lib/utils/tools-messages.js';

  import AccessText from './AccessText.svelte';
  import ToolAccessList from './ToolAccessList.svelte';
  import ToolCopyButton from './ToolCopyButton.svelte';
  import ToolImageDialog from './ToolImageDialog.svelte';

  // The details of a tool call (docs/features/system-tools/tool-activity.md,
  // tool calls in the reply): plugin details, what it touches, the result the
  // model read (with archives and images, ADR-0044), compact outcome and
  // decision metadata, and the raw JSON arguments as the model wrote them
  // (collapsed when a richer view already describes the call). Everything the
  // model or the system wrote is content, shown as plain text (T-3).
  interface Props {
    call: ToolCallView;
    conversationId: ConversationId;
    /** Whether the parent row already renders a saved file diff for this call. */
    hasFileChange?: boolean;
    /** The tool kind of a plugin's call, which its own plugin registered. */
    kind?: ToolKind | undefined;
    /** Concise command outcome (exit code and duration) shown in the metadata row. */
    outcomeNote?: string | null;
    /** The call's part, which holds the images of its result. */
    partId: PartId;
  }

  let { call, conversationId, hasFileChange = false, kind, outcomeNote = null, partId }: Props = $props();

  const id = $props.id();

  /** The image that the viewer shows, once the owner opened one. */
  let shownImage: ToolImageRef | null = $state(null);
  let viewerOpen = $state(false);
  let argumentsOpen = $state(false);

  const shownArguments = $derived(formatArguments(call));
  const archives = $derived(call.result?.archives ?? []);
  const images = $derived(call.result?.images ?? []);
  const KindDetails = $derived(kind?.details ?? null);
  const hasUnheadedAccess = $derived(call.accesses.some(lacksBlockHeader));
  const hasAccesses = $derived(call.accesses.length > 0);
  const hasKindDetails = $derived(KindDetails !== null);
  const collapseArguments = $derived(hasAccesses || hasKindDetails || hasFileChange);
  const decision = $derived(call.decision === null ? null : decisionMessage(call.decision));
  const hasOutcomeNote = $derived(outcomeNote !== null);
  const hasDecision = $derived(decision !== null);
  const hasMetadata = $derived(hasOutcomeNote || hasDecision);

  /** Whether an access has no CodeBlock header of its own, so the section label introduces it. */
  function lacksBlockHeader(access: Access): boolean {
    switch (access.kind) {
      case 'app_config':
      case 'browser_action':
      case 'browser_control':
      case 'network_request':
      case 'path':
      case 'project':
      case 'proxmox_vm': {
        return true;
      }
      case 'command':
      case 'database':
      case 'external_tool':
      case 'program':
      case 'remote_command':
      case 'terminal_command': {
        return false;
      }
    }
  }

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
  {#if KindDetails !== null}
    <div class="flex min-w-0 flex-col gap-1.5" data-tool-kind-details="">
      <KindDetails call={toolKindCall(call)} {conversationId} />
    </div>
  {/if}

  {#if call.accesses.length > 0}
    <div class="flex flex-col gap-1.5">
      {#if hasUnheadedAccess}
        <p class="text-xs font-medium text-muted-foreground">{m.tools_details_accesses()}</p>
      {/if}
      <ToolAccessList accesses={call.accesses} copyable />
    </div>
  {/if}

  {#if call.result !== null}
    <CodeBlock.Root size="sm">
      <CodeBlock.Header>
        <CodeBlock.Title id="{id}-result" class="text-xs font-medium text-muted-foreground">
          {m.tools_details_result()}
        </CodeBlock.Title>
        <CodeBlock.Actions>
          {#each archives as archive (archive.id)}
            <Button size="xs" variant="ghost" onclick={() => toolsActions.showOutput(conversationId, archive)}>
              <FileOutputIcon data-icon="inline-start" />
              {archiveLabel(archive)}
            </Button>
          {/each}
          <ToolCopyButton value={call.result.output} />
        </CodeBlock.Actions>
      </CodeBlock.Header>

      <CodeBlock.Content
        class="max-h-64"
        aria-labelledby="{id}-result"
        code={call.result.output}
        data-content
        role="group"
        wrap
      />
    </CodeBlock.Root>

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
  {/if}

  {#if hasMetadata}
    <div class="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground" data-tool-metadata="">
      {#if outcomeNote !== null}
        <Badge variant="secondary">{outcomeNote}</Badge>
      {/if}
      {#if decision !== null}
        <span>{decision}</span>
      {/if}
    </div>
  {/if}

  {#if shownArguments !== ''}
    {#if collapseArguments}
      <Trace.Collapsible bind:open={argumentsOpen}>
        <Trace.CollapsibleTrigger data-tool-arguments-toggle="">
          <Trace.Line class="text-xs">
            <Trace.Icon>
              <CodeIcon />
            </Trace.Icon>
            <Trace.Title>{m.tools_details_arguments()}</Trace.Title>
          </Trace.Line>
        </Trace.CollapsibleTrigger>

        <Trace.CollapsibleContent class="flex flex-col gap-1.5 text-foreground">
          <AccessText
            contentClass="max-h-56"
            copyable
            label={m.tools_details_arguments()}
            language="json"
            text={shownArguments}
          />
          {#if call.argumentsCut}
            <p class="text-xs text-muted-foreground">{m.tools_details_arguments_cut()}</p>
          {/if}
        </Trace.CollapsibleContent>
      </Trace.Collapsible>
    {:else}
      <div class="flex flex-col gap-1.5">
        <AccessText
          contentClass="max-h-56"
          copyable
          label={m.tools_details_arguments()}
          language="json"
          text={shownArguments}
        />
        {#if call.argumentsCut}
          <p class="text-xs text-muted-foreground">{m.tools_details_arguments_cut()}</p>
        {/if}
      </div>
    {/if}
  {/if}
</div>

{#if shownImage !== null}
  <ToolImageDialog {conversationId} image={shownImage} {partId} bind:open={viewerOpen} />
{/if}
