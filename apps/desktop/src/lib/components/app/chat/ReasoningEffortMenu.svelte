<script lang="ts">
  import BrainIcon from '@lucide/svelte/icons/brain';
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';

  import { Badge } from '#lib/components/ui/badge/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import type { ProviderProfile, ReasoningEffort } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';
  import { effortLabel, REASONING_EFFORTS } from '#lib/utils/provider-labels.js';

  interface Props {
    effort: ReasoningEffort | null;
    onChoose: (effort: ReasoningEffort | null) => Promise<ChatFailure | null>;
    profile: Pick<ProviderProfile, 'reasoningEffort' | 'supportedEfforts'>;
  }

  let { effort, onChoose, profile }: Props = $props();

  let failure = $state<ChatFailure | null>(null);

  const options = $derived(REASONING_EFFORTS.filter(offered));
  const effective = $derived.by(() => {
    if (effort === null) {
      return profile.reasoningEffort;
    }
    return options.includes(effort) ? effort : 'server_default';
  });
  const label = $derived(effortLabel(effective));

  function offered(level: ReasoningEffort): boolean {
    if (level === 'server_default') {
      return true;
    }
    if (level === profile.reasoningEffort) {
      return true;
    }
    return !profile.supportedEfforts || profile.supportedEfforts.includes(level);
  }

  async function choose(option: ReasoningEffort): Promise<void> {
    const next = option === profile.reasoningEffort ? null : option;

    failure = await onChoose(next);
  }
</script>

<div class="flex min-w-0 items-center gap-2">
  <DropdownMenu.Root>
    <DropdownMenu.Trigger>
      {#snippet child({ props })}
        <Button
          {...props}
          aria-label={m.chat_effort_trigger({ effort: label })}
          data-effort-picker=""
          size="sm"
          variant="ghost"
        >
          <BrainIcon data-icon="inline-start" />
          <span class="max-w-40 truncate">{label}</span>
          <ChevronDownIcon data-icon="inline-end" />
        </Button>
      {/snippet}
    </DropdownMenu.Trigger>

    <DropdownMenu.Content class="w-64" align="start">
      <DropdownMenu.Group>
        <DropdownMenu.Label>{m.chat_providers_field_effort()}</DropdownMenu.Label>
        <DropdownMenu.RadioGroup value={effective}>
          {#each options as option (option)}
            <DropdownMenu.RadioItem
              closeOnSelect
              data-effort={option}
              value={option}
              onSelect={() => void choose(option)}
            >
              <span class="min-w-0 truncate">{effortLabel(option)}</span>
              {#if option === profile.reasoningEffort}
                <Badge class="ms-auto shrink-0" variant="secondary">{m.chat_providers_default()}</Badge>
              {/if}
            </DropdownMenu.RadioItem>
          {/each}
        </DropdownMenu.RadioGroup>
      </DropdownMenu.Group>
    </DropdownMenu.Content>
  </DropdownMenu.Root>

  {#if failure !== null}
    <span class="text-sm text-destructive" role="alert">{chatFailureMessage(failure)}</span>
  {/if}
</div>
