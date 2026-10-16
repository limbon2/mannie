<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';

  import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { toolsActions } from '#lib/state/tools-actions.svelte.js';
  import { toolsFailureMessage } from '#lib/utils/tools-messages.js';

  // Confirms that a conversation turns full access on, and shows why a
  // switch failed (ADR-0029; docs/features/system-tools/tool-activity.md,
  // full access). `toolsActions.switchFullAccess` opens it before full
  // access turns on, and when turning it off failed.
  const request = $derived(toolsActions.fullAccess);
</script>

<AlertDialog.Root bind:open={toolsActions.fullAccessOpen}>
  <AlertDialog.Content>
    {#if request !== null}
      {#if request.enable}
        <AlertDialog.Header>
          <AlertDialog.Title>{m.tools_full_access_confirm_title()}</AlertDialog.Title>
          <AlertDialog.Description>{m.tools_full_access_confirm_description()}</AlertDialog.Description>
        </AlertDialog.Header>
        {#if request.failure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title>{m.tools_full_access_on_failed_title()}</Alert.Title>
            <Alert.Description>{toolsFailureMessage(request.failure)}</Alert.Description>
          </Alert.Root>
        {/if}
        <AlertDialog.Footer>
          <AlertDialog.Cancel>{m.tools_cancel()}</AlertDialog.Cancel>
          <Button
            data-full-access-confirm=""
            disabled={request.saving}
            variant="destructive"
            onclick={() => void toolsActions.saveFullAccess(request)}
          >
            {m.tools_full_access_confirm()}
          </Button>
        </AlertDialog.Footer>
      {:else}
        <AlertDialog.Header>
          <AlertDialog.Title>{m.tools_full_access_off_failed_title()}</AlertDialog.Title>
          <AlertDialog.Description>
            {request.failure === null ? '' : toolsFailureMessage(request.failure)}
          </AlertDialog.Description>
        </AlertDialog.Header>
        <AlertDialog.Footer>
          <AlertDialog.Cancel>{m.tools_full_access_close()}</AlertDialog.Cancel>
          <Button disabled={request.saving} onclick={() => void toolsActions.saveFullAccess(request)}>
            {m.tools_try_again()}
          </Button>
        </AlertDialog.Footer>
      {/if}
    {/if}
  </AlertDialog.Content>
</AlertDialog.Root>
