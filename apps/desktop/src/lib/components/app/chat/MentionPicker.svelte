<script lang="ts">
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
  import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
  import type { Component } from 'svelte';

  import * as Item from '#lib/components/ui/item/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { MentionChoice, MentionOption, MessageMentions } from '#lib/state/message-mentions.svelte.js';
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
  // so a click on an option must not take the focus either. Each kind shows
  // as soon as it answers, and a kind that hides entries ends with an option
  // that shows them.
  interface Props {
    /** The ID of the listbox, which the message box controls. */
    listId: string;
    /** The mentions of the message box, whose list this shows. */
    mentions: MessageMentions;
    /** Takes an option: puts an entry's token into the message, or shows the rest of a kind's entries. */
    onTake: (option: MentionOption) => void;
    /** The ID of the option at an index among every option, which the message box points at. */
    optionId: (index: number) => string;
  }

  /** An option with its index among every option and its words. */
  type ShownOption =
    | {
        readonly choice: MentionChoice;
        readonly Icon: Component;
        readonly index: number;
        readonly key: string;
        readonly option: MentionOption;
        readonly type: 'entry';
        readonly variantName: string;
        readonly warning: string | null;
      }
    | {
        readonly hidden: number;
        readonly index: number;
        readonly key: string;
        readonly option: MentionOption;
        readonly type: 'more';
      };

  let { listId, mentions, onTake, optionId }: Props = $props();

  const slots = getShellSlots();

  let panel = $state<HTMLDivElement | null>(null);

  /** The groups that the list shows, each option with its index among every option and its words. */
  const groups = $derived.by(() => {
    let index = 0;
    return mentions.shown.map(({ group, options, whole }) => {
      const kind = slots.mentionKinds.find(group.provider);
      const shownOptions = options.map((option): ShownOption => {
        const at = index;
        index += 1;
        if (option.type === 'more') {
          return { hidden: option.hidden, index: at, key: 'more', option, type: 'more' };
        }
        const { entry } = option.choice;
        return {
          choice: option.choice,
          Icon: mentionIcon(kind, entry.variant),
          index: at,
          key: `entry:${entry.key}`,
          option,
          type: 'entry',
          variantName: mentionVariantName(kind, entry.variant),
          warning: entry.warning === null ? null : mentionWarning(kind, entry.warning),
        };
      });
      const failure = group.failure === null ? null : mentionRefusal(kind, group.failure);
      return {
        failure,
        heading: mentionGroupHeading(kind),
        // A group that hides entries offers the rest itself.
        incomplete: group.incomplete && whole,
        options: shownOptions,
        provider: group.provider,
      };
    });
  });
  const failureText = $derived(listFailure());
  const count = $derived(mentions.options.filter((option) => option.type === 'entry').length);

  /** What a screen reader hears when the list changes. */
  const status = $derived.by(() => {
    if (mentions.status === 'loading') {
      return m.chat_mention_loading();
    }
    if (mentions.status !== 'ready') {
      return '';
    }
    if (mentions.offersNothing) {
      return m.chat_mention_nothing_here();
    }
    if (count > 0) {
      return m.chat_mention_count({ count });
    }
    return mentions.looking ? m.chat_mention_loading() : m.chat_mention_no_matches();
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
  class="absolute inset-x-3 bottom-full z-20 mb-1 flex max-h-[min(24rem,50vh)] flex-col gap-1 overflow-hidden rounded-md border bg-popover p-1 text-popover-foreground shadow-md"
  data-mention-list
  onwheel={scroll}
>
  {#if failureText !== null}
    <p class="px-2 py-1.5 text-sm text-destructive">{failureText}</p>
  {:else if mentions.offersNothing}
    <p class="px-2 py-1.5 text-sm text-muted-foreground">{m.chat_mention_nothing_here()}</p>
  {:else}
    {#if count > 0}
      <div id={listId} aria-label={m.chat_mention_list_label()} role="listbox">
        {#each groups as shownGroup (shownGroup.provider)}
          {#if shownGroup.options.length > 0}
            <div aria-label={shownGroup.heading} role="group">
              <div class="px-2 pt-1.5 pb-1 text-xs font-medium text-muted-foreground" aria-hidden="true">
                {shownGroup.heading}
              </div>
              {#each shownGroup.options as shown (shown.key)}
                <Item.Root
                  id={optionId(shown.index)}
                  class="cursor-default data-[highlighted=true]:bg-accent data-[highlighted=true]:text-accent-foreground"
                  aria-selected={shown.index === mentions.highlighted}
                  data-highlighted={shown.index === mentions.highlighted}
                  data-mention-option
                  role="option"
                  size="xs"
                  onclick={() => onTake(shown.option)}
                  onpointerdown={keepFocus}
                  onpointermove={() => mentions.highlight(shown.index)}
                >
                  {#if shown.type === 'entry'}
                    <shown.Icon class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                    <span class="sr-only">{shown.variantName}</span>
                    <span class="min-w-0 flex-1 truncate font-mono text-xs" data-content
                      >{shown.choice.entry.label}</span
                    >
                    {#if shown.warning !== null}
                      <span class="flex shrink-0 items-center gap-1 text-xs text-destructive" data-mention-warning>
                        <TriangleAlertIcon class="size-3.5" aria-hidden="true" />
                        {shown.warning}
                      </span>
                    {/if}
                  {:else}
                    <ChevronDownIcon class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                    <span class="min-w-0 flex-1 truncate text-xs text-muted-foreground" data-mention-more>
                      {m.chat_mention_show_more({ count: shown.hidden })}
                    </span>
                  {/if}
                </Item.Root>
              {/each}
            </div>
          {/if}
        {/each}
      </div>
    {:else if !mentions.looking}
      <p class="px-2 py-1.5 text-sm text-muted-foreground">{m.chat_mention_no_matches()}</p>
    {/if}
    <!-- Until every kind answered, below the kinds that did. -->
    {#if mentions.looking}
      <p class="flex items-center gap-2 px-2 py-1.5 text-sm text-muted-foreground" data-mention-looking>
        <Spinner aria-hidden="true" role="presentation" />
        {m.chat_mention_loading()}
      </p>
    {/if}
    {#each groups as shownGroup (shownGroup.provider)}
      {#if shownGroup.failure !== null}
        <p class="px-2 py-1 text-xs text-destructive">
          {m.chat_mention_group_failure({ group: shownGroup.heading, reason: shownGroup.failure })}
        </p>
      {/if}
      {#if shownGroup.incomplete}
        <p class="px-2 py-1 text-xs text-muted-foreground">
          {m.chat_mention_incomplete({ group: shownGroup.heading })}
        </p>
      {/if}
    {/each}
  {/if}
  <p class="sr-only" role="status">{status}</p>
</div>
