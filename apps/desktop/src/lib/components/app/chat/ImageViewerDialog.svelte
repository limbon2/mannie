<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import { untrack } from 'svelte';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Dialog from '#lib/components/ui/dialog/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import type { ImageView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { loadImage } from '#lib/state/image-previews.svelte.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import { chatFailureMessage, formatCount } from '#lib/utils/chat-errors.js';

  // An image of the owner's message at full size (docs/features/images.md,
  // the transcript). It loads each time the dialog opens, and Escape or
  // "Close" closes it.
  interface Props {
    /** The conversation that holds the image. */
    conversationId: string;
    /** What the image is. */
    image: ImageView;
    /** Whether the dialog shows. */
    open?: boolean;
    /** The image's part. */
    partId: string;
  }

  type Loaded =
    | { readonly failure: ChatFailure; readonly status: 'failed' }
    | { readonly status: 'loading' }
    | { readonly status: 'ready'; readonly url: string };

  let { conversationId, image, open = $bindable(false), partId }: Props = $props();

  let loaded: Loaded = $state({ status: 'loading' });

  /** The latest load, whose answer alone counts. */
  let latest = 0;

  function load(): void {
    latest += 1;
    const request = latest;
    loaded = { status: 'loading' };
    void loadImage(conversationId, partId, 'full').then((outcome) => {
      if (request !== latest) {
        return;
      }
      loaded = outcome.ok ? { status: 'ready', url: outcome.data } : { failure: outcome.failure, status: 'failed' };
    });
  }

  $effect(() => {
    if (open) {
      untrack(load);
    }
  });
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-5xl" showCloseButton={false}>
    <Dialog.Header>
      <Dialog.Title class="truncate" data-content>{image.name}</Dialog.Title>
      <Dialog.Description>
        {m.chat_image_size({ height: formatCount(image.height), width: formatCount(image.width) })}
      </Dialog.Description>
    </Dialog.Header>
    {#if loaded.status === 'ready'}
      <img class="max-h-[70vh] w-full rounded-md object-contain" alt={image.name} src={loaded.url} />
    {:else if loaded.status === 'failed'}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.chat_image_load_failed()}</Alert.Title>
        <Alert.Description>{chatFailureMessage(loaded.failure)}</Alert.Description>
      </Alert.Root>
    {:else}
      <p class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
        <Spinner aria-label={m.chat_image_loading()} />
        <span>{m.chat_image_loading()}</span>
      </p>
    {/if}
    <Dialog.Footer>
      {#if loaded.status === 'failed'}
        <Button variant="outline" onclick={load}>{m.chat_try_again()}</Button>
      {/if}
      <Button variant="outline" onclick={() => (open = false)}>{m.chat_close()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
