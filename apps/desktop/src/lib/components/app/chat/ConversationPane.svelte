<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import MessageSquareOffIcon from '@lucide/svelte/icons/message-square-off';
  import MessagesSquareIcon from '@lucide/svelte/icons/messages-square';
  import { onMount, untrack } from 'svelte';

  import ConversationActionsMenu from '#lib/components/app/shell/ConversationActionsMenu.svelte';
  import ConversationControlOutlet from '#lib/components/app/shell/ConversationControlOutlet.svelte';
  import PluginOffState from '#lib/components/app/shell/PluginOffState.svelte';
  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Empty from '#lib/components/ui/empty/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { type Attachment, commands, type ImageRef, type MentionRef } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';
  import { sameKey } from '#lib/state/conversation-draft.svelte.js';
  import { conversationList } from '#lib/state/conversation-list.svelte.js';
  import { OpenConversation } from '#lib/state/conversation.svelte.js';
  import { pluginStates } from '#lib/state/plugin-states.svelte.js';
  import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import { call } from '#lib/utils/calls.js';
  import { chatFailureMessage, pluginOffOf } from '#lib/utils/chat-errors.js';
  import { nameShownDraft, startNewConversation } from '#lib/utils/chat-urls.js';
  import { hasActiveRun } from '#lib/utils/conversation-reducer.js';

  import AttachmentChips from './AttachmentChips.svelte';
  import ChatGptPlanIndicator from './ChatGptPlanIndicator.svelte';
  import Composer from './Composer.svelte';
  import ContextMeter from './ContextMeter.svelte';
  import NoProfileNotice from './NoProfileNotice.svelte';
  import ProfileMenu from './ProfileMenu.svelte';
  import ReadOnlyFooter from './ReadOnlyFooter.svelte';
  import Transcript from './Transcript.svelte';

  // One conversation: its toolbar, its transcript and the message box
  // (docs/features/chat/conversation-view.md). The toolbar holds the profile
  // menu with the ChatGPT plan's indicator beside it while the profile uses
  // the plan, the controls of the toolbar and the conversation's menu; the
  // message box holds the controls of its place and the context meter
  // (ADR-0042). The view creates a new pane for each conversation, so the ID
  // never changes during a pane's life. A child agent's conversation shows
  // read-only: its parent gives it work, and the core refuses the owner's
  // commands on it but Stop. It has no message box, so its toolbar keeps the
  // context meter. A new conversation is the draft of its place (ADR-0068):
  // its pane shows a hint until its first message, after which the address
  // names it, and every control works on it from the start.
  interface Props {
    /** The conversation. */
    readonly id: string;
    /** Called once the conversation is gone, such as a draft that the owner deleted. */
    readonly onGone?: () => void;
  }

  let { id, onGone }: Props = $props();

  const conversation = new OpenConversation(untrack(() => id));

  /** Whether an attachment is being detached, and why the last detach failed (ADR-0046). */
  let detaching = $state(false);
  let detachFailure = $state<ChatFailure | null>(null);

  const transcript = $derived(conversation.transcript);
  const summary = $derived(transcript.conversation);
  const isChild = $derived(summary !== null && summary.task !== null);
  /** The kind of task of the plugin's executor whose conversation this is, or `null` for a child agent's. */
  const executorKind = $derived(summary?.task?.executor?.kind ?? null);
  const replying = $derived(hasActiveRun(transcript));
  const noProfile = $derived(providerProfiles.status === 'ready' && providerProfiles.profiles.length === 0);
  const compacting = $derived(chatActions.compacting.includes(id));
  const compactionFailure = $derived(chatActions.compactionFailures[id] ?? null);

  /** The plugin of a task conversation that cannot be followed while the plugin is off, or `null`. */
  const offPlugin = $derived(pluginOffOf(conversation.failure));
  const pluginBackOn = $derived(offPlugin !== null && pluginStates.isOn(offPlugin));

  /** What the conversation has attached (ADR-0046); a summary of a version before them has none. */
  const attachments = $derived(summary?.attachments ?? []);

  /** Whether it is a draft, which shows the hint of a new conversation until its first message (ADR-0068). */
  const isDraft = $derived(summary?.draft ?? false);
  const hasTurns = $derived(transcript.turns.length > 0);
  const showsHint = $derived(isDraft && !hasTurns);
  const hintTitle = $derived.by(() => {
    const home = summary?.home ?? null;
    if (home !== null) {
      const label = attachments.find((held) => sameKey(held, home))?.label;
      if (label !== undefined) {
        return m.chat_new_title_in_home({ label });
      }
    }
    const projectId = summary?.projectId ?? null;
    const project = projectId === null ? undefined : conversationList.findProject(projectId);
    return project === undefined ? m.chat_new_title() : m.chat_new_title_in_project({ project: project.name });
  });

  async function send(text: string, mentions: MentionRef[], images: ImageRef[]): Promise<ChatFailure | null> {
    const wasDraft = isDraft;
    const outcome = await call(() => commands.sendMessage({ conversationId: id, images, mentions, text }));
    if (!outcome.ok) {
      return outcome.failure;
    }
    // The first message makes the draft a conversation of the list, which
    // the address then names, so the next new conversation opens a new draft.
    if (wasDraft) {
      void nameShownDraft(id);
    }
    return null;
  }

  async function detach(attachment: Attachment): Promise<void> {
    detaching = true;
    const { key, provider } = attachment;
    const outcome = await call(() => commands.detachContext({ attachment: { key, provider }, conversationId: id }));
    detaching = false;
    detachFailure = outcome.ok ? null : outcome.failure;
  }

  async function stop(): Promise<void> {
    await chatActions.stop(id);
  }

  $effect(() => {
    if (conversation.status === 'deleted') {
      untrack(() => onGone?.());
    }
  });

  // A task conversation of a plugin that is off follows again once the
  // plugin is on (ADR-0030, decision 12).
  $effect(() => {
    if (pluginBackOn) {
      untrack(() => {
        conversation.retry();
      });
    }
  });

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
    {#if offPlugin === null}
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
      <PluginOffState plugin={offPlugin} />
    {/if}
  {:else}
    {#if summary !== null}
      <div class="flex items-center gap-2 border-b px-3 py-1.5">
        {#if !isChild}
          <ProfileMenu
            profileId={summary.profileId}
            onChoose={(profileId: string | null) => chatActions.setProfile(id, profileId)}
          />
          <ChatGptPlanIndicator profileId={summary.profileId} />
        {/if}
        <div class="ms-auto flex items-center gap-1">
          <ConversationControlOutlet
            backgroundCommands={transcript.backgroundCommands}
            conversation={summary}
            turns={transcript.turns}
          />
          {#if isChild}
            <ContextMeter checkpoint={transcript.checkpoint} context={transcript.context} conversationId={id} />
          {/if}
          <ConversationActionsMenu
            backgroundCommands={transcript.backgroundCommands}
            conversation={summary}
            turns={transcript.turns}
          />
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
    {#if showsHint}
      <div class="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 overflow-y-auto p-6">
        <Empty.Root>
          <Empty.Header>
            <Empty.Media variant="icon">
              <MessagesSquareIcon />
            </Empty.Media>
            <Empty.Title>{hintTitle}</Empty.Title>
            <Empty.Description>{m.chat_new_description()}</Empty.Description>
          </Empty.Header>
        </Empty.Root>
      </div>
    {:else}
      <Transcript {conversation} readOnly={isChild} />
    {/if}
  {/if}
  {#if isChild}
    <ReadOnlyFooter
      conversationId={id}
      kind={executorKind}
      owner={executorKind === null ? 'child' : 'task'}
      {replying}
    />
  {:else}
    {#if noProfile}
      <div class="mx-auto w-full max-w-3xl px-4 pb-2">
        <NoProfileNotice />
      </div>
    {/if}
    <Composer
      disabled={noProfile || conversation.status !== 'ready'}
      mentionPlace={{ conversationId: id, kind: 'conversation' }}
      newConversation={isDraft}
      {replying}
      onSend={send}
      onStop={stop}
    >
      {#snippet controls()}
        {#if summary !== null}
          <ConversationControlOutlet
            backgroundCommands={transcript.backgroundCommands}
            conversation={summary}
            place="message-box"
            turns={transcript.turns}
          />
        {/if}
      {/snippet}
      {#snippet status()}
        {#if summary !== null}
          <ContextMeter checkpoint={transcript.checkpoint} context={transcript.context} conversationId={id} />
        {/if}
      {/snippet}
      {#snippet attached()}
        {#if attachments.length > 0}
          <AttachmentChips
            {attachments}
            disabled={detaching}
            home={summary?.home ?? null}
            ondetach={(attachment: Attachment) => void detach(attachment)}
          />
        {/if}
        {#if detachFailure !== null}
          <p class="text-sm text-destructive" role="alert">{chatFailureMessage(detachFailure)}</p>
        {/if}
      {/snippet}
    </Composer>
  {/if}
</div>
