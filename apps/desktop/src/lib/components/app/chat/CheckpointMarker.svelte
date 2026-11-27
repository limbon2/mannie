<script lang="ts">
  import BookTextIcon from '@lucide/svelte/icons/book-text';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as Marker from '#lib/components/ui/marker/index.js';
  import { m } from '#lib/paraglide/messages.js';

  // A separator in the transcript marking where a checkpoint summary starts
  // replacing earlier messages or steps for the model (ADR-0083).
  interface Props {
    onShow: () => void;
    text: string;
  }

  let { onShow, text }: Props = $props();
</script>

<Marker.Root class="text-xs" variant="separator">
  <Marker.Content class="flex items-center gap-2">
    <span>{text}</span>

    <Button size="sm" variant="ghost" onclick={onShow}>
      <BookTextIcon data-icon="inline-start" />
      {m.chat_checkpoint_show()}
    </Button>
  </Marker.Content>
</Marker.Root>
