<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import { onMount } from 'svelte';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import * as RadioGroup from '#lib/components/ui/radio-group/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import type { CloseAction } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { windowSettings } from '#lib/state/window-settings.svelte.js';
  import type { Message } from '#lib/types/shell-slots.js';
  import { windowFailureMessage } from '#lib/utils/window-messages.js';

  // What closing the window does (docs/features/window-and-tray.md,
  // setting): ask on the next close, keep running in the tray, or quit.
  interface CloseChoiceItem {
    readonly description: Message;
    readonly label: Message;
    readonly value: CloseAction;
  }

  const CHOICES: readonly CloseChoiceItem[] = [
    { description: m.window_settings_close_ask_description, label: m.window_settings_close_ask, value: 'ask' },
    { description: m.window_settings_close_tray_description, label: m.window_settings_close_tray, value: 'tray' },
    { description: m.window_settings_close_quit_description, label: m.window_settings_close_quit, value: 'quit' },
  ];

  function change(value: string): void {
    const choice = CHOICES.find((item) => item.value === value);
    if (choice === undefined) {
      return;
    }
    if (choice.value !== windowSettings.closeAction) {
      void windowSettings.changeCloseAction(choice.value);
    }
  }

  onMount(() => {
    void windowSettings.load();
  });
</script>

{#if windowSettings.readFailure !== null}
  <Alert.Root variant="destructive">
    <CircleAlertIcon />
    <Alert.Title>{m.window_settings_load_failed_title()}</Alert.Title>
    <Alert.Description>{windowFailureMessage(windowSettings.readFailure)}</Alert.Description>
    <Alert.Action>
      <Button size="sm" variant="outline" onclick={() => void windowSettings.load()}>{m.window_try_again()}</Button>
    </Alert.Action>
  </Alert.Root>
{:else if windowSettings.closeAction === null}
  <div class="flex flex-col gap-3" aria-busy="true">
    <Skeleton class="h-5 w-48" />
    <Skeleton class="h-5 w-64" />
    <Skeleton class="h-5 w-56" />
  </div>
{:else}
  <Field.Set>
    <Field.Legend variant="label">{m.window_settings_close_legend()}</Field.Legend>
    <RadioGroup.Root disabled={windowSettings.saving} value={windowSettings.closeAction} onValueChange={change}>
      {#each CHOICES as choice (choice.value)}
        <Field.Field orientation="horizontal">
          <RadioGroup.Item id="window-close-{choice.value}" value={choice.value} />
          <Field.Content>
            <Field.Label class="font-normal" for="window-close-{choice.value}">{choice.label()}</Field.Label>
            <Field.Description>{choice.description()}</Field.Description>
          </Field.Content>
        </Field.Field>
      {/each}
    </RadioGroup.Root>
  </Field.Set>
  {#if windowSettings.saving}
    <p class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
      <Spinner aria-label={m.window_settings_saving()} />
      <span>{m.window_settings_saving()}</span>
    </p>
  {/if}
  {#if windowSettings.saveFailure !== null}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.window_settings_save_failed_title()}</Alert.Title>
      <Alert.Description>{windowFailureMessage(windowSettings.saveFailure)}</Alert.Description>
    </Alert.Root>
  {/if}
{/if}
