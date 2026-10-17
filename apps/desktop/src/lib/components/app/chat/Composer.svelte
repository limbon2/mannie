<script lang="ts">
  import SendIcon from '@lucide/svelte/icons/send';
  import SquareIcon from '@lucide/svelte/icons/square';
  import { untrack } from 'svelte';
  import type { Attachment } from 'svelte/attachments';

  import { Button } from '#lib/components/ui/button/index.js';
  import { Textarea } from '#lib/components/ui/textarea/index.js';
  import * as Tooltip from '#lib/components/ui/tooltip/index.js';
  import type { MentionPlace, MentionRef } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { type MentionChoice, MessageMentions } from '#lib/state/message-mentions.svelte.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';

  import MentionChips from './MentionChips.svelte';
  import MentionPicker from './MentionPicker.svelte';

  // The message box (docs/features/chat/conversation-view.md, the message
  // box): Enter sends, Shift+Enter starts a new line, and Escape stops the
  // reply while the box has the focus. The text stays when sending fails.
  // "@" opens the list of what the message can mention, whose keys come first
  // while it shows (docs/features/chat/mentions.md).
  interface Props {
    /** Whether the box takes no message, such as without a provider profile. */
    disabled?: boolean;
    /** Where the message is written, which decides what it can mention (ADR-0032). */
    mentionPlace: MentionPlace;
    /** Sends a message with what it mentions; resolves to why it failed, or `null`. */
    onSend: (text: string, mentions: MentionRef[]) => Promise<ChatFailure | null>;
    /** Stops the reply that is being written, when there can be one. */
    onStop?: () => Promise<void>;
    /** Whether a reply is being written, which the button then stops. */
    replying?: boolean;
    /**
     * Whether the box takes the focus once it is enabled, as the box of a
     * conversation that the first message of a new conversation just opened.
     */
    takeFocus?: boolean;
  }

  let { disabled = false, mentionPlace, onSend, onStop, replying = false, takeFocus = false }: Props = $props();

  const slots = getShellSlots();
  const uid = $props.id();
  const listId = `${uid}-mentions`;

  // The pane asks for the focus only when it opens.
  let focusPending = untrack(() => takeFocus);

  let box: HTMLTextAreaElement | null = $state(null);
  let text = $state('');
  let sending = $state(false);
  let failure = $state<ChatFailure | null>(null);

  const mentions = new MessageMentions(
    () => mentionPlace,
    (provider) => slots.mentionKinds.find(provider),
  );

  const hasText = $derived(text.trim() !== '');
  const canSend = $derived(hasText && !disabled && !sending && !replying);
  const mentioned = $derived(mentions.inText(text));
  const listShowsEntries = $derived(mentions.open && mentions.choices.length > 0);
  const hasActiveOption = $derived(listShowsEntries && mentions.active !== undefined);
  const activeOption = $derived(hasActiveOption ? optionId(mentions.highlighted) : undefined);

  function optionId(index: number): string {
    return `${uid}-mention-${String(index)}`;
  }

  /** Follows the caret, with which the "@" list opens and closes. */
  function follow(): void {
    if (box !== null) {
      mentions.update(box.value, box.selectionStart);
    }
  }

  function choose(choice: MentionChoice): void {
    if (box === null) {
      return;
    }
    const inserted = mentions.choose(box.value, choice);
    text = inserted.text;
    // The box shows the new text and caret at once, so that the events of
    // the key that chose, such as its keyup, read them.
    box.value = inserted.text;
    box.focus();
    box.setSelectionRange(inserted.caret, inserted.caret);
  }

  async function chooseWhenAnswered(): Promise<void> {
    const choice = await mentions.whenAnswered();
    if (choice !== undefined) {
      choose(choice);
    }
  }

  async function send(): Promise<void> {
    if (!canSend) {
      return;
    }
    sending = true;
    failure = null;
    const result = await onSend(text, mentions.refs(text));
    sending = false;
    if (result === null) {
      text = '';
      mentions.reset();
    } else {
      failure = result;
    }
  }

  /** Handles a key of the "@" list, and says whether it was one. */
  function handleListKey(event: KeyboardEvent): boolean {
    if (!mentions.open || event.isComposing) {
      return false;
    }
    // The first Escape closes the list; the next one stops the reply.
    if (event.key === 'Escape') {
      mentions.dismiss();
      return true;
    }
    // An Enter or Tab before the answer for what the owner typed chooses
    // from that answer, never from an earlier one.
    const choosesEntry = !event.shiftKey && (event.key === 'Enter' || event.key === 'Tab');
    if (choosesEntry && !mentions.settled) {
      void chooseWhenAnswered();
      return true;
    }
    const choice = mentions.active;
    if (choice === undefined) {
      return false;
    }
    switch (event.key) {
      case 'ArrowDown': {
        mentions.move(1);
        return true;
      }
      case 'ArrowUp': {
        mentions.move(-1);
        return true;
      }
      case 'Enter':
      case 'Tab': {
        if (event.shiftKey) {
          return false;
        }
        choose(choice);
        return true;
      }
      default: {
        return false;
      }
    }
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (handleListKey(event)) {
      event.preventDefault();
      return;
    }
    const sends = event.key === 'Enter' && !event.shiftKey && !event.isComposing;
    if (sends) {
      event.preventDefault();
      void send();
      return;
    }
    const stops = event.key === 'Escape' && replying;
    if (stops) {
      event.preventDefault();
      void onStop?.();
    }
  }

  function handleSubmit(event: SubmitEvent): void {
    event.preventDefault();
    void send();
  }

  /**
   * Gives the box the focus once it is enabled, when its pane asked for it.
   * The pane replaced the one whose box the owner wrote in, so the focus fell
   * to the page; an element that took the focus since then keeps it.
   */
  const takeFocusWhenEnabled: Attachment<HTMLTextAreaElement> = (element) => {
    const takesFocusNow = focusPending && !disabled;
    if (!takesFocusNow) {
      return;
    }
    focusPending = false;
    const focusIsLost = document.activeElement === null || document.activeElement === document.body;
    if (focusIsLost) {
      element.focus();
    }
  };
