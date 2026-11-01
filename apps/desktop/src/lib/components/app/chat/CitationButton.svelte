<script lang="ts">
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Popover from '#lib/components/ui/popover/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { CitedItem } from '#lib/state/citations.js';
  import type { ContextKind } from '#lib/types/shell-slots.js';
  import { citeKeyParts, markedLines, type MarkedText } from '#lib/utils/citations.js';
  import { locationText } from '#lib/utils/context-kinds.js';

  // A citation of a reply (ADR-0049, decision 4): the label of its cite link,
  // which opens the excerpt as the model read it with a message, with where
  // it comes from. The excerpt is the stored one, so it shows while its
  // plugin is off. A passage that the model read with a tool has no stored
  // excerpt, so the popover says so. The kind of context, while its plugin is
  // on, opens either where it comes from, with the whole key. A key that ends
  // with a line fragment marks those lines of the excerpt (ADR-0054,
  // decision 2).
  interface Props {
    /** The stored excerpt, or `undefined` for one that no message holds. */
    cited: CitedItem | undefined;
    /** The key that the link names. */
    key: string;
    /** The kind of context of the link's provider, while its plugin is on. */
    kind: ContextKind | undefined;
    /** The link's text. */
    label: string;
  }

  let { cited, key, kind, label }: Props = $props();

  let open = $state(false);

  const location = $derived(cited === undefined ? null : locationText(cited.item.location));
  const lines = $derived(citeKeyParts(key).lines);
  const marked = $derived(markedText());

  /** The excerpt split around the lines that the key names, when it names lines of a stored excerpt. */
  function markedText(): MarkedText | null {
    if (cited === undefined) {
      return null;
    }
    if (lines === null) {
      return null;
    }
    return markedLines(cited.item.text, lines);
  }

  /**
   * Scrolls the excerpt to the marked lines when the popover opens. Only the
   * excerpt scrolls, never the transcript behind it, and the excerpt is the
   * mark's offset parent.
   */
  function showMarked(mark: HTMLElement): void {
    const excerpt = mark.parentElement;
    if (excerpt !== null) {
      excerpt.scrollTop = mark.offsetTop;
    }
  }

  function openSource(): void {
    const opener = kind?.open;
    if (opener === undefined) {
      return;
    }
    open = false;
    opener.run(key);
  }
</script>

<Popover.Root bind:open>
  <Popover.Trigger
    class="mx-0.5 inline-flex h-5 max-w-64 items-center rounded-sm bg-muted px-1.5 align-baseline text-xs text-muted-foreground hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
    aria-label={m.chat_citation_label({ label })}
    data-citation={key}
  >
    <span class="truncate" data-content>{label}</span>
  </Popover.Trigger>
  <Popover.Content class="w-96 max-w-[calc(100vw-2rem)]" align="start">
    <Popover.Header>
      <Popover.Title class="wrap-anywhere" data-content>{cited?.item.title ?? label}</Popover.Title>
      {#if location !== null}
        <Popover.Description class="wrap-anywhere" data-content>{location}</Popover.Description>
      {/if}
    </Popover.Header>
    {#if cited === undefined}
      <p class="text-sm text-muted-foreground">{m.chat_citation_read_with_tool()}</p>
    {:else}
      <!-- Focusable, so that a long excerpt scrolls from the keyboard (WCAG 2.1.1). -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <p
        class="relative max-h-64 overflow-y-auto text-sm wrap-anywhere whitespace-pre-wrap"
        aria-label={m.chat_citation_text_label()}
        data-content
        tabindex="0"
      >
        {#if marked === null}
          {cited.item.text}
        {:else}
          {marked.before}<mark class="bg-accent text-accent-foreground" {@attach showMarked} data-cited-lines=""
            >{marked.marked}</mark
          >{marked.after}
        {/if}
      </p>
    {/if}
    {#if kind?.open !== undefined}
      <Button class="self-start" size="sm" variant="outline" onclick={openSource}>{kind.open.label()}</Button>
    {/if}
  </Popover.Content>
</Popover.Root>
