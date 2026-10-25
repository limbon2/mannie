<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import { Skeleton } from '@mannie/plugin-sdk/components/skeleton';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import FileTextIcon from '@mannie/plugin-sdk/icons/file-text';
  import { call } from '@mannie/plugin-sdk/ipc';
  import { onMount } from 'svelte';

  import { commands, type PassagePlace, type SectionPlace, type SourceText } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { MAX_SECTIONS_PER_READ } from '../generated/plugin.js';
  import type { ViewerRequest } from '../state/source-viewer.svelte.js';
  import type { NotebooksFailure } from '../types/notebooks.js';
  import { sectionPlaceLabel, sourceDetails } from '../utils/notebooks-labels.js';
  import { failureCode, notebooksFailureMessage } from '../utils/notebooks-messages.js';
  import { textSegments, windowStart } from '../utils/notebooks-text.js';

  // A source's text in the viewer (plugins/notebooks/docs/README.md, sources
  // and answers): its pages or sections, a window of them at a time, each
  // under its place, and the passage that a citation opened, scrolled to and
  // marked. The text is content, shown as plain text (T-65). The dialog shows
  // a new viewer for each request, so a request is read once.
  /** Sections of the source's text, from the first of a window. */
  interface Shown {
    readonly first: number;
    readonly text: SourceText;
  }

  interface Props {
    /** Closes the viewer, such as when its source is gone. */
    onclose: () => void;
    /** What to show. */
    request: ViewerRequest;
  }

  let { onclose, request }: Props = $props();

  const id = $props.id();

  let shown = $state<Shown | null>(null);
  let failure = $state<NotebooksFailure | null>(null);
  /** The passage to mark, once it was found. */
  let passage = $state<PassagePlace | null>(null);
  /** Whether the passage that a citation asked for is gone from the source. */
  let passageGone = $state(false);
  /** Counts reads, so that the answer of an older one changes nothing. */
  let reads = 0;

  const isGone = $derived(failure !== null && failureCode(failure) === 'source_not_found');
  const last = $derived(shown === null ? 0 : shown.first + shown.text.sections.length);
  const earlier = $derived(shown === null ? 0 : Math.max(0, shown.first - MAX_SECTIONS_PER_READ));
  const hasEarlier = $derived(shown !== null && shown.first > 0);
  const hasLater = $derived(shown !== null && last < shown.text.total);
  const range = $derived(shown === null ? null : rangeText(shown));

  /** The page of a section's place, if it is a page. */
  function pageOf(place: SectionPlace | undefined): number | null {
    return place?.kind === 'page' ? place.number : null;
  }

  /** The first and last pages of a window of a PDF's sections, or `null` when one of them is no page. */
  function pagesOf(window: Shown): { first: number; last: number } | null {
    const { sections, source } = window.text;
    if (source.kind !== 'pdf') {
      return null;
    }
    const first = pageOf(sections[0]?.place);
    const last = pageOf(sections.at(-1)?.place);
    if (first === null) {
      return null;
    }
    return last === null ? null : { first, last };
  }

  /** Which pages or sections a window shows, as the line between its buttons says. */
  function rangeText(window: Shown): string {
    const { sections, source, total } = window.text;
    const pages = pagesOf(window);
    if (pages !== null) {
      return m.notebooks_viewer_pages({
        first: formatCount(pages.first),
        last: formatCount(pages.last),
        total: formatCount(source.pages ?? total),
      });
    }
    return m.notebooks_viewer_sections({
      first: formatCount(window.first + 1),
      last: formatCount(window.first + sections.length),
      total: formatCount(total),
    });
  }

  /** Reads a window of sections. `read` is the number of the read that asks. */
  async function readFrom(first: number, read: number): Promise<void> {
    failure = null;
    shown = null;
    const outcome = await call(() => commands.readSource({ firstSection: first, sourceId: request.sourceId }));
    if (read !== reads) {
      return;
    }
    if (!outcome.ok) {
      failure = outcome.failure;
      return;
    }
    shown = { first, text: outcome.data };
  }

  /** Finds the request's passage, then reads the window that holds it, or the first one. */
  async function show(): Promise<void> {
    reads += 1;
    const read = reads;
    passage = null;
    passageGone = false;
    failure = null;
    shown = null;
    const key = request.key;
    if (key !== null) {
      const located = await call(() => commands.locatePassage({ key }));
      if (read !== reads) {
        return;
      }
      const isHere = located.ok && located.data.sourceId === request.sourceId;
      if (isHere) {
        passage = located.data;
      } else {
        passageGone = true;
      }
    }
    await readFrom(windowStart(passage?.section ?? 0), read);
  }

  /** Reads the window before or after the one shown. */
  function turn(first: number): void {
    reads += 1;
    void readFrom(first, reads);
  }

  /** Brings the marked passage into view once it shows. */
  function reveal(mark: HTMLElement): void {
    mark.scrollIntoView({ block: 'center' });
  }

  onMount(() => {
    void show();
  });
