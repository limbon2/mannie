<script lang="ts">
  import BrainIcon from '@lucide/svelte/icons/brain';
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';

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
  const usesOverride = $derived(effort !== null && options.includes(effective));
  const checked = $derived(usesOverride ? effective : 'default');
  const label = $derived(effortLabel(effective));

  function offered(level: ReasoningEffort): boolean {
    if (level === 'server_default') {
      return profile.reasoningEffort !== 'server_default';
    }
    return !profile.supportedEfforts || profile.supportedEfforts.includes(level);
  }

  async function choose(value: ReasoningEffort | null): Promise<void> {
    failure = await onChoose(value);
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

    <DropdownMenu.Content align="start">
      <DropdownMenu.Group>
        <DropdownMenu.Label>{m.chat_providers_field_effort()}</DropdownMenu.Label>
        <DropdownMenu.RadioGroup value={checked}>
          <DropdownMenu.RadioItem
            closeOnSelect
            data-effort="default"
            value="default"
            onSelect={() => void choose(null)}
          >
            {m.chat_effort_default({ effort: effortLabel(profile.reasoningEffort) })}
          </DropdownMenu.RadioItem>
          {#each options as option (option)}
            <DropdownMenu.RadioItem
              closeOnSelect
              data-effort={option}
              value={option}
              onSelect={() => void choose(option)}
            >
              {effortLabel(option)}
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
