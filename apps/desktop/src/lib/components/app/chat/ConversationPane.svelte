<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import MessageSquareOffIcon from '@lucide/svelte/icons/message-square-off';
  import { onMount, untrack } from 'svelte';

  import ConversationControlOutlet from '#lib/components/app/shell/ConversationControlOutlet.svelte';
  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Empty from '#lib/components/ui/empty/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { commands } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';
  import { OpenConversation } from '#lib/state/conversation.svelte.js';
  import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
  import { takeComposerFocus } from '#lib/state/started-conversation.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import { call } from '#lib/utils/calls.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';
  import { startNewConversation } from '#lib/utils/chat-urls.js';
  import { hasActiveRun } from '#lib/utils/conversation-reducer.js';

  import Composer from './Composer.svelte';
  import ContextMeter from './ContextMeter.svelte';
  import ConversationMenu from './ConversationMenu.svelte';
  import NoProfileNotice from './NoProfileNotice.svelte';
  import ProfileMenu from './ProfileMenu.svelte';
  import ReadOnlyFooter from './ReadOnlyFooter.svelte';
  import Transcript from './Transcript.svelte';

  // One conversation: its toolbar, its transcript and the message box
  // (docs/features/chat/conversation-view.md). The view creates a new pane for
  // each conversation, so the ID never changes during a pane's life. A child
  // agent's conversation shows read-only: its parent gives it work, and the
  // core refuses the owner's commands on it but Stop.
  let { id }: { id: string } = $props();

  const conversation = new OpenConversation(untrack(() => id));
  /** Whether the message box takes the focus, because the first message of a new conversation opened this pane. */
  const takesFocus = takeComposerFocus(untrack(() => id));

  const transcript = $derived(conversation.transcript);
  const summary = $derived(transcript.conversation);
  const isChild = $derived(summary !== null && summary.task !== null);
  /** Whether the conversation belongs to a task on a worker rather than to a child agent. */
  const onWorker = $derived(summary?.task?.worker !== null && summary?.task?.worker !== undefined);
  const replying = $derived(hasActiveRun(transcript));
  const noProfile = $derived(providerProfiles.status === 'ready' && providerProfiles.profiles.length === 0);
  const compacting = $derived(chatActions.compacting.includes(id));
  const compactionFailure = $derived(chatActions.compactionFailures[id] ?? null);

  async function send(text: string): Promise<ChatFailure | null> {
    const outcome = await call(() => commands.sendMessage({ conversationId: id, text }));
    return outcome.ok ? null : outcome.failure;
  }

  async function stop(): Promise<void> {
    await chatActions.stop(id);
  }

  onMount(() => {
    conversation.open();
    return () => {
      conversation.close();
    };
  });
</script>

<div class="flex min-h-0 flex-1 flex-col">
  {#if conversation.status === 'loading'}
    <div class="mx-auto flex w-full max-w-3xl flex-1 flex-col gap-6 px-4 py-6" aria-busy="true">
      <span class="sr-only" role="status">{m.chat_loading()}</span>
      <Skeleton class="ms-auto h-10 w-2/3" />
      <Skeleton class="h-24 w-full" />
      <Skeleton class="ms-auto h-10 w-1/2" />
    </div>
  {:else if conversation.status === 'deleted'}
    <Empty.Root class="flex-1">
      <Empty.Header>
        <Empty.Media variant="icon">
          <MessageSquareOffIcon />
        </Empty.Media>
        <Empty.Title>{m.chat_conversation_gone_title()}</Empty.Title>
        <Empty.Description>{m.chat_conversation_gone_description()}</Empty.Description>
      </Empty.Header>
      <Empty.Content>
        <Button onclick={() => void startNewConversation()}>{m.chat_new_conversation()}</Button>
      </Empty.Content>
    </Empty.Root>
  {:else if conversation.status === 'error'}
    <div class="mx-auto w-full max-w-xl p-6">
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.chat_conversation_failed_title()}</Alert.Title>
        <Alert.Description>
          {conversation.failure === null ? '' : chatFailureMessage(conversation.failure)}
        </Alert.Description>
        <Alert.Action>
          <Button size="sm" variant="outline" onclick={() => conversation.retry()}>{m.chat_try_again()}</Button>
        </Alert.Action>
      </Alert.Root>
    </div>
  {:else}
    {#if summary !== null}
      <div class="flex items-center gap-2 border-b px-3 py-1.5">
        {#if !isChild}
          <ProfileMenu conversation={summary} />
        {/if}
        <div class="ms-auto flex items-center gap-1">
          <ConversationControlOutlet conversation={summary} turns={transcript.turns} />
          <ContextMeter checkpoint={transcript.checkpoint} context={transcript.context} conversationId={id} />
          {#if !isChild}
            <ConversationMenu conversation={summary} />
          {/if}
        </div>
      </div>
    {/if}
    {#if compacting}
      <p class="flex items-center gap-2 border-b px-4 py-2 text-sm text-muted-foreground" role="status">
        <Spinner aria-label={m.chat_compacting()} />
        <span>{m.chat_compacting()}</span>
      </p>
    {/if}
    {#if compactionFailure !== null}
      <div class="border-b px-4 py-2">
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Description>{chatFailureMessage(compactionFailure)}</Alert.Description>
          <Alert.Action>
            <Button size="sm" variant="outline" onclick={() => chatActions.dismissCompactionFailure(id)}>
              {m.chat_dismiss()}
            </Button>
          </Alert.Action>
        </Alert.Root>
      </div>
    {/if}
    {#if conversation.status === 'ended'}
      <p class="border-b px-4 py-2 text-sm text-muted-foreground">{m.chat_conversation_ended()}</p>
    {/if}
    <Transcript {conversation} readOnly={isChild} />
  {/if}
  {#if isChild}
    <ReadOnlyFooter conversationId={id} owner={onWorker ? 'worker' : 'child'} {replying} />
  {:else}
    {#if noProfile}
      <div class="mx-auto w-full max-w-3xl px-4 pb-2">
        <NoProfileNotice />
      </div>
    {/if}
    <Composer
      disabled={noProfile || conversation.status !== 'ready'}
      {replying}
      takeFocus={takesFocus}
      onSend={send}
      onStop={stop}
    />
  {/if}
</div>
