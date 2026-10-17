<script lang="ts">
  import { Button } from '#lib/components/ui/button/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { imagePreviews } from '#lib/state/image-previews.svelte.js';
  import type { PartRendererProps } from '#lib/types/shell-slots.js';

  import ImageViewerDialog from './ImageViewerDialog.svelte';

  // An image of the owner's message, below its text and mentions
  // (docs/features/images.md, the transcript): its thumbnail, which opens the
  // image at full size. The core keeps the bytes, so the image shows also
  // while the plugin that added it is off (ADR-0033, decision 8).
  let { conversationId, part }: PartRendererProps = $props();

  let viewing = $state(false);

  const image = $derived(part.content.kind === 'image' ? part.content.image : null);
  const preview = $derived(imagePreviews.thumbnail(part.id));

  $effect(() => {
    if (image !== null) {
      imagePreviews.load(conversationId, part.id);
    }
  });
</script>

{#if image !== null}
  <div class="ms-auto flex w-fit max-w-[85%] min-w-0 flex-col items-end" data-image-part>
    {#if preview.status === 'ready'}
      <button
        class="overflow-hidden rounded-lg border outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50"
        aria-label={m.chat_image_open({ name: image.name })}
        type="button"
        onclick={() => (viewing = true)}
      >
        <img class="max-h-48 max-w-full object-contain" alt="" src={preview.url} />
      </button>
    {:else if preview.status === 'failed'}
      <div class="flex items-center gap-2 rounded-md border px-3 py-2 text-sm" role="alert">
        <span>{m.chat_image_load_failed()}</span>
        <Button size="sm" variant="outline" onclick={() => imagePreviews.retry(conversationId, part.id)}>
          {m.chat_try_again()}
        </Button>
      </div>
    {:else}
      <div class="h-32 max-w-full" style:aspect-ratio="{image.width} / {image.height}">
        <Skeleton class="size-full rounded-lg" />
      </div>
      <span class="sr-only" role="status">{m.chat_image_loading()}</span>
    {/if}
  </div>
  <ImageViewerDialog {conversationId} {image} partId={part.id} bind:open={viewing} />
{/if}
