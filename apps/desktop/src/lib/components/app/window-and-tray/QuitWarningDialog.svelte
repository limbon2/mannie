<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';

  import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { windowRequests } from '#lib/state/window-requests.svelte.js';
  import { windowFailureMessage } from '#lib/utils/window-messages.js';

  // Quitting while replies are being written, from the window in quit mode
  // or from the tray's menu (docs/features/window-and-tray.md, behavior).
  // Cancel keeps everything running.
  let { activeRuns }: { activeRuns: number } = $props();
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
      <AlertDialog.Title>{m.window_quit_warning_title()}</AlertDialog.Title>
      <AlertDialog.Description>{m.window_active_runs({ count: activeRuns })}</AlertDialog.Description>
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
