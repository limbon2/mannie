<script lang="ts">
  import type { Snippet } from 'svelte';

  import * as CodeBlock from '#lib/components/ui/code-block/index.js';
  import { useHighlightedCode } from '#lib/state/highlighted-code.svelte.js';

  import ToolCopyButton from './ToolCopyButton.svelte';

  // A command, statement or run of an installed program as an operation's
  // accesses show it: monospaced, wrapped and scrollable. It is content,
  // shown as plain text (T-3). Approvals leave `copyable` off so no button
  // sits inside the access description before the decision buttons.
  interface Props {
    contentClass?: string;
    copyable?: boolean;
    footer?: Snippet;
    label: string;
    /** Optional language for lazy Shiki syntax highlighting (ADR-0079). */
    language?: string | null;
    text: string;
  }

  let { contentClass = 'max-h-36', copyable = false, footer, label, language = null, text }: Props = $props();

  const highlighted = useHighlightedCode(
    () => text,
    () => language,
  );
</script>

<CodeBlock.Root size="sm">
  <CodeBlock.Header>
    <CodeBlock.Title class="text-xs font-medium text-muted-foreground">{label}</CodeBlock.Title>
    {#if copyable}
      <CodeBlock.Actions>
        <ToolCopyButton value={text} />
      </CodeBlock.Actions>
    {/if}
  </CodeBlock.Header>

  {#if highlighted.lines !== null}
    <CodeBlock.Content class={contentClass} aria-label={label} data-content role="group" wrap>
      {#each highlighted.lines as line, index (index)}
        <CodeBlock.Line code={line} />
      {/each}
    </CodeBlock.Content>
  {:else}
    <CodeBlock.Content class={contentClass} aria-label={label} code={text} data-content role="group" wrap />
  {/if}

  {#if footer !== undefined}
    <CodeBlock.Footer class="flex-wrap gap-x-4 gap-y-1 border-t bg-muted/30 pt-2 text-xs">
      {@render footer()}
    </CodeBlock.Footer>
  {/if}
</CodeBlock.Root>
