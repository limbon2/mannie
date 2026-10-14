<script lang="ts">
  import type { Snippet } from 'svelte';

  import { linkConfirmation } from '#lib/state/link-confirmation.svelte.js';

  // A link from model output is a button: the webview never follows it and
  // never connects to its host. The owner confirms it first (ADR-0021).
  let { children, url }: { children: Snippet; url: string } = $props();
</script>

<button
  class="cursor-pointer rounded-sm text-primary underline underline-offset-2 outline-none hover:no-underline focus-visible:ring-2 focus-visible:ring-ring"
  aria-haspopup="dialog"
  data-content
  type="button"
  onclick={() => void linkConfirmation.request(url)}
>
  {@render children()}
</button>
