<script lang="ts">
  import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';

  import { Button } from '#lib/components/ui/button/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import * as Tooltip from '#lib/components/ui/tooltip/index.js';
  import type { PartId, PartView, RunView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';
  import { secondClock } from '#lib/state/second-clock.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import { chatFailureMessage, runFailureMessage } from '#lib/utils/chat-errors.js';
  import { formatSeconds, secondsUntil } from '#lib/utils/durations.js';

  // What the latest reply is doing, below it (docs/features/chat/
  // conversation-view.md, parts; docs/features/system-tools/tool-activity.md,
  // status). The transcript announces the end of a reply.
  interface StatusLine {
    readonly busy: boolean;
    readonly retryable: boolean;
    readonly text: string;
  }

  interface Props {
    conversationId: string;
    /** The parts of the reply's turn, among them the tool call that runs. */
    parts: readonly PartView[];
    /** Whether the conversation shows read-only, without Retry. */
    readOnly?: boolean;
    run: RunView | null;
  }

  let { conversationId, parts, readOnly = false, run }: Props = $props();

  let retrying = $state(false);
  let retryFailure = $state<ChatFailure | null>(null);

  const line: StatusLine | null = $derived(statusLine(run));

  /** Whether the tool call in a part runs a command, which the status line names. */
  function runsCommand(partId: PartId): boolean {
    const content = parts.find((part) => part.id === partId)?.content;
    const isCommand = content?.kind === 'tool_call' && content.call.tool === 'run_command';
    return isCommand;
  }

  /** What a reply waits for: child agents, workers, or tasks of both. */
  function waitingForTasks(count: number, workers: number): string {
    if (workers === 0) {
      return m.chat_status_waiting_for_tasks({ count });
    }
    if (workers === count) {
      return m.chat_status_waiting_for_workers({ count });
    }
    return m.chat_status_waiting_for_any_tasks({ count });
  }

  function activeLine(current: RunView): StatusLine {
    const activity = current.activity;
    if (activity === null) {
      return { busy: true, retryable: false, text: m.chat_status_writing() };
    }
    switch (activity.kind) {
      case 'awaiting_approval': {
        return { busy: false, retryable: false, text: m.chat_status_awaiting_approval() };
      }
      case 'compacting': {
        return { busy: true, retryable: false, text: m.chat_status_compacting() };
      }
      case 'preparing': {
        return { busy: true, retryable: false, text: m.chat_status_preparing() };
      }
      case 'running_tool': {
        const text = runsCommand(activity.partId) ? m.chat_status_running_command() : m.chat_status_running_tool();
        return { busy: true, retryable: false, text };
      }
      case 'retrying': {
        // The countdown ticks once a second while it shows.
        const text = m.chat_status_retrying({
          attempt: String(activity.attempt),
          cause: runFailureMessage(activity.cause),
          maxAttempts: String(activity.maxAttempts),
          wait: formatSeconds(secondsUntil(activity.nextAttemptAt, secondClock.now)),
        });
        return { busy: true, retryable: false, text };
      }
      case 'waiting_for_slot': {
        return { busy: true, retryable: false, text: m.chat_status_waiting_for_slot() };
      }
      case 'waiting_for_lease': {
        return { busy: true, retryable: false, text: m.chat_status_waiting_for_lease() };
      }
      case 'waiting_for_tasks': {
        return { busy: true, retryable: false, text: waitingForTasks(activity.count, activity.workers ?? 0) };
      }
      case 'writing': {
        return { busy: true, retryable: false, text: m.chat_status_writing() };
      }
    }
  }

  function statusLine(current: RunView | null): StatusLine | null {
    if (current === null) {
      return null;
    }
    switch (current.state) {
      case 'completed': {
        // A server that cut the request answered without the start of the
        // conversation, which matters more than how the answer ended.
        if (current.outcome?.promptCut === true) {
          return { busy: false, retryable: false, text: m.chat_status_prompt_cut() };
        }
        const finish = current.outcome?.finish ?? null;
        if (finish === 'output_limit') {
          return { busy: false, retryable: false, text: m.chat_status_output_limit() };
        }
        if (finish === 'content_filter') {
          return { busy: false, retryable: false, text: m.chat_status_content_filter() };
        }
        return null;
      }
      case 'failed': {
        return null;
      }
      case 'interrupted': {
        return { busy: false, retryable: true, text: m.chat_status_interrupted() };
      }
      case 'queued':
      case 'running': {
        return activeLine(current);
      }
      case 'stopped': {
        return { busy: false, retryable: true, text: m.chat_status_stopped() };
      }
    }
  }

  async function retry(): Promise<void> {
    retrying = true;
    retryFailure = await chatActions.retry(conversationId);
    retrying = false;
  }
</script>

{#if line !== null}
  <div class="flex flex-col items-start gap-2 text-sm text-muted-foreground">
    <div class="flex flex-wrap items-center gap-2">
      {#if line.busy}
        <Spinner aria-label={m.chat_status_busy()} />
      {/if}
      <span>{line.text}</span>
    </div>
    {#if line.retryable && !readOnly}
      <Tooltip.Root>
        <Tooltip.Trigger disabled={retrying} onclick={retry}>
          {#snippet child({ props })}
            <Button {...props} aria-label={m.chat_retry()} size="icon-sm" variant="outline">
              <RotateCcwIcon />
            </Button>
          {/snippet}
        </Tooltip.Trigger>
        <Tooltip.Content>{m.chat_retry()}</Tooltip.Content>
      </Tooltip.Root>
    {/if}
  </div>
{/if}
{#if retryFailure !== null}
  <p class="text-sm text-destructive">{chatFailureMessage(retryFailure)}</p>
{/if}
