<script lang="ts">
  import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { updates } from '#lib/state/updates.svelte.js';
  import { restartQuestionTitle } from '#lib/utils/updates-messages.js';

  // The question before a restart that would end running work (docs/features/
  // updates.md). The core counted the work when the owner chose "Restart and
  // update", and it counts again when the owner confirms.
  const question = $derived(updates.question);
  const needsSystemPassword = $derived(updates.status?.needsSystemPassword ?? false);
</script>

<AlertDialog.Root
  bind:open={
    () => question !== null,
    (open) => {
      if (!open) {
        updates.cancelRestart();
      }
    }
  }
>
  <AlertDialog.Content data-update-question="">
    {#if question !== null}
      <AlertDialog.Header>
        <AlertDialog.Title>{restartQuestionTitle(question)}</AlertDialog.Title>
        <AlertDialog.Description class="flex flex-col gap-2">
          {#if question.activeRuns > 0}
            <span data-update-question-count="active-runs">
              {m.updates_restart_active_runs({ count: question.activeRuns })}
            </span>
          {/if}
          {#if question.backgroundCommands > 0}
            <span data-update-question-count="background-commands">
              {m.updates_restart_background_commands({ count: question.backgroundCommands })}
            </span>
          {/if}
          {#if question.terminals > 0}
            <span data-update-question-count="terminals">
              {m.updates_restart_terminals({ count: question.terminals })}
            </span>
          {/if}
          {#if needsSystemPassword}
            <span>{m.updates_needs_system_password()}</span>
          {/if}
        </AlertDialog.Description>
      </AlertDialog.Header>
      <AlertDialog.Footer>
        <AlertDialog.Cancel>{m.updates_cancel()}</AlertDialog.Cancel>
        <Button
          data-update-question-confirm=""
          disabled={updates.pending !== null}
          variant="destructive"
          onclick={() => void updates.confirmRestart()}
        >
          {m.updates_restart_and_update()}
        </Button>
      </AlertDialog.Footer>
    {/if}
  </AlertDialog.Content>
</AlertDialog.Root>