</script>

<div class="flex min-h-0 flex-1 flex-col gap-3" data-notebooks-viewer={request.sourceId}>
  {#if isGone}
    <Empty.Root class="flex-1">
      <Empty.Header>
        <Empty.Media variant="icon">
          <FileTextIcon />
        </Empty.Media>
        <Empty.Title>{m.notebooks_viewer_missing_title()}</Empty.Title>
        <Empty.Description>{m.notebooks_viewer_missing_description()}</Empty.Description>
      </Empty.Header>
      <Empty.Content>
        <Button variant="outline" onclick={onclose}>{m.notebooks_close()}</Button>
      </Empty.Content>
    </Empty.Root>
  {:else if failure !== null}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.notebooks_viewer_failed_title()}</Alert.Title>
      <Alert.Description>{notebooksFailureMessage(failure)}</Alert.Description>
      <Alert.Action>
        <Button size="sm" variant="outline" onclick={() => void show()}>{m.notebooks_try_again()}</Button>
      </Alert.Action>
    </Alert.Root>
  {:else if shown === null}
    <div class="flex flex-col gap-2" aria-busy="true">
      <span class="sr-only" role="status">{m.notebooks_viewer_loading()}</span>
      <Skeleton class="h-4 w-full" />
      <Skeleton class="h-4 w-5/6" />
      <Skeleton class="h-4 w-2/3" />
    </div>
  {:else}
    <p class="text-sm text-muted-foreground">{sourceDetails(shown.text.source)}</p>
    {#if passageGone}
      <p class="text-sm text-muted-foreground" role="status">{m.notebooks_passage_missing()}</p>
    {/if}
    {#if shown.text.total === 0}
      <p class="text-sm text-muted-foreground" data-notebooks-source-empty="">{m.notebooks_viewer_empty()}</p>
    {:else}
      {#if hasEarlier || hasLater}
        <div class="flex flex-wrap items-center gap-2">
          <Button disabled={!hasEarlier} size="sm" variant="outline" onclick={() => turn(earlier)}>
            {m.notebooks_viewer_earlier()}
          </Button>
          <p class="text-sm text-muted-foreground">{range}</p>
          <Button disabled={!hasLater} size="sm" variant="outline" onclick={() => turn(last)}>
            {m.notebooks_viewer_later()}
          </Button>
        </div>
      {/if}
      <!-- Focusable, so that the owner can scroll the text from the keyboard (WCAG 2.1.1). -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div
        class="flex min-h-0 flex-1 flex-col gap-4 overflow-auto rounded-md outline-none focus-visible:ring-2 focus-visible:ring-ring"
        aria-labelledby="{id}-label"
        role="region"
        tabindex="0"
      >
        <span id="{id}-label" class="sr-only">{m.notebooks_viewer_text_label()}</span>
        <div class="mx-auto flex w-full max-w-4xl flex-col gap-4">
          {#each shown.text.sections as section (section.position)}
            {const place = $derived(sectionPlaceLabel(section.place))}
            {const marked = $derived(passage?.section === section.position ? passage : null)}
            <section class="flex flex-col gap-1" data-notebooks-section={section.position}>
              {#if place !== null}
                <h3 class="text-xs font-medium text-muted-foreground" data-content dir="auto">{place}</h3>
              {/if}
              <p class="text-sm/relaxed wrap-anywhere whitespace-pre-wrap" data-content dir="auto">
                {#each textSegments(section.text, marked) as segment, index (index)}
                  {#if segment.marked}
                    <mark class="rounded-sm bg-primary/15 text-foreground" {@attach reveal} data-notebooks-passage="">
                      {segment.text}
                    </mark>
                  {:else}
                    {segment.text}
                  {/if}
                {/each}
              </p>
            </section>
          {/each}
        </div>
      </div>
    {/if}
  {/if}
</div>
