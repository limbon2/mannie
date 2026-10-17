<script lang="ts">
  import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';

  import * as Item from '#lib/components/ui/item/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { MentionChoice, MessageMentions } from '#lib/state/message-mentions.svelte.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';
  import {
    mentionGroupHeading,
    mentionIcon,
    mentionRefusal,
    mentionVariantName,
    mentionWarning,
  } from '#lib/utils/mention-kinds.js';

  // The "@" list above the message box (docs/features/chat/mentions.md). The
  // focus stays in the message box, which points at the highlighted option,
  // so a click on an option must not take the focus either.
  interface Props {
    /** The ID of the listbox, which the message box controls. */
    listId: string;
    /** The mentions of the message box, whose list this shows. */
    mentions: MessageMentions;
    /** Puts the token of an entry into the message. */
    onChoose: (choice: MentionChoice) => void;
    /** The ID of the option at an index among every entry, which the message box points at. */
    optionId: (index: number) => string;
  }

  let { listId, mentions, onChoose, optionId }: Props = $props();

  const slots = getShellSlots();

  let panel = $state<HTMLDivElement | null>(null);

  /** The groups that the list shows, each entry with its index among every entry and its words. */
  const groups = $derived.by(() => {
    let index = 0;
    return mentions.groups.map((group) => {
      const kind = slots.mentionKinds.find(group.provider);
      const entries = group.entries.map((entry) => {
        const choice: MentionChoice = { entry, provider: group.provider };
        const warning = entry.warning === null ? null : mentionWarning(kind, entry.warning);
        const shown = {
          choice,
          Icon: mentionIcon(kind, entry.variant),
          index,
          variantName: mentionVariantName(kind, entry.variant),
          warning,
        };
        index += 1;
        return shown;
      });
      const failure = group.failure === null ? null : mentionRefusal(kind, group.failure);
      return { entries, failure, group, heading: mentionGroupHeading(kind) };
    });
  });
  const failureText = $derived(listFailure());
  const count = $derived(mentions.choices.length);

  /** What a screen reader hears when the list changes. */
  const status = $derived.by(() => {
    if (mentions.status === 'loading') {
      return m.chat_mention_loading();
    }
    if (mentions.status !== 'ready') {
      return '';
    }
    if (mentions.groups.length === 0) {
      return m.chat_mention_nothing_here();
    }
    return count === 0 ? m.chat_mention_no_matches() : m.chat_mention_count({ count });
  });

  /** Why the core listed nothing, or `null`. */
  function listFailure(): string | null {
    if (mentions.status !== 'failed') {
      return null;
    }
    return mentions.failure === null ? null : chatFailureMessage(mentions.failure);
  }

  function keepFocus(event: PointerEvent): void {
    event.preventDefault();
  }

  // The list hides its overflow, so it is no region that the keyboard would
  // have to reach: the arrows in the message box move the highlight, which
  // stays in view, and the wheel scrolls the list.
  function scroll(event: WheelEvent): void {
    if (panel !== null) {
      event.preventDefault();
      panel.scrollTop += event.deltaY;
    }
  }

  /** Scrolls the list just enough to show the highlighted option. */
  function keepInView(list: HTMLDivElement | null, option: HTMLElement | null): void {
    if (list === null) {
      return;
    }
    if (option === null) {
      return;
    }
    const top = option.offsetTop;
    const bottom = top + option.offsetHeight;
    if (top < list.scrollTop) {
      list.scrollTop = top;
    } else if (bottom > list.scrollTop + list.clientHeight) {
      list.scrollTop = bottom - list.clientHeight;
    }
  }

  $effect(() => {
    keepInView(panel, document.getElementById(optionId(mentions.highlighted)));
  });
</script>

<div
  bind:this={panel}
  class="absolute inset-x-3 bottom-full z-20 mb-1 flex max-h-72 flex-col gap-1 overflow-hidden rounded-md border bg-popover p-1 text-popover-foreground shadow-md"
  data-mention-list
  onwheel={scroll}
>
  {#if mentions.status === 'loading'}
    <p class="flex items-center gap-2 px-2 py-1.5 text-sm text-muted-foreground">
      <Spinner aria-hidden="true" role="presentation" />
      {m.chat_mention_loading()}
    </p>
  {:else if failureText !== null}
    <p class="px-2 py-1.5 text-sm text-destructive">{failureText}</p>
  {:else if mentions.groups.length === 0}
    <p class="px-2 py-1.5 text-sm text-muted-foreground">{m.chat_mention_nothing_here()}</p>
  {:else}
    {#if count === 0}
      <p class="px-2 py-1.5 text-sm text-muted-foreground">{m.chat_mention_no_matches()}</p>
    {:else}
      <div id={listId} aria-label={m.chat_mention_list_label()} role="listbox">
        {#each groups as shownGroup (shownGroup.group.provider)}
          {#if shownGroup.entries.length > 0}
            <div aria-label={shownGroup.heading} role="group">
              <div class="px-2 pt-1.5 pb-1 text-xs font-medium text-muted-foreground" aria-hidden="true">
                {shownGroup.heading}
              </div>
              {#each shownGroup.entries as shown (shown.choice.entry.key)}
                <Item.Root
                  id={optionId(shown.index)}
                  class="cursor-default data-[highlighted=true]:bg-accent data-[highlighted=true]:text-accent-foreground"
                  aria-selected={shown.index === mentions.highlighted}
                  data-highlighted={shown.index === mentions.highlighted}
                  data-mention-option
                  role="option"
                  size="xs"
                  onclick={() => onChoose(shown.choice)}
                  onpointerdown={keepFocus}
                  onpointermove={() => {
                    mentions.highlighted = shown.index;
                  }}
                >
                  <shown.Icon class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                  <span class="sr-only">{shown.variantName}</span>
                  <span class="min-w-0 flex-1 truncate font-mono text-xs" data-content>{shown.choice.entry.label}</span>
                  {#if shown.warning !== null}
                    <span class="flex shrink-0 items-center gap-1 text-xs text-destructive" data-mention-warning>
                      <TriangleAlertIcon class="size-3.5" aria-hidden="true" />
                      {shown.warning}
                    </span>
                  {/if}
                </Item.Root>
              {/each}
            </div>
          {/if}
        {/each}
      </div>
    {/if}
    {#each groups as shownGroup (shownGroup.group.provider)}
      {#if shownGroup.failure !== null}
        <p class="px-2 py-1 text-xs text-destructive">
          {m.chat_mention_group_failure({ group: shownGroup.heading, reason: shownGroup.failure })}
        </p>
      {/if}
      {#if shownGroup.group.incomplete}
        <p class="px-2 py-1 text-xs text-muted-foreground">
          {m.chat_mention_incomplete({ group: shownGroup.heading })}
        </p>
      {/if}
    {/each}
  {/if}
  <p class="sr-only" role="status">{status}</p>
</div>
