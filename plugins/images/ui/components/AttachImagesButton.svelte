<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Tooltip from '@mannie/plugin-sdk/components/tooltip';
  import ImagePlusIcon from '@mannie/plugin-sdk/icons/image-plus';
  import type { ImageSourceButtonProps } from '@mannie/plugin-sdk/slots';

  import { m } from '../generated/messages.js';
  import { imagesActions } from '../state/images-actions.svelte.js';

  // The Images plugin's button in the message box (plugins/images/docs/README.md,
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
