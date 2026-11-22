<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Collapsible from '@mannie/plugin-sdk/components/collapsible';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import ChevronRightIcon from '@mannie/plugin-sdk/icons/chevron-right';
  import FolderIcon from '@mannie/plugin-sdk/icons/folder';
  import MinusIcon from '@mannie/plugin-sdk/icons/minus';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';

  import type { Entry, FileStat } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import type { RepositoryPanel } from '../../state/git-repository.svelte.js';
  import type { ChangeGroupKind } from '../../types/git.js';
  import { collectTreePaths, type FileTreeNode } from '../../utils/git-tree.js';

  import ChangesFileRow from './ChangesFileRow.svelte';
  import ChangesFileTree from './ChangesFileTree.svelte';

  // Recursive compressed directory tree for one change group in ChangesFileList.
  interface Props {
    readonly groupKind: ChangeGroupKind;
    readonly nodes: readonly FileTreeNode<Entry>[];
    readonly panel: RepositoryPanel;
    readonly statFor: (path: string, groupKind: ChangeGroupKind) => FileStat | undefined;
  }

  let { groupKind, nodes, panel, statFor }: Props = $props();

  function stageFolder(node: FileTreeNode<Entry>, event: MouseEvent): void {
    event.stopPropagation();
    void panel.runAction({ all: false, kind: 'stage_files', paths: collectTreePaths(node) });
  }

  function unstageFolder(node: FileTreeNode<Entry>, event: MouseEvent): void {
    event.stopPropagation();
    void panel.runAction({ all: false, kind: 'unstage_files', paths: collectTreePaths(node) });
  }
</script>

<ul class="flex flex-col gap-0.5">
  {#each nodes as node (node.path)}
    {#if node.kind === 'folder'}
      <li>
        <Collapsible.Root class="group/folder" open={true}>
          <div class="group/row flex min-w-0 items-center gap-1 rounded-sm pe-1 hover:bg-muted/50">
            <Collapsible.Trigger
              class="flex min-w-0 flex-1 items-center gap-1 px-1.5 py-0.5 text-start text-xs text-muted-foreground"
            >
              <ChevronRightIcon
                class="size-3 shrink-0 transition-transform group-data-[state=open]/folder:rotate-90"
                aria-hidden="true"
              />
              <FolderIcon class="size-3 shrink-0" aria-hidden="true" />
              <span class="min-w-0 truncate font-mono" title={node.path}>{node.label}</span>
              <span class="ms-1 shrink-0 text-[10px] tabular-nums">{formatCount(node.fileCount)}</span>
            </Collapsible.Trigger>

            {#if groupKind !== 'conflicted'}
              <div
                class="flex shrink-0 items-center opacity-0 transition-opacity group-focus-within/row:opacity-100 group-hover/row:opacity-100"
              >
                {#if groupKind === 'staged'}
                  <Button
                    class="size-5"
                    aria-label={m.git_changes_unstage_folder()}
                    disabled={panel.runningAction}
                    size="icon"
                    title={m.git_changes_unstage_folder()}
                    variant="ghost"
                    onclick={(event: MouseEvent) => unstageFolder(node, event)}
                  >
                    <MinusIcon class="size-3" aria-hidden="true" />
                  </Button>
                {:else}
                  <Button
                    class="size-5"
                    aria-label={m.git_changes_stage_folder()}
                    disabled={panel.runningAction}
                    size="icon"
                    title={m.git_changes_stage_folder()}
                    variant="ghost"
                    onclick={(event: MouseEvent) => stageFolder(node, event)}
                  >
                    <PlusIcon class="size-3" aria-hidden="true" />
                  </Button>
                {/if}
              </div>
            {/if}
          </div>

          <Collapsible.Content class="ps-3">
            <ChangesFileTree {groupKind} nodes={node.children} {panel} {statFor} />
          </Collapsible.Content>
        </Collapsible.Root>
      </li>
    {:else}
      <li>
        <ChangesFileRow
          entry={node.item}
          {groupKind}
          {panel}
          showDirectory={false}
          stat={statFor(node.item.path, groupKind)}
        />
      </li>
    {/if}
  {/each}
</ul>
