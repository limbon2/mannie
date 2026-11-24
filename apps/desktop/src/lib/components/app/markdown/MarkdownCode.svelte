<script lang="ts">
  import { Button } from '#lib/components/ui/button/index.js';
  import * as CodeBlock from '#lib/components/ui/code-block/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { useHighlightedCode } from '#lib/state/highlighted-code.svelte.js';

  interface Props {
    language: string | null;
    value: string;
  }

  let { language, value }: Props = $props();

  let copied = $state<'failed' | 'idle' | 'succeeded'>('idle');

  const highlighted = useHighlightedCode(
    () => value,
    () => language,
  );
  const status = $derived.by(() => {
    if (copied === 'succeeded') {
      return m.markdown_copy_succeeded();
    }
    return copied === 'failed' ? m.markdown_copy_failed() : '';
  });

  /** Copies the code on the owner's click; the webview never reads the clipboard. */
  async function copy(): Promise<void> {
    try {
      await navigator.clipboard.writeText(value);
      copied = 'succeeded';
    } catch {
      // WebKitGTK can refuse the write; the owner learns that it failed.
      copied = 'failed';
    }
  }
</script>

<CodeBlock.Root class="my-2">
  <CodeBlock.Header>
    <CodeBlock.Title class="font-mono text-xs text-muted-foreground" data-content>{language ?? ''}</CodeBlock.Title>
    <CodeBlock.Actions>
      <span class="text-xs text-muted-foreground" role="status">{status}</span>
      <Button size="sm" variant="ghost" onclick={copy}>{m.markdown_copy()}</Button>
    </CodeBlock.Actions>
  </CodeBlock.Header>
  <!-- Long lines wrap, so the code needs no horizontal scrolling of its own. -->
  {#if highlighted.lines !== null}
    <CodeBlock.Content data-content wrap>
      {#each highlighted.lines as line, index (index)}
        <CodeBlock.Line code={line} />
      {/each}
    </CodeBlock.Content>
  {:else}
    <CodeBlock.Content code={value} data-content wrap />
  {/if}
</CodeBlock.Root>
