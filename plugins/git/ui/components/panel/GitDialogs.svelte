<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Checkbox } from '@mannie/plugin-sdk/components/checkbox';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import { Label } from '@mannie/plugin-sdk/components/label';
  import { Textarea } from '@mannie/plugin-sdk/components/textarea';

  import type { OwnerGitAction } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import type { ActiveGitDialog } from '../../state/git-repository.svelte.js';

  // Modal dialogs for creating branches, renaming branches, creating tags,
  // and pushing stash entries. Passes `showCloseButton={false}` per PATCHES.md.
  interface Props {
    readonly busy: boolean;
    readonly dialog: ActiveGitDialog | null;
    readonly onClose: () => void;
    readonly onSubmit: (action: OwnerGitAction) => void;
  }

  let { busy, dialog, onClose, onSubmit }: Props = $props();

  let branchName = $state('');
  let branchStart = $state('');
  let branchSwitch = $state(true);
  let renameTarget = $state('');
  let tagName = $state('');
  let tagTarget = $state('');
  let tagMessage = $state('');
  let stashMessage = $state('');
  let stashUntracked = $state(false);

  const open = $derived(dialog !== null);

  function handleOpenChange(next: boolean): void {
    if (!next) {
      onClose();
    }
  }

  function submitForm(event: SubmitEvent): void {
    event.preventDefault();
    if (dialog === null) {
      return;
    }
    if (dialog.kind === 'create_branch') {
      const name = branchName.trim();
      if (name.length === 0) {
        return;
      }
      const start = branchStart.trim();
      onSubmit({
        kind: 'create_branch',
        name,
        start: start.length > 0 ? start : null,
        switchTo: branchSwitch,
      });
      onClose();
    } else if (dialog.kind === 'rename_branch') {
      const nextName = renameTarget.trim();
      const isEmpty = nextName.length === 0;
      const isUnchanged = nextName === dialog.name;
      if (isEmpty || isUnchanged) {
        return;
      }
      onSubmit({
        kind: 'rename_branch',
        name: dialog.name,
        newName: nextName,
      });
      onClose();
    } else if (dialog.kind === 'create_tag') {
      const name = tagName.trim();
      if (name.length === 0) {
        return;
      }
      const target = tagTarget.trim();
      const message = tagMessage.trim();
      onSubmit({
        kind: 'create_tag',
        message: message.length > 0 ? message : null,
        name,
        target: target.length > 0 ? target : null,
      });
      onClose();
    } else {
      const message = stashMessage.trim();
      onSubmit({
        includeUntracked: stashUntracked,
        kind: 'stash_push',
        message: message.length > 0 ? message : null,
      });
      onClose();
    }
  }

  $effect(() => {
    if (dialog === null) {
      return;
    }
    if (dialog.kind === 'create_branch') {
      branchName = '';
      branchStart = dialog.start;
      branchSwitch = true;
    } else if (dialog.kind === 'rename_branch') {
      renameTarget = dialog.name;
    } else if (dialog.kind === 'create_tag') {
      tagName = '';
      tagTarget = dialog.target;
      tagMessage = '';
    } else {
      stashMessage = '';
      stashUntracked = false;
    }
  });
</script>

<Dialog.Root {open} onOpenChange={handleOpenChange}>
  <Dialog.Content class="max-w-md" showCloseButton={false}>
    {#if dialog !== null}
      <form class="flex flex-col gap-4" onsubmit={submitForm}>
        <Dialog.Header>
          <Dialog.Title>
            {#if dialog.kind === 'create_branch'}
              {m.git_dialog_create_branch_title()}
            {:else if dialog.kind === 'rename_branch'}
              {m.git_dialog_rename_branch_title({ name: dialog.name })}
            {:else if dialog.kind === 'create_tag'}
              {m.git_dialog_create_tag_title()}
            {:else}
              {m.git_dialog_stash_title()}
            {/if}
          </Dialog.Title>
        </Dialog.Header>

        {#if dialog.kind === 'create_branch'}
          <div class="flex flex-col gap-3">
            <div class="flex flex-col gap-1.5">
              <Label for="git-branch-name">{m.git_dialog_branch_name()}</Label>
              <Input id="git-branch-name" required bind:value={branchName} />
            </div>

            <div class="flex flex-col gap-1.5">
              <Label for="git-branch-start">{m.git_dialog_branch_start()}</Label>
              <Input id="git-branch-start" placeholder={m.git_placeholder_head()} bind:value={branchStart} />
            </div>

            <label class="flex items-center gap-2 text-sm">
              <Checkbox bind:checked={branchSwitch} />
              <span>{m.git_dialog_branch_switch()}</span>
            </label>
          </div>
        {:else if dialog.kind === 'rename_branch'}
          <div class="flex flex-col gap-1.5">
            <Label for="git-rename-branch">{m.git_dialog_new_branch_name()}</Label>
            <Input id="git-rename-branch" required bind:value={renameTarget} />
          </div>
        {:else if dialog.kind === 'create_tag'}
          <div class="flex flex-col gap-3">
            <div class="flex flex-col gap-1.5">
              <Label for="git-tag-name">{m.git_dialog_tag_name()}</Label>
              <Input id="git-tag-name" required bind:value={tagName} />
            </div>

            <div class="flex flex-col gap-1.5">
              <Label for="git-tag-target">{m.git_dialog_tag_target()}</Label>
              <Input id="git-tag-target" placeholder={m.git_placeholder_head()} bind:value={tagTarget} />
            </div>

            <div class="flex flex-col gap-1.5">
              <Label for="git-tag-message">{m.git_dialog_tag_message()}</Label>
              <Textarea id="git-tag-message" rows={3} bind:value={tagMessage} />
            </div>
          </div>
        {:else}
          <div class="flex flex-col gap-3">
            <div class="flex flex-col gap-1.5">
              <Label for="git-stash-message">{m.git_dialog_stash_message()}</Label>
              <Input id="git-stash-message" bind:value={stashMessage} />
            </div>

            <label class="flex items-center gap-2 text-sm">
              <Checkbox bind:checked={stashUntracked} />
              <span>{m.git_dialog_stash_untracked()}</span>
            </label>
          </div>
        {/if}

        <Dialog.Footer>
          <Button disabled={busy} type="button" variant="outline" onclick={onClose}>{m.git_dialog_cancel()}</Button>
          <Button disabled={busy} type="submit">{m.git_dialog_confirm()}</Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
