<script lang="ts">
  import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';

  import * as Attachment from '#lib/components/ui/attachment/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { MentionChoice } from '#lib/state/message-mentions.svelte.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import { mentionIcon, mentionVariantName, mentionWarning } from '#lib/utils/mention-kinds.js';

  // What the message that the owner writes mentions, below the message box
  // (docs/features/chat/mentions.md), with the warning of a mention that has
  // one, such as a file that may hold secrets. Deleting a token from the text
  // removes its chip.
  let { mentions }: { mentions: readonly MentionChoice[] } = $props();

  const slots = getShellSlots();

  /** Each mention with the words of its kind. */
  const chips = $derived(
    mentions.map((mention) => {
      const kind = slots.mentionKinds.find(mention.provider);
      return {
        Icon: mentionIcon(kind, mention.entry.variant),
        key: `${mention.provider}:${mention.entry.key}`,
        label: mention.entry.label,
        variantName: mentionVariantName(kind, mention.entry.variant),
        warning: mention.entry.warning === null ? null : mentionWarning(kind, mention.entry.warning),
      };
    }),
  );
</script>

<ul class="flex flex-wrap gap-1.5" aria-label={m.chat_mention_chips_label()} data-mention-chips>
  {#each chips as chip (chip.key)}
    <li class="max-w-full min-w-0">
      <Attachment.Root class="min-w-0" size="xs" state={chip.warning !== null ? 'error' : 'done'}>
        <Attachment.Media>
          <chip.Icon />
        </Attachment.Media>
        <Attachment.Content>
          <span class="sr-only">{chip.variantName}</span>
          <Attachment.Title class="font-mono" data-content>{chip.label}</Attachment.Title>
          {#if chip.warning !== null}
            <Attachment.Description class="flex items-center gap-1" data-mention-warning>
              <TriangleAlertIcon class="size-3 shrink-0" aria-hidden="true" />
              <span>{chip.warning}</span>
            </Attachment.Description>
          {/if}
        </Attachment.Content>
      </Attachment.Root>
    </li>
  {/each}
</ul>
