<script lang="ts">
  import { onMount } from 'svelte';

  import { commands } from '#lib/generated/bindings.js';
  import { windowRequests } from '#lib/state/window-requests.svelte.js';
  import { trayLabels } from '#lib/utils/window-messages.js';

  import CloseQuestionDialog from './CloseQuestionDialog.svelte';
  import QuitWarningDialog from './QuitWarningDialog.svelte';

  // The window and tray feature's overlay (ADR-0020). For as long as the
  // window runs, it receives the core's questions about closing and quitting,
  // and it gives the tray its labels in the window's language (ADR-0019). A
  // reload mounts it again, which subscribes again and sends the labels again.
  const request = $derived(windowRequests.question?.request ?? null);

  onMount(() => {
    windowRequests.start();
    // The core logs a refused label or a tray that cannot be created. Mannie
    // then runs without the tray: closing in tray mode hides the window, and
    // starting Mannie again shows it.
    void commands.setTrayLabels(trayLabels());
  });
</script>

{#if request?.kind === 'ask_close_action'}
  <CloseQuestionDialog activeRuns={request.activeRuns} />
{:else if request?.kind === 'confirm_quit'}
  <QuitWarningDialog activeRuns={request.activeRuns} />
{/if}
