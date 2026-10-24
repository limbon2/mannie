<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import { untrack } from 'svelte';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Dialog from '#lib/components/ui/dialog/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import type { ConversationId, PartId, ToolImageRef } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { loadToolImage } from '#lib/state/tools-actions.svelte.js';
  import type { ToolsFailure } from '#lib/types/tools.js';
  import { toolsFailureMessage } from '#lib/utils/tools-messages.js';

  // An image of a tool call's result at full size, as the model saw it
  // (ADR-0044, decision 7). It loads each time the dialog opens, and Escape
  // or "Close" closes it. A tool returned it, so it is content.
  interface Props {
    /** The conversation that holds the call. */
    conversationId: ConversationId;
    /** The image. */
    image: ToolImageRef;
    /** Whether the dialog shows. */
    open?: boolean;
    /** The call's part. */
    partId: PartId;
  }

  type Loaded =
    | { readonly failure: ToolsFailure; readonly status: 'failed' }
    | { readonly status: 'loading' }
    | { readonly status: 'ready'; readonly url: string };

  let { conversationId, image, open = $bindable(false), partId }: Props = $props();

  let loaded: Loaded = $state({ status: 'loading' });

  /** The latest load, whose answer alone counts. */
  let latest = 0;

  const number = $derived(image.position + 1);
  const counts = new Intl.NumberFormat();

  function load(): void {
    latest += 1;
    const request = latest;
    loaded = { status: 'loading' };
    void loadToolImage(conversationId, partId, image.position).then((outcome) => {
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
      <Dialog.Title>{m.tools_image_title({ number })}</Dialog.Title>
      <Dialog.Description>
        {m.tools_image_size({ height: counts.format(image.height), width: counts.format(image.width) })}
      </Dialog.Description>
    </Dialog.Header>
    {#if loaded.status === 'ready'}
      <img class="max-h-[70vh] w-full rounded-md object-contain" alt={m.tools_image_alt({ number })} src={loaded.url} />
    {:else if loaded.status === 'failed'}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.tools_image_load_failed()}</Alert.Title>
        <Alert.Description>{toolsFailureMessage(loaded.failure)}</Alert.Description>
      </Alert.Root>
    {:else}
      <p class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
        <Spinner aria-label={m.tools_image_loading()} />
        <span>{m.tools_image_loading()}</span>
      </p>
    {/if}
    <Dialog.Footer>
      {#if loaded.status === 'failed'}
        <Button variant="outline" onclick={load}>{m.tools_try_again()}</Button>
      {/if}
      <Button variant="outline" onclick={() => (open = false)}>{m.tools_close()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
