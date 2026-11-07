<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import UsersIcon from '@lucide/svelte/icons/users';
  import { onMount, untrack } from 'svelte';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Empty from '#lib/components/ui/empty/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import type { TaskView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { OpenConversation } from '#lib/state/conversation.svelte.js';
  import { secondClock } from '#lib/state/second-clock.js';
  import { sidePanels } from '#lib/state/side-panels.svelte.js';
  import type { SidePanelProps } from '#lib/types/shell-slots.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';
  import { CONVERSATION_PANEL } from '#lib/utils/chat-urls.js';
  import { agentLabel, taskStateLabel } from '#lib/utils/child-agents-messages.js';
  import { relativeTime } from '#lib/utils/relative-time.js';
  import { childAgentsOf, isTaskActive } from '#lib/utils/task-parts.js';

  // A conversation's child agents in a side panel (docs/features/child-agents/
  // README.md, visibility): those that work and those that ended, each with its
  // state, agent and time. A row opens the child's conversation. The panel
  // follows the conversation on its own, and reads its earlier turns too, because
  // the list of ended children must be complete (ADR-0059).
  let { argument, setLabel }: SidePanelProps = $props();

  const conversation = new OpenConversation(untrack(() => argument));

  const numbers = new Intl.NumberFormat();

  const children = $derived(childAgentsOf(conversation.transcript.turns));
  const working = $derived(children.filter((task) => isTaskActive(task.state)));
  const ended = $derived(
    children
      .filter((task) => !isTaskActive(task.state))
      .toSorted((left, right) => (right.endedAt ?? 0) - (left.endedAt ?? 0)),
  );
  const groups = $derived([
    {
      heading: m.child_agents_panel_working({ count: numbers.format(working.length) }),
      id: 'working',
      tasks: working,
    },
    {
      heading: m.child_agents_panel_ended({ count: numbers.format(ended.length) }),
      id: 'ended',
      tasks: ended,
    },
  ]);
  /** Whether earlier turns remain to read, so the list of ended children may still grow. */
  const readsEarlier = $derived.by(() => {
    const hasMore = conversation.status === 'ready' ? conversation.transcript.hasEarlierTurns : false;
    return hasMore ? conversation.loadEarlierFailure === null : false;
  });
  const isReading = $derived(conversation.status === 'loading' ? true : readsEarlier);

  function open(task: TaskView, opener: HTMLElement): void {
    const label = {
      isContent: true,
      text: m.child_agents_tab_label({ number: String(task.number), title: task.title }),
    };
    void sidePanels.open(CONVERSATION_PANEL, task.conversationId, { label, opener });
  }

  // The earlier turns hold children that ended, so the panel reads them all, one
  // page at a time, while it shows.
  $effect(() => {
    if (readsEarlier && !conversation.loadingEarlier) {
      untrack(() => {
        void conversation.loadEarlier();
      });
    }
  });

  onMount(() => {
    setLabel(m.child_agents_panel_label(), false);
    conversation.open();
    return () => {
      conversation.close();
    };
  });
</script>

<div class="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4" aria-busy={isReading} data-child-agents-panel="">
  {#if conversation.status === 'loading'}
    <div class="flex flex-col gap-2">
      <span class="sr-only" role="status">{m.chat_loading()}</span>
      <Skeleton class="h-12 w-full" />
      <Skeleton class="h-12 w-full" />
    </div>
  {:else if conversation.status === 'error'}
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
  {:else if children.length === 0}
    <Empty.Root class="flex-1">
      <Empty.Header>
        <Empty.Media variant="icon">
          <UsersIcon />
        </Empty.Media>
        <Empty.Title>{m.child_agents_panel_empty_title()}</Empty.Title>
        <Empty.Description>{m.child_agents_panel_empty_description()}</Empty.Description>
      </Empty.Header>
    </Empty.Root>
  {:else}
    {#each groups as group (group.id)}
      <section class="flex flex-col gap-2" data-child-agents-group={group.id}>
        <h3 class="text-sm font-medium text-muted-foreground">{group.heading}</h3>
        {#if group.tasks.length === 0}
          <p class="text-sm text-muted-foreground">
            {group.id === 'working' ? m.child_agents_panel_none_working() : m.child_agents_panel_none_ended()}
          </p>
        {/if}
        <ul class="flex flex-col gap-1">
          {#each group.tasks as task (task.conversationId)}
            <li>
              <Button
                class="h-auto w-full justify-between gap-3 py-2 text-start font-normal"
                data-child-agent-row={task.number}
                variant="outline"
                onclick={(event: MouseEvent) => {
                  open(task, event.currentTarget as HTMLElement);
                }}
              >
                <span class="flex min-w-0 flex-col gap-0.5">
                  <span class="truncate" data-content="" dir="auto">{task.title}</span>
                  <span class="truncate text-xs text-muted-foreground">
                    {taskStateLabel(task)} · {agentLabel(task.agent)}
                  </span>
                </span>
                {#if task.endedAt !== null}
                  <span class="shrink-0 text-xs text-muted-foreground">
                    {relativeTime(task.endedAt, secondClock.now)}
                  </span>
                {/if}
              </Button>
            </li>
          {/each}
        </ul>
      </section>
    {/each}
    {#if conversation.loadEarlierFailure !== null}
      <p class="text-sm text-destructive" role="alert">{chatFailureMessage(conversation.loadEarlierFailure)}</p>
    {/if}
  {/if}
</div>
