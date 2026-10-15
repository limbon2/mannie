<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';

  import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { projectActions } from '#lib/state/project-actions.svelte.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';

  // Confirms the removal of the project that `projectActions.startRemove`
  // opened it for (docs/features/chat/projects.md, renaming and removing). It
  // says how many conversations go with the project, and its button says it
  // too. When the project has another number of conversations by then, the
  // core removes nothing, and the dialog shows the new number to confirm.
  const request = $derived(projectActions.removing);
</script>

<AlertDialog.Root bind:open={projectActions.removeOpen}>
  <AlertDialog.Content>
    {#if request !== null}
      <AlertDialog.Header>
        <AlertDialog.Title>{m.chat_project_remove_title()}</AlertDialog.Title>
        <AlertDialog.Description>
          {request.conversations === 0
            ? m.chat_project_remove_description_empty({ project: request.project.name })
            : m.chat_project_remove_description({ count: request.conversations, project: request.project.name })}
        </AlertDialog.Description>
      </AlertDialog.Header>
      {#if request.failure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Description>{chatFailureMessage(request.failure)}</Alert.Description>
        </Alert.Root>
      {/if}
      <AlertDialog.Footer>
        <AlertDialog.Cancel>{m.chat_cancel()}</AlertDialog.Cancel>
        <Button
          disabled={request.removing}
          variant="destructive"
          onclick={() => void projectActions.confirmRemove(request)}
        >
          {request.conversations === 0
            ? m.chat_project_remove_confirm_empty()
            : m.chat_project_remove_confirm({ count: request.conversations })}
        </Button>
      </AlertDialog.Footer>
    {/if}
  </AlertDialog.Content>
</AlertDialog.Root>
