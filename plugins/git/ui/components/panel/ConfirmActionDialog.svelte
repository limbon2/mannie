<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import CloudIcon from '@mannie/plugin-sdk/icons/cloud';
  import ShieldAlertIcon from '@mannie/plugin-sdk/icons/shield-alert';
  import TerminalIcon from '@mannie/plugin-sdk/icons/terminal';
  import TrashIcon from '@mannie/plugin-sdk/icons/trash';
  import TriangleAlertIcon from '@mannie/plugin-sdk/icons/triangle-alert';

  import type { OwnerActionEffect } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import type { PendingConfirmation } from '../../state/git-repository.svelte.js';
  import { formatActionEffect } from '../../utils/git-messages.js';

  // Server-enforced confirmation dialog for owner actions that run repository
  // hooks or repo-configured programs, discard work, rewrite published
  // commits, or delete branches/tags (ADR-0078).
  interface Props {
    readonly busy: boolean;
    readonly onCancel: () => void;
    readonly onConfirm: () => void;
    readonly pending: PendingConfirmation | null;
  }

  let { busy, onCancel, onConfirm, pending }: Props = $props();

  const open = $derived(pending !== null);

  function isDestructiveEffect(effect: OwnerActionEffect): boolean {
    const isDiscard = effect.kind === 'discards_work';
    const isDelete = effect.kind === 'deletes_ref';
    return isDiscard || isDelete;
  }

  function discardedItems(effect: OwnerActionEffect): readonly string[] {
    return effect.kind === 'discards_work' ? effect.items : [];
  }

  function handleOpenChange(next: boolean): void {
    if (!next) {
      onCancel();
    }
  }
</script>

<Dialog.Root {open} onOpenChange={handleOpenChange}>
  <Dialog.Content class="max-w-lg" data-git-confirm-dialog="" showCloseButton={false}>
    <Dialog.Header>
      <Dialog.Title class="flex items-center gap-2">
        <ShieldAlertIcon class="size-4 shrink-0 text-destructive" aria-hidden="true" />
        <span>{m.git_confirm_title()}</span>
      </Dialog.Title>
      <Dialog.Description>{m.git_confirm_description()}</Dialog.Description>
    </Dialog.Header>

    {#if pending !== null}
      <ul class="flex max-h-64 flex-col gap-2 overflow-y-auto py-1 text-sm">
        {#each pending.preview.effects as effect, index (index)}
          {const items = $derived(discardedItems(effect))}
          <li class="flex flex-col gap-1 rounded-md border bg-muted/30 p-2.5">
            <div class="flex items-start gap-2">
              {#if effect.kind === 'runs_folder_code'}
                <TerminalIcon class="mt-0.5 size-4 shrink-0 text-git-modified" aria-hidden="true" />
              {:else if isDestructiveEffect(effect)}
                <TrashIcon class="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden="true" />
              {:else if effect.kind === 'rewrites_published'}
                <TriangleAlertIcon class="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden="true" />
              {:else}
                <CloudIcon class="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
              {/if}
              <span>{formatActionEffect(effect)}</span>
            </div>

            {#if items.length > 0}
              <ul class="ms-6 flex flex-col gap-0.5 font-mono text-xs text-muted-foreground">
                {#each items as item (item)}
                  <li class="truncate" title={item}>{item}</li>
                {/each}
              </ul>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}

    <Dialog.Footer>
      <Button disabled={busy} variant="outline" onclick={onCancel}>{m.git_dialog_cancel()}</Button>
      <Button disabled={busy} variant="destructive" onclick={onConfirm}>{m.git_confirm_proceed()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
