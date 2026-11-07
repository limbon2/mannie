<script lang="ts" module>
  import { SvelteSet } from 'svelte/reactivity';

  /** The conversations whose panel opened by itself, so a panel that the owner closed stays closed. */
  const opened = new SvelteSet<string>();
</script>

<script lang="ts">
  import { untrack } from 'svelte';

  import { sidePanels } from '#lib/state/side-panels.svelte.js';
  import type { ConversationControlProps } from '#lib/types/shell-slots.js';
  import { CHILD_AGENTS_PANEL } from '#lib/utils/child-agents-messages.js';
  import { childAgentsOf, isTaskActive } from '#lib/utils/task-parts.js';

  // Opens the child agents' side panel on a conversation's first task, while the
  // owner watches it, when its agent is a custom one, whose work is mostly its
  // children's. It renders nothing, takes no focus, and opens the panel once for
  // each conversation in a window's life.
  let { conversation, turns }: ConversationControlProps = $props();

  const isCustomAgent = $derived((conversation.agentKey ?? null) !== null);
  const anyWorks = $derived(childAgentsOf(turns).some((task) => isTaskActive(task.state)));
  const shouldOpen = $derived(isCustomAgent ? anyWorks : false);

  $effect(() => {
    if (!shouldOpen) {
      return;
    }
    const id = conversation.id;
    untrack(() => {
      if (!opened.has(id)) {
        opened.add(id);
        void sidePanels.open(CHILD_AGENTS_PANEL, id, { takesFocus: false });
      }
    });
  });
</script>
