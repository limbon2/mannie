<script lang="ts">
  import CheckIcon from '@lucide/svelte/icons/check';
  import CopyIcon from '@lucide/svelte/icons/copy';
  import { onDestroy } from 'svelte';

  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';

  // Copies a command, argument block or tool output on the owner's click; the
  // webview never reads the clipboard. Reports WebKitGTK write failures.
  interface Props {
    value: string;
  }

  let { value }: Props = $props();

  let copied = $state<'failed' | 'idle' | 'succeeded'>('idle');
  let timer: ReturnType<typeof setTimeout> | undefined;

  const status = $derived.by(() => {
    if (copied === 'succeeded') {
      return m.tools_details_copied();
    }
    return copied === 'failed' ? m.tools_details_copy_failed() : '';
  });

  async function copy(): Promise<void> {
    clearTimeout(timer);
    try {
      await navigator.clipboard.writeText(value);
      copied = 'succeeded';
      timer = setTimeout(() => {
        copied = 'idle';
      }, 2_000);
    } catch {
      copied = 'failed';
    }
  }

  onDestroy(() => {
    clearTimeout(timer);
  });
</script>

<span class="flex items-center gap-1.5">
  <span class="text-xs text-muted-foreground" role="status">{status}</span>
  <Button size="xs" variant="ghost" onclick={copy}>
    {#if copied === 'succeeded'}
      <CheckIcon data-icon="inline-start" />
    {:else}
      <CopyIcon data-icon="inline-start" />
    {/if}
    {m.tools_details_copy()}
  </Button>
</span>
