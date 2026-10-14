<script lang="ts">
  import ArrowDownIcon from '@lucide/svelte/icons/arrow-down';
  import BookTextIcon from '@lucide/svelte/icons/book-text';
  import { tick } from 'svelte';

  import PartOutlet from '#lib/components/app/shell/PartOutlet.svelte';
  import { Button } from '#lib/components/ui/button/index.js';
  import { Separator } from '#lib/components/ui/separator/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import type { RunView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';
  import type { OpenConversation } from '#lib/state/conversation.svelte.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';
  import { latestTurn } from '#lib/utils/conversation-reducer.js';

  import RunStatus from './RunStatus.svelte';

  // The turns of a conversation (docs/features/chat/conversation-view.md,
  // layout): it follows a streaming reply while the owner is at the bottom,
  // offers a way back to the newest text otherwise, and loads earlier turns
  // when the owner reaches the top. Screen readers hear when a reply ends;
  // the streamed text itself is not announced.
  let { conversation }: { conversation: OpenConversation } = $props();

  /** How close to the bottom, in pixels, still counts as following. */
  const FOLLOW_DISTANCE = 48;

  /** How close to the top, in pixels, loads earlier turns. */
  const LOAD_DISTANCE = 64;

  let scroller = $state<HTMLDivElement | undefined>();
  let following = $state(true);

  const transcript = $derived(conversation.transcript);
  const latest = $derived(latestTurn(transcript));
  const checkpoint = $derived(transcript.checkpoint);

  function endAnnouncement(run: RunView): string {
    switch (run.state) {
      case 'completed': {
        return m.chat_announce_completed();
      }
      case 'failed': {
        return m.chat_announce_failed();
      }
      case 'interrupted': {
        return m.chat_status_interrupted();
      }
      case 'queued':
      case 'running': {
        return '';
      }
      case 'stopped': {
        return m.chat_status_stopped();
      }
    }
  }

  /** Keeps the transcript at the bottom while the owner follows it and new and growing parts change its size. */
  function followGrowth(content: HTMLElement): () => void {
    const observer = new ResizeObserver(() => {
      if (following) {
        scrollToBottom();
      }
    });
    observer.observe(content);
    return () => {
      observer.disconnect();
    };
  }

  function scrollToBottom(): void {
    if (scroller !== undefined) {
      scroller.scrollTop = scroller.scrollHeight;
    }
  }

  function handleScroll(): void {
    if (scroller === undefined) {
      return;
    }
    const distance = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
    following = distance <= FOLLOW_DISTANCE;
    const nearTop = scroller.scrollTop <= LOAD_DISTANCE;
    const canLoad = transcript.hasEarlierTurns && !conversation.loadingEarlier;
    if (nearTop && canLoad) {
      void loadEarlier();
    }
  }

  /** Whether the checkpoint's marker follows this turn: the last one it covers. */
  function checkpointFollows(position: number): boolean {
    return checkpoint?.coversThroughTurn === position;
  }

  async function loadEarlier(): Promise<void> {
    if (scroller === undefined) {
      return;
    }
    const heightBefore = scroller.scrollHeight;
    await conversation.loadEarlier();
    await tick();
    // The turns above the old first one keep the owner's place.
    scroller.scrollTop += scroller.scrollHeight - heightBefore;
  }

  function showCheckpoint(): void {
    if (checkpoint !== null) {
      void chatActions.showCheckpoint({ checkpointId: checkpoint.id, conversationId: conversation.id });
    }
  }
</script>

<div class="relative min-h-0 flex-1">
  <!-- Focusable, so that the owner can scroll the transcript from the keyboard
       when it holds nothing else that takes the focus (WCAG 2.1.1). -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div
    bind:this={scroller}
    class="h-full overflow-y-auto outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
    aria-label={m.chat_transcript_label()}
    role="region"
    tabindex="0"
    onscroll={handleScroll}
  >
    <div class="mx-auto flex w-full max-w-3xl flex-col gap-6 px-4 py-6" {@attach followGrowth}>
      {#if transcript.hasEarlierTurns}
        <div class="flex flex-col items-center gap-2">
          <Button disabled={conversation.loadingEarlier} size="sm" variant="ghost" onclick={loadEarlier}>
            {#if conversation.loadingEarlier}
              <Spinner aria-label={m.chat_loading_earlier()} data-icon="inline-start" />
            {/if}
            {m.chat_load_earlier()}
          </Button>
          {#if conversation.loadEarlierFailure !== null}
            <p class="text-sm text-destructive">{chatFailureMessage(conversation.loadEarlierFailure)}</p>
          {/if}
        </div>
      {/if}
      {#each transcript.turns as turn (turn.id)}
        <article class="flex flex-col gap-3" aria-label={m.chat_turn_label({ number: String(turn.position + 1) })}>
          {#each turn.parts as part (part.id)}
            <PartOutlet conversationId={conversation.id} latest={turn === latest} {part} />
          {/each}
          {#if turn === latest}
            <RunStatus conversationId={conversation.id} run={turn.run} />
          {/if}
        </article>
        {#if checkpointFollows(turn.position)}
          <div class="flex items-center gap-3 text-xs text-muted-foreground">
            <Separator class="flex-1" />
            <span>{m.chat_checkpoint_marker()}</span>
            <Button size="sm" variant="ghost" onclick={showCheckpoint}>
              <BookTextIcon data-icon="inline-start" />
              {m.chat_checkpoint_show()}
            </Button>
            <Separator class="flex-1" />
          </div>
        {/if}
      {/each}
    </div>
  </div>
  <!-- A new element for each ended reply, so that the same text is announced again. -->
  <p class="sr-only" role="status">
    {#if conversation.endedRun !== null}
      {#key conversation.endedRun.id}
        <span>{endAnnouncement(conversation.endedRun)}</span>
      {/key}
    {/if}
  </p>
  {#if !following}
    <div class="pointer-events-none absolute inset-x-0 bottom-3 flex justify-center">
      <Button class="pointer-events-auto shadow-sm" size="sm" variant="secondary" onclick={scrollToBottom}>
        <ArrowDownIcon data-icon="inline-start" />
        {m.chat_jump_to_newest()}
      </Button>
    </div>
  {/if}
</div>
