<script lang="ts">
  import XIcon from '@lucide/svelte/icons/x';

  import * as Attachment from '#lib/components/ui/attachment/index.js';
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
      <li class="max-w-full min-w-0">
        <Attachment.Root class="min-w-0" size="sm" state={image.ready === null ? 'processing' : 'done'}>
          <Attachment.Media variant={image.ready === null ? 'icon' : 'image'}>
            {#if image.ready === null}
              <Spinner aria-label={m.chat_image_preparing()} />
            {:else}
              <img alt="" src={image.ready.thumbnailUrl} />
            {/if}
          </Attachment.Media>
          <Attachment.Content>
            <Attachment.Title class="max-w-40" data-content>{image.name}</Attachment.Title>
          </Attachment.Content>
          <Attachment.Actions>
            <Attachment.Action
              aria-label={m.chat_image_remove({ name: image.name })}
              onclick={() => images.remove(image.id)}
            >
              <XIcon />
            </Attachment.Action>
          </Attachment.Actions>
        </Attachment.Root>
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
