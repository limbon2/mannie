<script lang="ts">
  import { Button } from '#lib/components/ui/button/index.js';
  import type { BackgroundCommandView, ConversationId } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { sidePanels } from '#lib/state/side-panels.svelte.js';
  import { stopCommand } from '#lib/state/tools-background.svelte.js';
  import type { ToolsFailure } from '#lib/types/tools.js';
  import { COMMAND_OUTPUT_PANEL, commandLabel, commandPanelArgument, isRunning } from '#lib/utils/tools-background.js';
  import { toolsFailureMessage } from '#lib/utils/tools-messages.js';

  // What the owner can do with a command from the card of the call that
  // started it (docs/features/system-tools/background-commands.md,
  // interface): show its output in a side panel, and stop it while it runs.
  // A worker task's command stays on the worker: its card says how to stop
  // it.
  interface Props {
    command: BackgroundCommandView;
    conversationId: ConversationId;
  }

  let { command, conversationId }: Props = $props();

  let opener = $state<HTMLElement | null>(null);
  let stopping = $state(false);
  let failure = $state<ToolsFailure | null>(null);

  function showOutput(): void {
    const label = { isContent: true, text: commandLabel(command) };
    const argument = commandPanelArgument(conversationId, command.number);
    void sidePanels.open(COMMAND_OUTPUT_PANEL, argument, { label, opener });
  }

  async function stop(): Promise<void> {
    stopping = true;
    failure = await stopCommand(conversationId, command.number);
    stopping = false;
  }
</script>

{#if command.onWorker}
  <p class="text-sm text-muted-foreground">{m.tools_background_on_worker()}</p>
{:else}
  <div class="flex flex-wrap items-center gap-2" data-background-command-actions="">
    <Button size="sm" variant="outline" bind:ref={opener} onclick={showOutput}>
      {m.tools_background_show_output()}
    </Button>
    {#if isRunning(command)}
      <Button disabled={stopping} size="sm" variant="outline" onclick={() => void stop()}>
        {stopping ? m.tools_background_stopping() : m.tools_background_stop()}
      </Button>
    {/if}
  </div>
  {#if failure !== null}
    <p class="text-sm text-destructive" role="alert">{toolsFailureMessage(failure)}</p>
  {/if}
{/if}
