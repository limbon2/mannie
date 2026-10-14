<script lang="ts">
  import ImageIcon from '@lucide/svelte/icons/image';

  import { m } from '#lib/paraglide/messages.js';

  import MarkdownLink from './MarkdownLink.svelte';

  // An image from model output is never loaded: its place shows the
  // description and a link to its address (ADR-0021, decision 5).
  let { description, url }: { description: string; url: string } = $props();
</script>

<span class="inline-flex flex-wrap items-center gap-1 rounded-md border px-1.5 py-0.5 text-muted-foreground">
  <ImageIcon class="size-4 shrink-0" aria-hidden="true" />
  {#if description.trim() === ''}
    <span>{m.markdown_image()}</span>
  {:else}
    <span data-content>{description}</span>
  {/if}
  <MarkdownLink {url}>{m.markdown_image_address()}</MarkdownLink>
</span>
