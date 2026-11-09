<script lang="ts">
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
  import ClipboardListIcon from '@lucide/svelte/icons/clipboard-list';
  import { untrack } from 'svelte';

  import * as Collapsible from '#lib/components/ui/collapsible/index.js';
  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { conversationList } from '#lib/state/conversation-list.svelte.js';
  import { plans } from '#lib/state/plans.svelte.js';
  import { sidePanels } from '#lib/state/side-panels.svelte.js';
  import { sidebarCollapse } from '#lib/state/sidebar-collapse.svelte.js';
  import { PLAN_PANEL } from '#lib/utils/plans-messages.js';

  // The "Plans" group of Chat's sidebar (docs/features/plans.md): every plan,
  // the one that changed last first. It hides while there are none, because a
  // plan's card in the transcript is where the owner first meets one. It reads
  // the plans again when a reply ends, because a plan comes from a reply that
  // may not show. The group collapses from its heading, as "Projects" does.
  const replying = $derived(conversationList.chats.filter((chat) => chat.hasActiveRun).length);

  function setOpen(open: boolean): void {
    sidebarCollapse.setGroupOpen('plans', open);
  }

  $effect(() => {
    const count = replying;
    untrack(() => {
      if (count === 0) {
        void plans.reload();
      }
    });
  });
</script>

{#if plans.plans.length > 0}
  <Collapsible.Root
    class="group/collapsible group-data-[collapsible=icon]:hidden"
    bind:open={() => sidebarCollapse.isGroupOpen('plans'), setOpen}
  >
    <Sidebar.Group data-plans-panel="">
      <Sidebar.GroupLabel class="gap-1 hover:text-sidebar-foreground">
        {#snippet child({ props })}
          <Collapsible.Trigger {...props}>
            <span>{m.plans_panel_title()}</span>
            <ChevronDownIcon class="transition-transform group-data-[state=closed]/collapsible:-rotate-90" />
          </Collapsible.Trigger>
        {/snippet}
      </Sidebar.GroupLabel>
      <Collapsible.Content>
        <Sidebar.GroupContent>
          <Sidebar.Menu>
            {#each plans.plans as plan (plan.number)}
              <Sidebar.MenuItem>
                <Sidebar.MenuButton
                  data-plan-row={plan.number}
                  onclick={(event: MouseEvent) =>
                    void sidePanels.open(PLAN_PANEL, String(plan.number), {
                      label: { isContent: true, text: plan.title },
                      opener: event.currentTarget as HTMLElement,
                    })}
                >
                  <ClipboardListIcon />
                  <span class="truncate" data-content="" dir="auto">{plan.title}</span>
                </Sidebar.MenuButton>
              </Sidebar.MenuItem>
            {/each}
          </Sidebar.Menu>
        </Sidebar.GroupContent>
      </Collapsible.Content>
    </Sidebar.Group>
  </Collapsible.Root>
{/if}
