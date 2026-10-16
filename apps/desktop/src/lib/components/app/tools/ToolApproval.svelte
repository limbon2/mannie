<script lang="ts">
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import type { Access, ApprovalChoice, ApprovalRequest, ConversationId, PartId } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { toolsActions } from '#lib/state/tools-actions.svelte.js';
  import { approvalReasonMessage, grantScopeMessage } from '#lib/utils/tools-messages.js';

  import ToolAccessList from './ToolAccessList.svelte';

  // A tool call that waits for the owner (docs/features/system-tools/
  // tool-activity.md, approval requests): what will happen, exactly as the
  // core prepared it, why Mannie asks, what "Allow for…" would cover, and the
  // owner's options. "Allow for…" lasts for this conversation, its project or
  // every conversation; the project is offered only in a project's
  // conversation. The buttons belong to the card's own layout, outside every
  // piece of content (T-3). Nothing here takes the focus by itself.
  interface Props {
    accesses: readonly Access[];
    conversationId: ConversationId;
    partId: PartId;
    request: ApprovalRequest;
  }

  let { accesses, conversationId, partId, request }: Props = $props();

  const deciding = $derived(toolsActions.deciding.includes(partId));
  const inProject = $derived(request.project !== null && request.project !== undefined);
  /** The computer that runs the task of the operation, by the name its card shows, or `null` for this computer. */
  const host = $derived(request.host ?? null);

  function decide(choice: ApprovalChoice): void {
    void toolsActions.decide(conversationId, partId, request.digest, choice);
  }
</script>

<div class="flex flex-col gap-3 rounded-md border bg-muted/40 p-3">
  <p class="text-sm font-medium">{m.tools_approval_title()}</p>
  <div class="flex flex-col gap-1.5">
    <p class="text-xs font-medium text-muted-foreground">{m.tools_approval_will()}</p>
    <ToolAccessList {accesses} />
  </div>
  {#if host !== null}
    <!-- The computer's name is content, which it or the owner wrote. -->
    <div class="flex flex-col gap-1">
      <p class="text-xs font-medium text-muted-foreground">{m.tools_approval_host()}</p>
      <p class="text-sm wrap-anywhere" data-approval-host="" data-content dir="auto">{host}</p>
    </div>
  {/if}
  {#if request.reasons.length > 0}
    <div class="flex flex-col gap-1">
      <p class="text-xs font-medium text-muted-foreground">{m.tools_approval_why()}</p>
      <ul class="list-disc ps-5 text-sm">
        {#each request.reasons as reason (reason)}
          <li>{approvalReasonMessage(reason)}</li>
        {/each}
      </ul>
    </div>
  {/if}
  {#if request.grants.length > 0}
    <div class="flex flex-col gap-1">
      <p class="text-xs font-medium text-muted-foreground">{m.tools_approval_covers()}</p>
      <ul class="list-disc ps-5 text-sm break-all">
        {#each request.grants as grant, index (index)}
          <li>{grantScopeMessage(grant)}</li>
        {/each}
      </ul>
    </div>
  {/if}
  <div class="flex flex-wrap gap-2">
    <Button disabled={deciding} size="sm" onclick={() => decide({ choice: 'allow_once' })}>
      {m.tools_allow_once()}
    </Button>
    {#if request.grants.length > 0}
      <DropdownMenu.Root>
        <DropdownMenu.Trigger>
          {#snippet child({ props })}
            <Button {...props} data-allow-for disabled={deciding} size="sm" variant="outline">
              {m.tools_allow_for()}
              <ChevronDownIcon data-icon="inline-end" />
            </Button>
          {/snippet}
        </DropdownMenu.Trigger>
        <DropdownMenu.Content align="start">
          <DropdownMenu.Group>
            <DropdownMenu.Item onSelect={() => decide({ choice: 'allow_for_conversation' })}>
              {m.tools_allow_for_conversation()}
            </DropdownMenu.Item>
            {#if inProject}
              <DropdownMenu.Item onSelect={() => decide({ choice: 'allow_for_project' })}>
                {m.tools_allow_for_project()}
              </DropdownMenu.Item>
            {/if}
            <DropdownMenu.Item onSelect={() => decide({ choice: 'allow_always' })}>
              {m.tools_allow_always()}
            </DropdownMenu.Item>
          </DropdownMenu.Group>
        </DropdownMenu.Content>
      </DropdownMenu.Root>
    {/if}
    {#if request.rule !== null}
      <Button
        disabled={deciding}
        size="sm"
        variant="outline"
        onclick={() => toolsActions.startSaveRule(conversationId, partId, request)}
      >
        {m.tools_save_rule_open()}
      </Button>
    {/if}
    <Button disabled={deciding} size="sm" variant="destructive" onclick={() => decide({ choice: 'deny' })}>
      {m.tools_deny()}
    </Button>
  </div>
</div>
