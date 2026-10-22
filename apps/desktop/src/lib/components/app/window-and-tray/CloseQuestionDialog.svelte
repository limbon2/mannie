<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Dialog from '#lib/components/ui/dialog/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { windowRequests } from '#lib/state/window-requests.svelte.js';
  import { windowFailureMessage } from '#lib/utils/window-messages.js';

  // The first close of the window (docs/features/window-and-tray.md,
  // behavior): keep running in the tray, or quit. While replies are being
  // written or background commands run, it says that quitting ends them, so
  // choosing Quit here needs no second warning. Cancel keeps the window open
  // and asks again on the next close.
  let { activeRuns, backgroundCommands }: { activeRuns: number; backgroundCommands: number } = $props();
</script>

<Dialog.Root
  bind:open={
    () => windowRequests.open,
    (open) => {
      if (!open) {
        void windowRequests.dismiss();
      }
    }
  }
>
  <Dialog.Content showCloseButton={false}>
    <Dialog.Header>
      <Dialog.Title>{m.window_close_question_title()}</Dialog.Title>
      <Dialog.Description>{m.window_close_question_description()}</Dialog.Description>
    </Dialog.Header>
    {#if activeRuns > 0}
      <Alert.Root>
        <CircleAlertIcon />
        <Alert.Description>{m.window_active_runs({ count: activeRuns })}</Alert.Description>
      </Alert.Root>
    {/if}
    {#if backgroundCommands > 0}
      <Alert.Root>
        <CircleAlertIcon />
        <Alert.Description>{m.window_background_commands({ count: backgroundCommands })}</Alert.Description>
      </Alert.Root>
    {/if}
    {#if windowRequests.failure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Description>{windowFailureMessage(windowRequests.failure)}</Alert.Description>
      </Alert.Root>
    {/if}
    <Dialog.Footer>
      <Button variant="outline" onclick={() => void windowRequests.dismiss()}>{m.window_cancel()}</Button>
      <Button
        disabled={windowRequests.answering}
        variant="outline"
        onclick={() => void windowRequests.answerClose('quit')}
      >
        {m.window_close_question_quit()}
      </Button>
      <Button disabled={windowRequests.answering} onclick={() => void windowRequests.answerClose('tray')}>
        {m.window_close_question_tray()}
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
