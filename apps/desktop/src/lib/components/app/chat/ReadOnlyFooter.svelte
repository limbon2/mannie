<script lang="ts">
  import SquareIcon from '@lucide/svelte/icons/square';

  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';

  // In place of the message box of a conversation that the owner cannot
  // write to: a child agent's, whose parent conversation gives it work, or
  // any conversation in a side panel (docs/features/child-agents/
  // child-agent-activity.md). Stop still works while a reply runs.
  interface Props {
    /** Whether the conversation belongs to a child agent. */
    child: boolean;
    conversationId: string;
    /** Whether a reply runs, which Stop ends. */
    replying: boolean;
  }

  let { child, conversationId, replying }: Props = $props();

  let stopping = $state(false);
  let failure = $state<ChatFailure | null>(null);

  async function stop(): Promise<void> {
    stopping = true;
    failure = await chatActions.stop(conversationId);
    stopping = false;
  }
</script>

<div class="flex flex-wrap items-center gap-3 border-t px-4 py-3 text-sm text-muted-foreground">
  <p class="min-w-0 flex-1">{child ? m.chat_read_only_child() : m.chat_read_only()}</p>
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