</script>

<form class="relative flex flex-col gap-2 border-t bg-background p-3" onsubmit={handleSubmit}>
  {#if mentions.open}
    <MentionPicker {listId} {mentions} {optionId} onChoose={choose} />
  {/if}
  <div class="flex items-end gap-2">
    <Textarea
      class="max-h-48 min-h-10 resize-none"
      {@attach takeFocusWhenEnabled}
      aria-activedescendant={activeOption}
      aria-autocomplete="list"
      aria-controls={listShowsEntries ? listId : undefined}
      aria-describedby="composer-hint"
      aria-label={m.chat_composer_label()}
      {disabled}
      placeholder={m.chat_composer_placeholder()}
      rows={1}
      bind:ref={box}
      bind:value={text}
      onblur={() => mentions.close()}
      onclick={follow}
      oninput={follow}
      onkeydown={handleKeydown}
      onkeyup={follow}
    />
    {#if replying}
      <Button type="button" variant="outline" onclick={() => void onStop?.()}>
        <SquareIcon data-icon="inline-start" />
        {m.chat_stop()}
      </Button>
    {:else}
      <Tooltip.Root>
        <Tooltip.Trigger disabled={!canSend} type="submit">
          {#snippet child({ props })}
            <Button {...props} aria-label={m.chat_send()} size="icon">
              <SendIcon />
            </Button>
          {/snippet}
        </Tooltip.Trigger>
        <Tooltip.Content>{m.chat_send()}</Tooltip.Content>
      </Tooltip.Root>
    {/if}
  </div>
  {#if mentioned.length > 0}
    <MentionChips mentions={mentioned} />
  {/if}
  {#if failure !== null}
    <p class="text-sm text-destructive" role="alert">
      {chatFailureMessage(failure, (provider) => slots.mentionKinds.find(provider))}
    </p>
  {/if}
  <p id="composer-hint" class="text-xs text-muted-foreground">{m.chat_composer_hint()}</p>
</form>
