<script lang="ts">
  import XIcon from '@lucide/svelte/icons/x';

  import { Button } from '#lib/components/ui/button/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { MessageImages } from '#lib/state/message-images.svelte.js';

  // The images of the message that the owner writes, below the message box
  // (plugins/images/docs/README.md, adding images): a chip for each, with its
  // thumbnail once its source prepared it, and why an image was not added.
  let { images }: { images: MessageImages } = $props();
</script>

{#if images.images.length > 0}
  <ul class="flex flex-wrap gap-2" aria-label={m.chat_image_chips_label()} data-image-chips>
    {#each images.images as image (image.id)}
      <li class="flex max-w-full min-w-0 items-center gap-1.5 rounded-md border bg-muted/50 p-1 text-xs">
        {#if image.ready === null}
          <span class="flex size-10 shrink-0 items-center justify-center rounded-sm bg-muted">
            <Spinner aria-label={m.chat_image_preparing()} />
          </span>
        {:else}
          <img class="size-10 shrink-0 rounded-sm object-cover" alt="" src={image.ready.thumbnailUrl} />
        {/if}
        <span class="max-w-40 min-w-0 truncate" data-content>{image.name}</span>
        <Button
          aria-label={m.chat_image_remove({ name: image.name })}
          size="icon-xs"
          variant="ghost"
          onclick={() => images.remove(image.id)}
        >
          <XIcon />
        </Button>
      </li>
    {/each}
  </ul>
{/if}
{#if images.failures.length > 0}
  <ul class="flex flex-col gap-0.5 text-sm text-destructive" data-image-failures role="alert">
    {#each images.failures as failure, index (index)}
      <li>{failure}</li>
    {/each}
  </ul>
{/if}
