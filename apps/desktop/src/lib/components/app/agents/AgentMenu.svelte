<script lang="ts">
  import BotIcon from '@lucide/svelte/icons/bot';
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
  import { onMount } from 'svelte';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { agents } from '#lib/state/agents.svelte.js';
  import { agentName, DEFAULT_AGENT } from '#lib/utils/agents-messages.js';
  import { openAgents } from '#lib/utils/agents-urls.js';

  // The agent of a conversation's next replies, or of a new conversation's first
  // one (ADR-0059, decision 6). A key that no agent has, such as a deleted
  // agent's, reads as the built-in agent, as the core reads it.
  interface Props {
    /** The chosen agent's key, or `null` for the built-in one. */
    chosen: string | null;
    onChoose: (key: string | null) => void;
  }

  let { chosen, onChoose }: Props = $props();

  const shown = $derived(agents.find(chosen) ?? agents.find(DEFAULT_AGENT));
  const name = $derived(shown === undefined ? m.agents_name_default() : agentName(shown));
  const value = $derived(shown?.key ?? DEFAULT_AGENT);

  onMount(() => {
    void agents.ensureLoaded();
  });
</script>

<DropdownMenu.Root>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}
      <Button {...props} aria-label={m.agents_picker_trigger({ name })} data-agent-picker="" size="sm" variant="ghost">
        <BotIcon data-icon="inline-start" />
        <span class="max-w-40 truncate" data-content={shown?.builtin === false ? '' : undefined} dir="auto">{name}</span
        >
        <ChevronDownIcon data-icon="inline-end" />
      </Button>
    {/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Content class="w-64" align="start">
    <DropdownMenu.Group>
      <DropdownMenu.Label>{m.agents_picker_label()}</DropdownMenu.Label>
      <DropdownMenu.RadioGroup
        {value}
        onValueChange={(next: string) => {
          onChoose(next === DEFAULT_AGENT ? null : next);
        }}
      >
        {#each agents.conversationAgents as agent (agent.key)}
          <DropdownMenu.RadioItem closeOnSelect data-agent-option={agent.key} value={agent.key}>
            <span class="truncate" data-content={agent.builtin ? undefined : ''} dir="auto">{agentName(agent)}</span>
          </DropdownMenu.RadioItem>
        {/each}
      </DropdownMenu.RadioGroup>
    </DropdownMenu.Group>
    <DropdownMenu.Separator />
    <DropdownMenu.Group>
      <DropdownMenu.Item data-agent-manage="" onSelect={() => void openAgents()}>
        {m.agents_picker_manage()}
      </DropdownMenu.Item>
    </DropdownMenu.Group>
  </DropdownMenu.Content>
</DropdownMenu.Root>
