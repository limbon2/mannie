<script lang="ts">
  import XIcon from '@lucide/svelte/icons/x';

  import * as AttachmentItem from '#lib/components/ui/attachment/index.js';
  import type { Attachment, AttachmentRef } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { sameKey } from '#lib/state/conversation-draft.svelte.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import { attachmentNoun, contextIcon } from '#lib/utils/context-kinds.js';

  // What a conversation has attached, below the message box (ADR-0046,
  // decision 6): a chip for each attachment with the button that detaches it.
  // The attachment that the conversation belongs to has no button, since it
  // stays attached (ADR-0051). An attachment of a plugin that is off shows
  // the label it was attached with, and detaches too.
  interface Props {
    attachments: readonly Attachment[];
    /** Whether the buttons detach nothing now, such as while a detach is on its way. */
    disabled?: boolean;
    /** The attachment that the conversation belongs to, or `null`. */
    home?: AttachmentRef | null;
    /** Detaches an attachment. */
    ondetach: (attachment: Attachment) => void;
  }

  let { attachments, disabled = false, home = null, ondetach }: Props = $props();

  const slots = getShellSlots();

  /** Each attachment with the words of its kind, and whether it is the home. */
  const chips = $derived(
    attachments.map((attachment) => {
      const kind = slots.contextKinds.find(attachment.provider);
      return {
        attachment,
        Icon: contextIcon(kind),
        isHome: home !== null && sameKey(home, attachment),
        key: `${attachment.provider}:${attachment.key}`,
        noun: attachmentNoun(kind),
      };
    }),
  );
</script>

<ul class="flex flex-wrap gap-1.5" aria-label={m.chat_context_chips_label()} data-attachment-chips>
  {#each chips as chip (chip.key)}
    <li class="max-w-full min-w-0">
      <AttachmentItem.Root class="min-w-0" data-home={chip.isHome ? '' : undefined} size="xs">
        <AttachmentItem.Media>
          <chip.Icon />
        </AttachmentItem.Media>
        <AttachmentItem.Content>
          <span class="sr-only">{chip.noun}</span>
          <AttachmentItem.Title data-content>{chip.attachment.label}</AttachmentItem.Title>
        </AttachmentItem.Content>
        {#if !chip.isHome}
          <AttachmentItem.Actions>
            <AttachmentItem.Action
              aria-label={m.chat_context_detach({ label: chip.attachment.label })}
              {disabled}
              onclick={() => ondetach(chip.attachment)}
            >
              <XIcon />
            </AttachmentItem.Action>
          </AttachmentItem.Actions>
        {/if}
      </AttachmentItem.Root>
    </li>
  {/each}
</ul>
