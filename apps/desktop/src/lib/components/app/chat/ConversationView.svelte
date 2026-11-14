<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import { onMount, untrack } from 'svelte';

  import { page } from '$app/state';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { type AttachmentRef, commands } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { conversationDraft, type DraftIntent, sameKey } from '#lib/state/conversation-draft.svelte.js';
  import { conversationList } from '#lib/state/conversation-list.svelte.js';
  import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
  import { shownDraft } from '#lib/state/shown-draft.svelte.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import { call } from '#lib/utils/calls.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';
  import { CONVERSATION_PARAMETER, newConversationProjectId } from '#lib/utils/chat-urls.js';

  import ConversationPane from './ConversationPane.svelte';

  // The chat view (docs/features/chat/conversation-view.md): the conversation
  // that the address names, or the draft of a new one (ADR-0068). A draft
  // belongs to a place, the general conversations, a project or a plugin's
  // home item, and the core keeps one for each, so what the owner prepared in
  // it stays when the owner opens another conversation and comes back. The
  // first message makes it a conversation of the list, and the address then
  // names it without a new pane, so the next new conversation opens a new
  // draft.

  /** A draft that the core opened, with the place it belongs to. */
  interface OpenedDraft {
    readonly id: string;
    readonly place: string;
  }

  let draft = $state<OpenedDraft | null>(null);
  let failure = $state<ChatFailure | null>(null);
  /** Counts the requests, so only the latest one's answer shows. */
  let request = 0;
  /** The place whose draft is being opened, so the effect asks for it once. */
  let opening: string | null = null;
  /** What a request that failed asked for, which its place's next request asks for again. */
  let unapplied: { readonly intent: DraftIntent; readonly place: string } | null = null;

  const conversationId = $derived(page.url.searchParams.get(CONVERSATION_PARAMETER));
  const home = $derived(conversationDraft.home);
  const projectId = $derived(home === null ? newConversationProjectId() : null);
  /** The place whose draft shows, as a key, or `null` while the address names a conversation. */
  const place = $derived(conversationId === null ? placeKey(projectId, home) : null);
  /** The draft of the place that shows, once the core opened it. */
  const draftId = $derived(draftOf(draft, place));
  const shownId = $derived(conversationId ?? draftId);
  /** Why the place's draft could not be opened, while the view shows no conversation. */
  const draftFailure = $derived(place === null ? null : failure);

  function draftOf(opened: OpenedDraft | null, key: string | null): string | null {
    if (opened === null) {
      return null;
    }
    return opened.place === key ? opened.id : null;
  }

  /** What the draft of the place asks for: what a failed request asked for, and what the window asks for now. */
  function intentFor(key: string): DraftIntent {
    const earlier = unapplied?.place === key ? unapplied.intent : null;
    const current = conversationDraft.isFor(projectId) ? conversationDraft.takeIntent() : null;
    const attachments = [...(earlier?.attachments ?? [])];
    for (const attachment of current?.attachments ?? []) {
      if (!attachments.some((held) => sameKey(held, attachment))) {
        attachments.push(attachment);
      }
    }
    return { agentKey: current?.agentKey ?? earlier?.agentKey ?? null, attachments };
  }

  function placeKey(project: string | null, item: AttachmentRef | null): string {
    return JSON.stringify([project, item?.provider ?? null, item?.key ?? null]);
  }

  /** Opens the draft of a place, and gives it what the window asked for. */
  async function openDraft(key: string): Promise<void> {
    request += 1;
    const current = request;
    opening = key;
    failure = null;
    const intent = intentFor(key);
    const outcome = await call(() =>
      commands.openDraftConversation({
        agentKey: intent.agentKey,
        attachments: intent.attachments,
        home,
        projectId,
      }),
    );
    if (current !== request) {
      return;
    }
    if (!outcome.ok) {
      // Try again asks for it again, so the draft gets what the window asked for.
      unapplied = { intent, place: key };
      opening = null;
      failure = outcome.failure;
      return;
    }
    unapplied = null;
    draft = { id: outcome.data, place: key };
  }

  function retry(): void {
    if (place !== null) {
      void openDraft(place);
    }
  }

  /** A shown draft that is gone, such as one the owner deleted, gives way to a new one. */
  function draftGone(id: string): void {
    if (draft?.id === id) {
      draft = null;
      retry();
    }
  }

  // Opens the place's draft when the view shows a new conversation, and
  // again when the window asks for something more of it, such as a plan's
  // agent. Opening a conversation lets go of the draft, which opens again on
  // the next new conversation, and then may be a new one.
  $effect(() => {
    const key = place;
    const asks = conversationDraft.hasIntent && conversationDraft.isFor(projectId);
    // Opening takes the intent, which changes `hasIntent` and runs this
    // again; the place being opened keeps it from asking a second time.
    untrack(() => {
      if (key === null) {
        opening = null;
        draft = null;
        failure = null;
        return;
      }
      const isOpenOrOpening = draft?.place === key || opening === key;
      if (isOpenOrOpening && !asks) {
        return;
      }
      void openDraft(key);
    });
  });

  $effect(() => {
    shownDraft.id = conversationId === null ? draftId : null;
  });

  onMount(() => {
    conversationList.start();
    void providerProfiles.ensureLoaded();
    return () => {
      shownDraft.id = null;
    };
  });
</script>

{#if shownId !== null}
  {#key shownId}
    <ConversationPane id={shownId} onGone={() => draftGone(shownId)} />
  {/key}
{:else if draftFailure !== null}
  <div class="mx-auto w-full max-w-xl p-6">
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.chat_conversation_failed_title()}</Alert.Title>
      <Alert.Description>{chatFailureMessage(draftFailure)}</Alert.Description>
      <Alert.Action>
        <Button size="sm" variant="outline" onclick={retry}>{m.chat_try_again()}</Button>
      </Alert.Action>
    </Alert.Root>
  </div>
{:else}
  <div class="mx-auto flex w-full max-w-3xl flex-1 flex-col gap-6 px-4 py-6" aria-busy="true">
    <span class="sr-only" role="status">{m.chat_loading()}</span>
    <Skeleton class="h-24 w-full" />
  </div>
{/if}
