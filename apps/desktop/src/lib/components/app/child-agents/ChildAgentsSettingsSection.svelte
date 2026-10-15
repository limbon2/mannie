<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import { onMount } from 'svelte';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import * as Select from '#lib/components/ui/select/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { childAgentsSettings } from '#lib/state/child-agents-settings.svelte.js';
  import { childAgentsFailureMessage } from '#lib/utils/child-agents-messages.js';

  // The Child agents section of the settings (docs/features/child-agents/
  // child-agent-activity.md, settings): how many child agents work at the
  // same time on this computer. The core applies a change at once: a higher
  // limit starts waiting tasks, and a lower one takes effect as children end.
  interface LimitItem {
    readonly label: string;
    readonly value: string;
  }

  /** The limits the core accepts, and the one it starts with. */
  const MIN_LIMIT = 1;
  const MAX_LIMIT = 16;
  const DEFAULT_LIMIT = 4;

  const numbers = new Intl.NumberFormat();
  const items: LimitItem[] = Array.from({ length: MAX_LIMIT - MIN_LIMIT + 1 }, (_, index) => {
    const limit = MIN_LIMIT + index;
    const count = numbers.format(limit);
    const label = limit === DEFAULT_LIMIT ? m.child_agents_settings_default_limit({ count }) : count;
    return { label, value: String(limit) };
  });

  const settings = $derived(childAgentsSettings.settings);

  function change(value: string): void {
    const limit = Number(value);
    const isChange = limit !== settings?.parallelChildren;
    if (isChange) {
      void childAgentsSettings.setParallelChildren(limit);
    }
  }

  onMount(() => {
    void childAgentsSettings.load();
  });
</script>

{#if childAgentsSettings.readFailure !== null}
  <Alert.Root variant="destructive">
    <CircleAlertIcon />
    <Alert.Title>{m.shell_settings_load_failed_title()}</Alert.Title>
    <Alert.Description>{childAgentsFailureMessage(childAgentsSettings.readFailure)}</Alert.Description>
    <Alert.Action>
      <Button size="sm" variant="outline" onclick={() => void childAgentsSettings.load()}>
        {m.child_agents_try_again()}
      </Button>
    </Alert.Action>
  </Alert.Root>
{:else if settings === null}
  <div class="flex flex-col gap-3" aria-busy="true">
    <span class="sr-only" role="status">{m.child_agents_loading()}</span>
    <Skeleton class="h-5 w-56" />
    <Skeleton class="h-9 w-full max-w-xs" />
  </div>
{:else}
  <Field.Field>
    <Field.Label for="child-agents-limit">{m.child_agents_settings_limit_label()}</Field.Label>
    <Select.Root
      disabled={childAgentsSettings.saving}
      {items}
      type="single"
      value={String(settings.parallelChildren)}
      onValueChange={change}
    >
      <Select.Trigger id="child-agents-limit" class="w-full max-w-xs">
        <Select.Value />
      </Select.Trigger>
      <Select.Content>
        {#each items as item (item.value)}
          <Select.Item label={item.label} value={item.value}>{item.label}</Select.Item>
        {/each}
      </Select.Content>
    </Select.Root>
    <Field.Description>{m.child_agents_settings_limit_description()}</Field.Description>
    <div role="status">
      {#if childAgentsSettings.saving}
        <p class="flex items-center gap-2 text-sm text-muted-foreground">
          <Spinner aria-label={m.child_agents_loading()} />
          <span>{m.child_agents_settings_saving()}</span>
        </p>
      {:else if childAgentsSettings.saved}
        <p class="text-sm text-muted-foreground">{m.child_agents_settings_saved()}</p>
      {/if}
    </div>
    {#if childAgentsSettings.saveFailure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.child_agents_settings_save_failed_title()}</Alert.Title>
        <Alert.Description>{childAgentsFailureMessage(childAgentsSettings.saveFailure)}</Alert.Description>
      </Alert.Root>
    {/if}
  </Field.Field>
{/if}
