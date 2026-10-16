<script lang="ts">
  import LockOpenIcon from '@lucide/svelte/icons/lock-open';

  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { toolsActions } from '#lib/state/tools-actions.svelte.js';
  import type { ConversationControlProps } from '#lib/types/shell-slots.js';

  // The switch of a conversation's full access in its toolbar (ADR-0029;
  // docs/features/system-tools/tool-activity.md, full access). While full
  // access is on, the button shows in the destructive color, so the owner
  // sees that nothing asks. Turning it on asks first. A task conversation
  // follows its parent and has no switch.
  let { conversation }: ConversationControlProps = $props();

  const isTask = $derived(conversation.task !== null);
  const enabled = $derived(conversation.fullAccess ?? false);
</script>

{#if !isTask}
  <Button
    aria-pressed={enabled}
    data-full-access={enabled ? 'on' : 'off'}
    size="sm"
    variant={enabled ? 'destructive' : 'ghost'}
    onclick={() => toolsActions.switchFullAccess(conversation.id, !enabled)}
  >
    <LockOpenIcon data-icon="inline-start" />
    {m.tools_full_access()}
  </Button>
{/if}
