<script lang="ts">
  import ArrowUpIcon from '@lucide/svelte/icons/arrow-up';
  import SquareIcon from '@lucide/svelte/icons/square';
  import { onMount, type Snippet, untrack } from 'svelte';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as InputGroup from '#lib/components/ui/input-group/index.js';
  import * as Tooltip from '#lib/components/ui/tooltip/index.js';
  import type { ImageRef, MentionPlace, MentionRef } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { messageBoxes, NEW_CONVERSATION_BOX } from '#lib/state/message-boxes.svelte.js';
  import { MessageImages } from '#lib/state/message-images.svelte.js';
  import { type MentionChoice, type MentionOption, MessageMentions } from '#lib/state/message-mentions.svelte.js';
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
  // when sending fails. The box of a conversation takes mentions from views
  // outside it while it shows, such as a terminal's (ADR-0054, decision 4).
  // The unsent text, mentions and images stay for the conversation when the
  // box goes, and come back with its next box (ADR-0068, decision 7).
  interface Props {
    /** What the conversation has attached, below the box, such as notebooks (ADR-0046, decision 6). */
    attached?: Snippet;
    /**
     * The controls of the row after the image sources' buttons, such as the
     * conversation's full access (ADR-0042, decision 4).
     */
    controls?: Snippet;
    /** Whether the box takes no message, such as without a provider profile. */
    disabled?: boolean;
    /** Where the message is written, which decides what it can mention (ADR-0032). */
    mentionPlace: MentionPlace;
    /** Whether the box is a new conversation's draft's, which takes a plan to run too (ADR-0060, decision 10). */
    newConversation?: boolean;
    /** Sends a message with what it mentions and its images; resolves to why it failed, or `null`. */
    onSend: (text: string, mentions: MentionRef[], images: ImageRef[]) => Promise<ChatFailure | null>;
    /** Stops the reply that is being written, when there can be one. */
    onStop?: () => Promise<void>;
    /** Whether a reply is being written, which the button then stops. */
    replying?: boolean;
    /** What the row shows before Send, such as the conversation's context meter. */
    status?: Snippet;
  }

  let {
    attached,
    controls,
    disabled = false,
    mentionPlace,
    newConversation = false,
    onSend,
    onStop,
    replying = false,
    status,
  }: Props = $props();

  const slots = getShellSlots();
  const uid = $props.id();
  const listId = `${uid}-mentions`;

  /** The conversation whose message the box writes, or the new conversation's box. */
  const target = untrack(() =>
    mentionPlace.kind === 'conversation' ? mentionPlace.conversationId : NEW_CONVERSATION_BOX,
  );
  const unsent = messageBoxes.takeUnsent(target);

  let box: HTMLTextAreaElement | null = $state(null);
  let text = $state(unsent?.text ?? '');
  let sending = $state(false);
  let failure = $state<ChatFailure | null>(null);
  /** How many drags with files are over the box, which counts the entries into its children. */
  let dragDepth = $state(0);

  const mentions = new MessageMentions(
    () => mentionPlace,
    (provider) => slots.mentionKinds.find(provider),
  );
  mentions.restore(unsent?.mentions ?? []);
  const images = unsent?.images ?? new MessageImages();
  /** The sources of images, in the order of their buttons. */
  const sources = $derived([...slots.imageSources.entries].sort((left, right) => left.order - right.order));

  const hasText = $derived(text.trim() !== '');
  const hasContent = $derived(hasText || images.hasReady);
  const canSend = $derived(hasContent && !images.preparing && !disabled && !sending && !replying);
  const takesFiles = $derived(sources.length > 0 && !disabled);
  const mentioned = $derived(mentions.inText(text));
  const listShowsEntries = $derived(mentions.open && mentions.options.length > 0);
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

  /** Puts a mention that a view outside the box adds at the end of the text, and moves the focus to it. */
  function mentionFromOutside(choice: MentionChoice): 'added' | 'full' {
    const inserted = mentions.append(text, choice);
    if (inserted === null) {
      return 'full';
    }
    text = inserted.text;
    if (box !== null) {
      box.value = inserted.text;
      box.focus();
      box.setSelectionRange(inserted.caret, inserted.caret);
    }
    return 'added';
  }

  /** Puts a text and a mention that a view outside the box adds after the owner's own text. */
  function fillFromOutside(added: string, choice: MentionChoice): void {
    text = text === '' ? added : `${text} ${added}`;
    if (box !== null) {
      box.value = text;
    }
    mentionFromOutside(choice);
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

  /** Takes an option of the list: inserts an entry, or shows the rest of a kind's entries. */
  function take(option: MentionOption): void {
    if (option.type === 'entry') {
      choose(option.choice);
    } else {
      mentions.expand(option.provider);
    }
  }

  async function takeWhenAnswered(): Promise<void> {
    const option = await mentions.whenAnswered();
    if (option !== undefined) {
      take(option);
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
    // An Enter or Tab before the answers for what the owner typed chooses
    // from those answers, never from earlier ones.
    const takesOption = !event.shiftKey && (event.key === 'Enter' || event.key === 'Tab');
    if (takesOption && !mentions.settled) {
      void takeWhenAnswered();
      return true;
    }
    const option = mentions.active;
    if (option === undefined) {
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
        take(option);
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

  // The box of a conversation takes mentions from views outside it while it
  // shows (ADR-0054, decision 4), and the box of a new conversation takes a
  // plan to run (ADR-0060, decision 10). `untrack` keeps a box from registering
  // again as the owner types, because a filled box reads its own text.
  $effect(() => {
    const takesPlans = newConversation && target !== NEW_CONVERSATION_BOX;
    return untrack(() => {
      const entry = { fill: fillFromOutside, mention: mentionFromOutside };
      const unregister = messageBoxes.register(target, entry);
      const unregisterNew = takesPlans ? messageBoxes.register(NEW_CONVERSATION_BOX, entry) : undefined;
      return () => {
        unregister();
        unregisterNew?.();
      };
    });
  });

  onMount(() => () => {
    messageBoxes.keepUnsent(target, { images, mentions: mentions.inText(text), text });
  });
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
    <MentionPicker {listId} {mentions} {optionId} onTake={take} />
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
  {@render attached?.()}
  {#if failure !== null}
    <p class="text-sm text-destructive" role="alert">
      {chatFailureMessage(
        failure,
        (provider) => slots.mentionKinds.find(provider),
        (source) => slots.imageSources.find(source),
        (provider) => slots.contextKinds.find(provider),
      )}
    </p>
  {/if}
  <p id="composer-hint" class="text-center text-xs text-muted-foreground">{m.chat_composer_hint()}</p>
</form>
