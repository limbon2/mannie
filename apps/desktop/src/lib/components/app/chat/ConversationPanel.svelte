<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import MessageSquareOffIcon from '@lucide/svelte/icons/message-square-off';
  import { onMount, untrack } from 'svelte';

  import ConversationActionsMenu from '#lib/components/app/shell/ConversationActionsMenu.svelte';
  import ConversationControlOutlet from '#lib/components/app/shell/ConversationControlOutlet.svelte';
  import PluginOffState from '#lib/components/app/shell/PluginOffState.svelte';
  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Empty from '#lib/components/ui/empty/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { OpenConversation } from '#lib/state/conversation.svelte.js';
  import { pluginStates } from '#lib/state/plugin-states.svelte.js';
  import type { SidePanelProps } from '#lib/types/shell-slots.js';
  import { chatFailureMessage, pluginOffOf } from '#lib/utils/chat-errors.js';
  import { hasActiveRun } from '#lib/utils/conversation-reducer.js';

  import ReadOnlyFooter from './ReadOnlyFooter.svelte';
  import Transcript from './Transcript.svelte';

  // A conversation in a side panel (ADR-0025, decision 7; docs/features/
  // child-agents/child-agent-activity.md): its turns, its tool calls and its
  // status line, read-only, with Stop while a reply runs. Child agents open
  // their task conversations here. The shell keeps the panel while its tab
  // is open, so the argument never changes during the panel's life.
  let { argument, close, setLabel }: SidePanelProps = $props();

  const conversation = new OpenConversation(untrack(() => argument));

  const transcript = $derived(conversation.transcript);
  const summary = $derived(transcript.conversation);
  const replying = $derived(hasActiveRun(transcript));
  /** The kind of task of the plugin's executor whose conversation the panel shows, or `null`. */
  const executorKind = $derived(summary?.task?.executor?.kind ?? null);
  /** Whose conversation the panel shows: the owner's, a child agent's or a plugin's task's. */
  const owner = $derived.by(() => {
    const task = summary?.task ?? null;
    if (task === null) {
      return 'owner';
    }
    return executorKind === null ? 'child' : 'task';
  });

  /** The plugin of a task conversation that cannot be followed while the plugin is off, or `null`. */
  const offPlugin = $derived(pluginOffOf(conversation.failure));
  const pluginBackOn = $derived(offPlugin !== null && pluginStates.isOn(offPlugin));

  /** The tab's name once the conversation is known: a child's with its number, as its card shows it. */
  const tabLabel = $derived.by(() => {
    if (summary === null) {
      return null;
    }
    if (summary.task === null) {
      return summary.title;
    }
    return m.chat_panel_task_label({ number: String(summary.task.number), title: summary.title });
  });

  // The shell keeps the tabs' labels; the panel tells it its own.
  $effect(() => {
    if (tabLabel !== null) {
      setLabel(tabLabel, true);
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
    <div class="flex flex-1 flex-col gap-4 p-4" aria-busy="true">
      <span class="sr-only" role="status">{m.chat_loading()}</span>
      <Skeleton class="h-16 w-full" />
      <Skeleton class="ms-auto h-10 w-2/3" />
      <Skeleton class="h-24 w-full" />
    </div>
  {:else if conversation.status === 'deleted'}
    <Empty.Root class="flex-1">
      <Empty.Header>
        <Empty.Media variant="icon">
          <MessageSquareOffIcon />
        </Empty.Media>
        <Empty.Title>{m.chat_conversation_gone_title()}</Empty.Title>
        <Empty.Description>{m.chat_panel_gone_description()}</Empty.Description>
      </Empty.Header>
      <Empty.Content>
        <Button variant="outline" onclick={close}>{m.chat_close()}</Button>
      </Empty.Content>
    </Empty.Root>
  {:else if conversation.status === 'error'}
    {#if offPlugin === null}
      <div class="p-4">
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
      <div class="flex shrink-0 items-center justify-end gap-1 border-b px-3 py-1.5">
        <ConversationControlOutlet
          backgroundCommands={transcript.backgroundCommands}
          conversation={summary}
          turns={transcript.turns}
        />
        <ConversationActionsMenu
          backgroundCommands={transcript.backgroundCommands}
          conversation={summary}
          turns={transcript.turns}
        />
      </div>
    {/if}
    {#if conversation.status === 'ended'}
      <p class="border-b px-4 py-2 text-sm text-muted-foreground">{m.chat_conversation_ended()}</p>
    {/if}
    <Transcript {conversation} readOnly />
    <ReadOnlyFooter conversationId={conversation.id} kind={executorKind} {owner} {replying} />
  {/if}
</div>
