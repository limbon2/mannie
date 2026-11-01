<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';

  import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { windowRequests } from '#lib/state/window-requests.svelte.js';
  import { quitWarningTitle, windowFailureMessage } from '#lib/utils/window-messages.js';

  // Quitting while replies are being written, background commands run or
  // terminals run commands, from the window in quit mode or from the tray's
  // menu (docs/features/window-and-tray.md, behavior). Cancel keeps
  // everything running.
  let {
    activeRuns,
    backgroundCommands,
    terminals,
  }: { activeRuns: number; backgroundCommands: number; terminals: number } = $props();

  const title = $derived(quitWarningTitle({ activeRuns, backgroundCommands, terminals }));
</script>

<AlertDialog.Root
  bind:open={
    () => windowRequests.open,
    (open) => {
      if (!open) {
        void windowRequests.dismiss();
      }
    }
  }
>
  <AlertDialog.Content>
    <AlertDialog.Header>
      <AlertDialog.Title>{title}</AlertDialog.Title>
      <AlertDialog.Description class="flex flex-col gap-2">
        {#if activeRuns > 0}
          <span>{m.window_active_runs({ count: activeRuns })}</span>
        {/if}
        {#if backgroundCommands > 0}
          <span>{m.window_background_commands({ count: backgroundCommands })}</span>
        {/if}
        {#if terminals > 0}
          <span>{m.window_terminals({ count: terminals })}</span>
        {/if}
      </AlertDialog.Description>
    </AlertDialog.Header>
    {#if windowRequests.failure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Description>{windowFailureMessage(windowRequests.failure)}</Alert.Description>
      </Alert.Root>
    {/if}
    <AlertDialog.Footer>
      <AlertDialog.Cancel>{m.window_cancel()}</AlertDialog.Cancel>
      <Button
        disabled={windowRequests.answering}
        variant="destructive"
        onclick={() => void windowRequests.confirmQuit()}
      >
        {m.window_quit_warning_confirm()}
      </Button>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>
