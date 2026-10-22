<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import { onMount, untrack } from 'svelte';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import { Label } from '#lib/components/ui/label/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { Switch } from '#lib/components/ui/switch/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { secondClock } from '#lib/state/second-clock.js';
  import { toolsActions } from '#lib/state/tools-actions.svelte.js';
  import { CommandOutputView } from '#lib/state/tools-background.svelte.js';
  import type { SidePanelProps } from '#lib/types/shell-slots.js';
  import {
    commandLabel,
    commandPlace,
    commandState,
    outputArchive,
    parseCommandPanelArgument,
  } from '#lib/utils/tools-background.js';
  import { toolsFailureMessage } from '#lib/utils/tools-messages.js';

  import AccessText from './AccessText.svelte';

  // The output of a background command in a side panel (docs/features/
  // system-tools/background-commands.md, interface): the command, where it
  // runs, its state and Stop while it runs, and its output as it arrives,
  // the last lines of it, in a log region that scrolls from the keyboard.
  // After the end, the whole output opens from its archive. The output is
  // content, shown as plain text (T-3). The shell keeps the panel while its
  // tab is open, so the argument never changes during the panel's life. An
  // address that names no command can only close.
  let { argument, close, setLabel }: SidePanelProps = $props();

  const id = $props.id();
  const target = parseCommandPanelArgument(untrack(() => argument));
  const view = target === null ? null : new CommandOutputView(target.conversationId, target.number);

  let follow = $state(true);

  const command = $derived(view?.command ?? null);
  const running = $derived(view?.running ?? false);
  // Only a command that runs reads the clock, so an ended one starts no timer.
  const now = $derived(running ? secondClock.now : 0);
  const stateLabel = $derived(command === null ? '' : commandState(command, now));
  const lost = $derived(command !== null && command.ending !== null && command.archiveId === null);
  /** Why the output could not be followed, if it could not. */
  const failure = $derived(view?.failure ?? null);
  const archive = $derived(command === null ? null : outputArchive(command));
  const text = $derived(
    (view?.entries ?? [])
      .map((entry) =>
        entry.kind === 'line'
          ? entry.text
          : `[${m.tools_background_lines_left_out({ first: entry.first, last: entry.last })}]`,
      )
      .join('\n'),
  );

  /**
   * Keeps the output at its end while the owner follows it. It runs again
   * when lines arrive or the switch changes, after the output shows them.
   */
  function followEnd(region: HTMLElement): void {
    const followsEnd = follow && text !== '';
    if (followsEnd) {
      region.scrollTop = region.scrollHeight;
    }
  }

  function showFullOutput(): void {
    const canShow = view !== null && archive !== null;
    if (canShow) {
      toolsActions.showOutput(view.conversationId, archive);
    }
  }

  // The shell keeps the tabs' labels; the panel tells it its own.
  $effect(() => {
    if (command !== null) {
      setLabel(commandLabel(command), true);
    }
  });

  onMount(() => {
    void view?.follow();
    return () => {
      view?.close();
    };
  });
</script>

<div class="flex min-h-0 flex-1 flex-col gap-3 p-4" data-command-output="">
  {#if view === null}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.tools_background_output_failed()}</Alert.Title>
      <Alert.Description>{m.tools_background_panel_invalid()}</Alert.Description>
      <Alert.Action>
        <Button size="sm" variant="outline" onclick={close}>{m.tools_background_close()}</Button>
      </Alert.Action>
    </Alert.Root>
  {:else}
    {#if command !== null}
      <div class="flex flex-col gap-2">
        <AccessText label={m.tools_access_command()} text={command.command} />
        <p class="text-sm break-all text-muted-foreground" data-content dir="auto">{commandPlace(command)}</p>
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div class="flex flex-wrap items-center gap-3">
            <p class="text-sm font-medium">{stateLabel}</p>
            {#if running && !command.onWorker}
              <Button disabled={view.stopping} size="sm" variant="outline" onclick={() => void view.stop()}>
                {view.stopping ? m.tools_background_stopping() : m.tools_background_stop()}
              </Button>
            {/if}
            {#if archive !== null}
              <Button size="sm" variant="outline" onclick={showFullOutput}>
                {m.tools_background_show_full_output()}
              </Button>
            {/if}
          </div>
          <div class="flex items-center gap-2">
            <Switch id="{id}-follow" bind:checked={follow} />
            <Label for="{id}-follow">{m.tools_background_follow()}</Label>
          </div>
        </div>
        {#if view.stopFailure !== null}
          <p class="text-sm text-destructive" role="alert">{toolsFailureMessage(view.stopFailure)}</p>
        {/if}
        {#if command.onWorker}
          <p class="text-sm text-muted-foreground">{m.tools_background_on_worker()}</p>
        {/if}
      </div>
      {#if lost}
        <p class="text-sm text-muted-foreground">{m.tools_background_output_lost()}</p>
      {:else if text === ''}
        <p class="text-sm text-muted-foreground">{m.tools_background_output_empty()}</p>
      {:else}
        <!-- Focusable, so that the owner can scroll the output from the keyboard (WCAG 2.1.1). -->
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <pre
          class="min-h-0 flex-1 overflow-auto rounded-md bg-muted px-3 py-2 font-mono text-xs break-all whitespace-pre-wrap outline-none focus-visible:ring-2 focus-visible:ring-ring"
          {@attach followEnd}
          aria-label={m.tools_background_output_label({ number: command.number })}
          data-content
          role="log"
          tabindex="0">{text}</pre>
      {/if}
    {:else if failure === null}
      <div class="flex flex-col gap-2" aria-busy="true">
        <span class="sr-only" role="status">{m.tools_background_loading()}</span>
        <Skeleton class="h-5 w-40" />
        <Skeleton class="h-12 w-full" />
        <Skeleton class="h-4 w-full" />
        <Skeleton class="h-4 w-5/6" />
        <Skeleton class="h-4 w-2/3" />
      </div>
    {/if}
    {#if failure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.tools_background_output_failed()}</Alert.Title>
        <Alert.Description>{toolsFailureMessage(failure)}</Alert.Description>
        <Alert.Action>
          <Button size="sm" variant="outline" onclick={() => void view.follow()}>
            {m.tools_background_try_again()}
          </Button>
        </Alert.Action>
      </Alert.Root>
    {/if}
  {/if}
</div>
