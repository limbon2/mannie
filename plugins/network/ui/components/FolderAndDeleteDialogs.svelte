<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import { m } from '../generated/messages.js';
  import { networkDialogs } from '../state/network-dialogs.svelte.js';
  import { networkLibrary } from '../state/network.svelte.js';
  import type { NetworkFailure } from '../types/network.js';
  import { networkFailureMessage } from '../utils/network-messages.js';
  import { openNetwork, shownCollectionId, shownRequestId } from '../utils/network-places.js';

  let folderName = $state('');
  let folderBusy = $state(false);
  let folderFailure = $state<NetworkFailure | null>(null);

  let deleteBusy = $state(false);
  let deleteFailure = $state<NetworkFailure | null>(null);

  const folderOpen = $derived(networkDialogs.folderDialog !== null);
  const deleteOpen = $derived(networkDialogs.deleteDialog !== null);
  const deleteTitle = $derived.by(() => {
    const target = networkDialogs.deleteDialog;
    if (target === null) {
      return '';
    }
    switch (target.kind) {
      case 'collection': {
        return m.network_delete_collection_title({ name: target.name });
      }
      case 'folder': {
        return m.network_delete_folder_title({ name: target.name });
      }
      case 'request': {
        return m.network_delete_request_title({ name: target.name });
      }
      case 'environment': {
        return m.network_delete_environment_title({ name: target.name });
      }
      case 'clear_cookies': {
        return m.network_cookies_clear_confirm_title();
      }
    }
  });
  const deleteDescription = $derived.by(() => {
    const target = networkDialogs.deleteDialog;
    if (target === null) {
      return '';
    }
    switch (target.kind) {
      case 'collection': {
        return m.network_delete_collection_description();
      }
      case 'folder': {
        return m.network_delete_folder_description();
      }
      case 'request': {
        return m.network_delete_request_description();
      }
      case 'environment': {
        return m.network_delete_environment_description();
      }
      case 'clear_cookies': {
        return m.network_cookies_clear_confirm_description();
      }
    }
  });

  function handleFolderOpenChange(nextOpen: boolean): void {
    if (!nextOpen && !folderBusy) {
      networkDialogs.closeFolderDialog();
    }
  }

  function handleDeleteOpenChange(nextOpen: boolean): void {
    if (!nextOpen && !deleteBusy) {
      networkDialogs.closeDeleteDialog();
    }
  }

  async function submitFolder(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const dialog = networkDialogs.folderDialog;
    const hasName = folderName.trim() !== '';

    if (dialog === null) {
      return;
    }
    if (!hasName) {
      return;
    }
    folderBusy = true;
    folderFailure = null;
    const outcome =
      dialog.mode === 'create'
        ? await networkLibrary.createFolder({
            collectionId: dialog.collectionId,
            name: folderName,
            parentId: null,
          })
        : await networkLibrary.renameFolder(dialog.folder.id, folderName);

    folderBusy = false;
    if (outcome.ok) {
      networkDialogs.closeFolderDialog();
    } else {
      folderFailure = outcome.failure;
    }
  }

  function deletedCurrentSelection(target: NonNullable<typeof networkDialogs.deleteDialog>): boolean {
    if (target.kind === 'collection') {
      return shownCollectionId() === target.id;
    }
    if (target.kind === 'request') {
      return shownRequestId() === target.id;
    }
    return false;
  }

  async function executeDelete(): Promise<void> {
    const target = networkDialogs.deleteDialog;
    if (target === null) {
      return;
    }
    deleteBusy = true;
    deleteFailure = null;

    let outcome;
    switch (target.kind) {
      case 'collection': {
        outcome = await networkLibrary.deleteCollection(target.id);
        break;
      }
      case 'folder': {
        outcome = await networkLibrary.deleteFolder(target.id);
        break;
      }
      case 'request': {
        outcome = await networkLibrary.deleteRequest(target.id);
        break;
      }
      case 'environment': {
        outcome = await networkLibrary.deleteEnvironment(target.id);
        break;
      }
      case 'clear_cookies': {
        outcome = await networkLibrary.clearCookies();
        break;
      }
    }

    deleteBusy = false;
    if (!outcome.ok) {
      deleteFailure = outcome.failure;
      return;
    }

    networkDialogs.closeDeleteDialog();
    if (deletedCurrentSelection(target)) {
      await openNetwork({ replace: true });
    }
  }

  $effect(() => {
    const dialog = networkDialogs.folderDialog;
    if (dialog === null) {
      return;
    }
    folderName = dialog.mode === 'rename' ? dialog.folder.name : '';
    folderBusy = false;
    folderFailure = null;
  });

  $effect(() => {
    if (networkDialogs.deleteDialog !== null) {
      deleteBusy = false;
      deleteFailure = null;
    }
  });
</script>

<Dialog.Root open={folderOpen} onOpenChange={handleFolderOpenChange}>
  <Dialog.Content class="sm:max-w-md" showCloseButton={false}>
    <form class="flex flex-col gap-4" onsubmit={(event: SubmitEvent) => void submitFolder(event)}>
      <Dialog.Header>
        <Dialog.Title>
          {networkDialogs.folderDialog?.mode === 'rename'
            ? m.network_folder_rename_title()
            : m.network_folder_create_title()}
        </Dialog.Title>
        <Dialog.Description>{m.network_folder_description()}</Dialog.Description>
      </Dialog.Header>

      {#if folderFailure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Description>{networkFailureMessage(folderFailure)}</Alert.Description>
        </Alert.Root>
      {/if}

      <Field.Field>
        <Field.Label>{m.network_folder_name_label()}</Field.Label>
        <Input
          disabled={folderBusy}
          placeholder={m.network_folder_name_placeholder()}
          required
          bind:value={folderName}
        />
      </Field.Field>

      <Dialog.Footer>
        <Button
          disabled={folderBusy}
          type="button"
          variant="outline"
          onclick={() => networkDialogs.closeFolderDialog()}
        >
          {m.network_cancel()}
        </Button>
        <Button disabled={folderBusy || folderName.trim() === ''} type="submit">
          {networkDialogs.folderDialog?.mode === 'rename' ? m.network_rename() : m.network_create()}
        </Button>
      </Dialog.Footer>
    </form>
  </Dialog.Content>
</Dialog.Root>

<AlertDialog.Root open={deleteOpen} onOpenChange={handleDeleteOpenChange}>
  <AlertDialog.Content>
    <AlertDialog.Header>
      <AlertDialog.Title>{deleteTitle}</AlertDialog.Title>
      <AlertDialog.Description>{deleteDescription}</AlertDialog.Description>
    </AlertDialog.Header>

    {#if deleteFailure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Description>{networkFailureMessage(deleteFailure)}</Alert.Description>
      </Alert.Root>
    {/if}

    <AlertDialog.Footer>
      <AlertDialog.Cancel disabled={deleteBusy}>{m.network_cancel()}</AlertDialog.Cancel>
      <AlertDialog.Action disabled={deleteBusy} variant="destructive" onclick={() => void executeDelete()}>
        {networkDialogs.deleteDialog?.kind === 'clear_cookies' ? m.network_clear_all() : m.network_delete()}
      </AlertDialog.Action>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>
