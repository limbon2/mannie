<script lang="ts">
  import ImagePlusIcon from '@lucide/svelte/icons/image-plus';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as Tooltip from '#lib/components/ui/tooltip/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { imagesActions } from '#lib/state/images-actions.svelte.js';
  import type { ImageSourceButtonProps } from '#lib/types/shell-slots.js';

  // The Images plugin's button in the message box (docs/features/images.md,
  // adding images): it opens the system's file dialog, and is the keyboard's
  // way to add images that a paste or a drop would add.
  let { disabled, target }: ImageSourceButtonProps = $props();

  let picking = $state(false);

  async function pick(): Promise<void> {
    picking = true;
    await imagesActions.pick(target);
    picking = false;
  }
</script>

<Tooltip.Root>
  <Tooltip.Trigger disabled={disabled || picking}>
    {#snippet child({ props })}
      <Button
        {...props}
        aria-label={m.images_attach()}
        data-attach-images
        size="icon"
        type="button"
        variant="outline"
        onclick={() => void pick()}
      >
        <ImagePlusIcon />
      </Button>
    {/snippet}
  </Tooltip.Trigger>
  <Tooltip.Content>{m.images_attach()}</Tooltip.Content>
</Tooltip.Root>
