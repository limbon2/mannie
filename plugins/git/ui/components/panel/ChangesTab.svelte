<script lang="ts">
  import * as Resizable from '@mannie/plugin-sdk/components/resizable';

  import type { RepositoryOverview } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import type { RepositoryPanel } from '../../state/git-repository.svelte.js';

  import ChangesDiffPane from './ChangesDiffPane.svelte';
  import ChangesFileList from './ChangesFileList.svelte';
  import CommitBox from './CommitBox.svelte';

  // Working-tree Changes tab: responsive master-detail split combining the
  // CommitBox, filterable tree/list file browser, and diff viewer pane.
  interface Props {
    readonly overview: RepositoryOverview;
    readonly panel: RepositoryPanel;
    readonly wide: boolean;
  }

  let { overview, panel, wide }: Props = $props();
</script>

{#if wide}
  <Resizable.PaneGroup class="min-h-0 flex-1" direction="horizontal">
    <Resizable.Pane defaultSize={36} minSize={24}>
      <div class="flex h-full min-h-0 flex-col overflow-hidden">
        <CommitBox {overview} {panel} />
        <ChangesFileList {overview} {panel} />
      </div>
    </Resizable.Pane>
    <Resizable.Handle aria-label={m.git_panel_split_resize()} aria-orientation="vertical" withHandle />
    <Resizable.Pane defaultSize={64} minSize={30}>
      <ChangesDiffPane {overview} {panel} {wide} />
    </Resizable.Pane>
  </Resizable.PaneGroup>
{:else if panel.file !== null}
  <ChangesDiffPane {overview} {panel} {wide} />
{:else}
  <div class="flex h-full min-h-0 flex-col overflow-hidden">
    <CommitBox {overview} {panel} />
    <ChangesFileList {overview} {panel} />
  </div>
{/if}
