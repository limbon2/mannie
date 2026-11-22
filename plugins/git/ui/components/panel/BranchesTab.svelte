<script lang="ts">
  import * as Resizable from '@mannie/plugin-sdk/components/resizable';

  import { m } from '../../generated/messages.js';
  import type { RepositoryPanel } from '../../state/git-repository.svelte.js';

  import BranchesListPane from './BranchesListPane.svelte';
  import StashInspector from './StashInspector.svelte';

  // Full Refs Explorer tab: responsive split view pairing BranchesListPane
  // with StashInspector when a stash entry is chosen.
  interface Props {
    readonly panel: RepositoryPanel;
    readonly wide: boolean;
  }

  let { panel, wide }: Props = $props();
</script>

{#if panel.chosenStashIndex !== null}
  <Resizable.PaneGroup class="min-h-0 flex-1" direction={wide ? 'horizontal' : 'vertical'}>
    <Resizable.Pane defaultSize={48} minSize={24}>
      <BranchesListPane {panel} />
    </Resizable.Pane>
    <Resizable.Handle
      aria-label={m.git_panel_split_resize()}
      aria-orientation={wide ? 'vertical' : 'horizontal'}
      withHandle
    />
    <Resizable.Pane defaultSize={52} minSize={25}>
      <StashInspector {panel} />
    </Resizable.Pane>
  </Resizable.PaneGroup>
{:else}
  <BranchesListPane {panel} />
{/if}
