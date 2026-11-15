<script lang="ts">
  import type { Snippet } from 'svelte';

  import { linkConfirmation } from '#lib/state/link-confirmation.svelte.js';
  import { isFileLink } from '#lib/utils/file-links.js';

  import MarkdownFileLink from './MarkdownFileLink.svelte';

  // A link from model output is a button: the webview never follows it and
  // never connects to its host. Web links require confirmation (ADR-0021);
  // local navigation follows ADR-0069.
  let { children, url }: { children: Snippet; url: string } = $props();
</script>

{#if isFileLink(url)}
  <MarkdownFileLink path={url}>{@render children()}</MarkdownFileLink>
{:else}
  <button
    class="cursor-pointer rounded-sm text-primary underline underline-offset-2 outline-none hover:no-underline focus-visible:ring-2 focus-visible:ring-ring"
    aria-haspopup="dialog"
    data-content
    type="button"
    onclick={() => void linkConfirmation.request(url)}
  >
    {@render children()}
  </button>
{/if}
