<script lang="ts">
  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';

  let { language, value }: { language: string | null; value: string } = $props();

  let copied = $state<'failed' | 'idle' | 'succeeded'>('idle');

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

<figure class="my-2 overflow-hidden rounded-md border">
  <figcaption class="flex items-center justify-between gap-2 border-b bg-muted px-3 py-1 text-xs text-muted-foreground">
    <span class="truncate font-mono" data-content>{language ?? ''}</span>
    <span class="flex items-center gap-2">
      <span role="status">{status}</span>
      <Button size="sm" variant="ghost" onclick={copy}>{m.markdown_copy()}</Button>
    </span>
  </figcaption>
  <!-- Long lines wrap, so the code needs no scrolling region of its own. -->
  <pre class="p-3 font-mono text-sm wrap-break-word whitespace-pre-wrap"><code data-content>{value}</code></pre>
</figure>
