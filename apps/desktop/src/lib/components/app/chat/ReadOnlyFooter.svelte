<script lang="ts">
  import SquareIcon from '@lucide/svelte/icons/square';

  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';

  // In place of the message box of a conversation that the owner cannot
  // write to: a child agent's or the task of a plugin's executor, whose
  // parent conversation gives it work, or any conversation in a side panel
  // (docs/features/child-agents/child-agent-activity.md). The kind of task
  // of a plugin that is on says it in its own words (ADR-0031, decision 7).
  // Stop still works while a reply runs.
  interface Props {
    conversationId: string;
    /** The kind of task of the plugin's executor, for a task conversation of one. */
    kind?: string | null;
    /** Whose conversation it is: the owner's, a child agent's or a plugin's task's. */
    owner: 'child' | 'owner' | 'task';
    /** Whether a reply runs, which Stop ends. */
    replying: boolean;
  }

  let { conversationId, kind = null, owner, replying }: Props = $props();

  const slots = getShellSlots();

  let stopping = $state(false);
  let failure = $state<ChatFailure | null>(null);

  const note = $derived.by(() => {
    switch (owner) {
      case 'child': {
        return m.chat_read_only_child();
      }
      case 'owner': {
        return m.chat_read_only();
      }
      case 'task': {
        const taskKind = kind === null ? undefined : slots.taskKinds.find(kind);
        return taskKind?.readOnly() ?? m.chat_read_only_task();
      }
    }
  });

  async function stop(): Promise<void> {
    stopping = true;
    failure = await chatActions.stop(conversationId);
    stopping = false;
  }
</script>

<div class="flex flex-wrap items-center gap-3 border-t px-4 py-3 text-sm text-muted-foreground">
  <p class="min-w-0 flex-1">{note}</p>
  {#if replying}
    <Button disabled={stopping} size="sm" variant="outline" onclick={stop}>
      <SquareIcon data-icon="inline-start" />
      {m.chat_stop()}
    </Button>
  {/if}
  {#if failure !== null}
    <p class="w-full text-destructive">{chatFailureMessage(failure)}</p>
  {/if}
</div>
