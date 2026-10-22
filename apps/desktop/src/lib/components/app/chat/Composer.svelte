<script lang="ts">
  import ArrowUpIcon from '@lucide/svelte/icons/arrow-up';
  import SquareIcon from '@lucide/svelte/icons/square';
  import { type Snippet, untrack } from 'svelte';
  import type { Attachment } from 'svelte/attachments';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as InputGroup from '#lib/components/ui/input-group/index.js';
  import * as Tooltip from '#lib/components/ui/tooltip/index.js';
  import type { ImageRef, MentionPlace, MentionRef } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { MessageImages } from '#lib/state/message-images.svelte.js';
  import { type MentionChoice, MessageMentions } from '#lib/state/message-mentions.svelte.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';
  import { assignFiles, carriesFiles, pastedFiles } from '#lib/utils/transfer-files.js';

  import AttachedImages from './AttachedImages.svelte';
  import MentionChips from './MentionChips.svelte';
  import MentionPicker from './MentionPicker.svelte';

  // The message box (docs/features/chat/conversation-view.md, the message
  // box): one framed field with the text on top and a row of buttons below
  // it (ADR-0042, decision 7). Enter sends, Shift+Enter starts a new line,
  // and Escape stops the reply while the box has the focus. The text stays
  // when sending fails. "@" opens the list of what the message can mention,
  // whose keys come first while it shows (docs/features/chat/mentions.md).
  // Image sources add their buttons at the start of the row, and take the
  // files of a paste or a drop (ADR-0033, decision 7); the images stay too
  // when sending fails.
  interface Props {
    /**
     * The controls of the row after the image sources' buttons, such as the
     * conversation's full access (ADR-0042, decision 4).
     */
    controls?: Snippet;
    /** Whether the box takes no message, such as without a provider profile. */
    disabled?: boolean;
    /** Where the message is written, which decides what it can mention (ADR-0032). */
    mentionPlace: MentionPlace;
    /** Sends a message with what it mentions and its images; resolves to why it failed, or `null`. */
    onSend: (text: string, mentions: MentionRef[], images: ImageRef[]) => Promise<ChatFailure | null>;
    /** Stops the reply that is being written, when there can be one. */
    onStop?: () => Promise<void>;
    /** Whether a reply is being written, which the button then stops. */
    replying?: boolean;
    /** What the row shows before Send, such as the conversation's context meter. */
    status?: Snippet;
    /**
     * Whether the box takes the focus once it is enabled, as the box of a
     * conversation that the first message of a new conversation just opened.
     */
    takeFocus?: boolean;
  }

  let {
    controls,
    disabled = false,
    mentionPlace,
    onSend,
    onStop,
    replying = false,
    status,
    takeFocus = false,
  }: Props = $props();

  const slots = getShellSlots();
  const uid = $props.id();
  const listId = `${uid}-mentions`;

  // The pane asks for the focus only when it opens.
  let focusPending = untrack(() => takeFocus);

  let box: HTMLTextAreaElement | null = $state(null);
  let text = $state('');
  let sending = $state(false);
  let failure = $state<ChatFailure | null>(null);
  /** How many drags with files are over the box, which counts the entries into its children. */
  let dragDepth = $state(0);

  const mentions = new MessageMentions(
    () => mentionPlace,
    (provider) => slots.mentionKinds.find(provider),
  );
  const images = new MessageImages();
  /** The sources of images, in the order of their buttons. */
  const sources = $derived([...slots.imageSources.entries].sort((left, right) => left.order - right.order));

  const hasText = $derived(text.trim() !== '');
  const hasContent = $derived(hasText || images.hasReady);
  const canSend = $derived(hasContent && !images.preparing && !disabled && !sending && !replying);
  const takesFiles = $derived(sources.length > 0 && !disabled);
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
    const result = await onSend(text, mentions.refs(text), images.refs());
    sending = false;
    if (result === null) {
      text = '';
      mentions.reset();
      images.reset();
    } else {
      failure = result;
    }
  }

  /** Gives files to the sources that take them, and says which no source takes. */
  function takeFiles(files: readonly File[]): void {
    const { assigned, refused } = assignFiles(files, sources);
    for (const [source, taken] of assigned) {
      source.take(taken, images);
    }
    for (const file of refused) {
      images.refuse(m.chat_image_not_taken({ name: file.name }));
    }
  }

  /** A paste of files adds images; a paste that carries text pastes the text, as before. */
  function handlePaste(event: ClipboardEvent): void {
    if (!takesFiles) {
      return;
    }
    const files = event.clipboardData === null ? [] : pastedFiles(event.clipboardData);
    if (files.length === 0) {
      return;
    }
    event.preventDefault();
    takeFiles(files);
  }

  /** Whether the box takes the files that this drag carries. */
  function takesDrag(event: DragEvent): boolean {
    return takesFiles && carriesFiles(event.dataTransfer);
  }

  function handleDragEnter(event: DragEvent): void {
    if (!takesDrag(event)) {
      return;
    }
    event.preventDefault();
    dragDepth += 1;
  }

  function handleDragOver(event: DragEvent): void {
    if (!takesDrag(event)) {
      return;
    }
    event.preventDefault();
    if (event.dataTransfer !== null) {
      event.dataTransfer.dropEffect = 'copy';
    }
  }

  function handleDragLeave(event: DragEvent): void {
    if (carriesFiles(event.dataTransfer)) {
      dragDepth = Math.max(0, dragDepth - 1);
    }
  }

  function handleDrop(event: DragEvent): void {
    dragDepth = 0;
    if (!takesDrag(event)) {
      return;
    }
    event.preventDefault();
    takeFiles(Array.from(event.dataTransfer?.files ?? []));
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

<form
  class={[
    'relative mx-auto flex w-full max-w-3xl flex-col gap-2 px-4 pt-2 pb-3',
    dragDepth > 0 && 'outline-2 -outline-offset-4 outline-primary outline-dashed',
  ]}
  data-composer
  ondragenter={handleDragEnter}
  ondragleave={handleDragLeave}
  ondragover={handleDragOver}
  ondrop={handleDrop}
  onsubmit={handleSubmit}
>
  {#if mentions.open}
    <MentionPicker {listId} {mentions} {optionId} onChoose={choose} />
  {/if}
  {#if dragDepth > 0}
    <p class="text-sm text-primary" role="status">{m.chat_image_drop_hint()}</p>
  {/if}
  <!--
    The field never dims as a whole: a disabled text or Send dims itself, so
    the row's other controls, such as the context meter, keep their contrast.
  -->
  <InputGroup.Root
    class="rounded-3xl has-disabled:bg-transparent has-disabled:opacity-100 dark:has-disabled:bg-input/30"
  >
    <InputGroup.Textarea
      class="max-h-48 min-h-12 px-4 pt-3"
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
      onpaste={handlePaste}
    />
    <InputGroup.Addon class="flex-wrap px-3 pb-3 text-foreground" align="block-end">
      {#each sources as source (source.id)}
        <source.button disabled={disabled || sending} target={images} />
      {/each}
      {@render controls?.()}
      <div class="ms-auto flex items-center gap-2">
        {@render status?.()}
        {#if replying}
          <Tooltip.Root>
            <Tooltip.Trigger>
              {#snippet child({ props })}
                <Button
                  {...props}
                  class="rounded-full"
                  aria-label={m.chat_stop()}
                  size="icon"
                  type="button"
                  onclick={() => void onStop?.()}
                >
                  <SquareIcon class="fill-current" />
                </Button>
              {/snippet}
            </Tooltip.Trigger>
            <Tooltip.Content>{m.chat_stop()}</Tooltip.Content>
          </Tooltip.Root>
        {:else}
          <Tooltip.Root>
            <Tooltip.Trigger disabled={!canSend} type="submit">
              {#snippet child({ props })}
                <Button {...props} class="rounded-full" aria-label={m.chat_send()} size="icon">
                  <ArrowUpIcon />
                </Button>
              {/snippet}
            </Tooltip.Trigger>
            <Tooltip.Content>{m.chat_send()}</Tooltip.Content>
          </Tooltip.Root>
        {/if}
      </div>
    </InputGroup.Addon>
  </InputGroup.Root>
  <AttachedImages {images} />
  {#if mentioned.length > 0}
    <MentionChips mentions={mentioned} />
  {/if}
  {#if failure !== null}
    <p class="text-sm text-destructive" role="alert">
      {chatFailureMessage(
        failure,
        (provider) => slots.mentionKinds.find(provider),
        (source) => slots.imageSources.find(source),
      )}
    </p>
  {/if}
  <p id="composer-hint" class="text-center text-xs text-muted-foreground">{m.chat_composer_hint()}</p>
</form>
