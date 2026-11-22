<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import CheckIcon from '@mannie/plugin-sdk/icons/check';
  import TrashIcon from '@mannie/plugin-sdk/icons/trash';

  import type { Conflict, ConflictFileView, Side } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import type { RepositoryPanel } from '../../state/git-repository.svelte.js';
  import { conflictWords, gitFailureMessage } from '../../utils/git-messages.js';
  import ConflictHunkView from '../cards/ConflictHunkView.svelte';

  // Interactive Conflicts tab: whole-file resolution (Current, Incoming,
  // Base, Both, Keep/Delete for delete conflicts, Mark resolved) and
  // per-hunk resolution choices.
  interface Props {
    readonly panel: RepositoryPanel;
  }

  let { panel }: Props = $props();

  let hunkChoices = $state<Record<string, Partial<Record<number, Side>>>>({});

  const view = $derived(panel.conflicts.value);
  const isLoadingInitial = $derived(panel.conflicts.status === 'loading' && view === null);
  const loadFailure = $derived(panel.conflicts.status === 'error' ? panel.conflicts.failure : null);
  const conflictFiles = $derived(view?.files ?? []);

  function isDeleteConflict(conflict: Conflict): boolean {
    const isDeletedCurrent = conflict === 'deleted_current';
    const isDeletedIncoming = conflict === 'deleted_incoming';
    const isBothDeleted = conflict === 'both_deleted';
    return isDeletedCurrent || isDeletedIncoming || isBothDeleted;
  }

  function resolveWhole(path: string, take: Side): void {
    void panel.runAction({
      deleted: null,
      edited: false,
      hunks: null,
      kind: 'resolve_conflict',
      path,
      take,
    });
  }

  function resolveDelete(path: string, deleted: boolean): void {
    void panel.runAction({
      deleted,
      edited: false,
      hunks: null,
      kind: 'resolve_conflict',
      path,
      take: null,
    });
  }

  function markEditedResolved(path: string): void {
    void panel.runAction({
      deleted: null,
      edited: true,
      hunks: null,
      kind: 'resolve_conflict',
      path,
      take: null,
    });
  }

  function chooseHunkSide(path: string, number: number, side: Side): void {
    const existing = hunkChoices[path] ?? {};
    hunkChoices = {
      ...hunkChoices,
      [path]: { ...existing, [number]: side },
    };
  }

  function selectedHunkSide(path: string, number: number): Side | null {
    const forPath = hunkChoices[path] as Partial<Record<number, Side>> | undefined;
    return forPath?.[number] ?? null;
  }

  function hasAllHunkChoices(item: ConflictFileView): boolean {
    if (item.hunks.length === 0) {
      return false;
    }
    return item.hunks.every((hunk) => selectedHunkSide(item.file.path, hunk.number) !== null);
  }

  function applyHunkChoices(item: ConflictFileView): void {
    const hunks = item.hunks.map((hunk) => ({
      number: hunk.number,
      take: selectedHunkSide(item.file.path, hunk.number),
      text: null,
    }));
    void panel.runAction({
      deleted: null,
      edited: false,
      hunks,
      kind: 'resolve_conflict',
      path: item.file.path,
      take: null,
    });
  }
</script>

<div class="flex h-full min-h-0 flex-col gap-3 overflow-y-auto p-2.5">
  {#if isLoadingInitial}
    <div class="flex items-center gap-2 py-4 text-xs text-muted-foreground" aria-busy="true">
      <Spinner aria-label={m.git_panel_loading()} />
      <span>{m.git_panel_loading()}</span>
    </div>
  {:else if loadFailure !== null}
    <Alert.Root variant="destructive">
      <Alert.Description class="flex items-center justify-between gap-2 text-xs">
        <span>{gitFailureMessage(loadFailure)}</span>
        <Button size="sm" variant="outline" onclick={() => panel.readConflicts()}>{m.git_panel_try_again()}</Button>
      </Alert.Description>
    </Alert.Root>
  {:else if conflictFiles.length === 0}
    <p class="py-6 text-center text-xs text-muted-foreground">{m.git_panel_no_conflicts()}</p>
  {:else}
    {#each conflictFiles as item (item.file.path)}
      {const deleteKind = $derived(isDeleteConflict(item.file.conflict))}
      {const allPicked = $derived(hasAllHunkChoices(item))}
      <section class="flex flex-col gap-2 rounded-md border p-2.5">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <div class="flex min-w-0 items-center gap-2">
            <span class="min-w-0 truncate font-mono text-xs font-medium" title={item.file.path}>
              {item.file.path}
            </span>
            <Badge class="shrink-0 text-[11px]" variant="outline">{conflictWords(item.file.conflict)}</Badge>
          </div>

          <div class="flex flex-wrap items-center gap-1">
            {#if deleteKind}
              <Button
                class="h-6 px-2 text-xs"
                disabled={panel.runningAction}
                size="sm"
                variant="outline"
                onclick={() => resolveDelete(item.file.path, false)}
              >
                <CheckIcon class="size-3" aria-hidden="true" />
                <span>{m.git_conflicts_keep_file()}</span>
              </Button>
              <Button
                class="h-6 px-2 text-xs"
                disabled={panel.runningAction}
                size="sm"
                variant="destructive"
                onclick={() => resolveDelete(item.file.path, true)}
              >
                <TrashIcon class="size-3" aria-hidden="true" />
                <span>{m.git_conflicts_delete_file()}</span>
              </Button>
            {:else}
              <Button
                class="h-6 px-2 text-xs"
                disabled={panel.runningAction}
                size="sm"
                variant="outline"
                onclick={() => resolveWhole(item.file.path, 'current')}
              >
                {m.git_conflicts_accept_current()}
              </Button>
              <Button
                class="h-6 px-2 text-xs"
                disabled={panel.runningAction}
                size="sm"
                variant="outline"
                onclick={() => resolveWhole(item.file.path, 'incoming')}
              >
                {m.git_conflicts_accept_incoming()}
              </Button>
              <Button
                class="h-6 px-2 text-xs"
                disabled={panel.runningAction}
                size="sm"
                variant="outline"
                onclick={() => resolveWhole(item.file.path, 'both')}
              >
                {m.git_conflicts_accept_both()}
              </Button>
              <Button
                class="h-6 px-2 text-xs"
                disabled={panel.runningAction}
                size="sm"
                variant="ghost"
                onclick={() => markEditedResolved(item.file.path)}
              >
                {m.git_conflicts_mark_resolved()}
              </Button>
            {/if}
          </div>
        </div>

        {#if item.hunks.length > 0}
          <div class="flex flex-col gap-2">
            {#each item.hunks as hunk (hunk.number)}
              <ConflictHunkView
                busy={panel.runningAction}
                {hunk}
                selected={selectedHunkSide(item.file.path, hunk.number)}
                onChoose={(side: Side) => chooseHunkSide(item.file.path, hunk.number, side)}
              />
            {/each}

            <div class="flex items-center justify-end gap-2">
              {#if !allPicked}
                <span class="text-xs text-muted-foreground">{m.git_conflicts_choose_all_hunks()}</span>
              {/if}
              <Button disabled={panel.runningAction || !allPicked} size="sm" onclick={() => applyHunkChoices(item)}>
                {m.git_conflicts_apply_hunks()}
              </Button>
            </div>
          </div>
        {/if}
      </section>
    {/each}
  {/if}
</div>
