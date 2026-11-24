<script lang="ts">
  import * as Resizable from '@mannie/plugin-sdk/components/resizable';

  import { m } from '../../generated/messages.js';
  import type { RepositoryPanel } from '../../state/git-repository.svelte.js';

  import CommitInspector from './CommitInspector.svelte';
  import GraphListPane from './GraphListPane.svelte';

  // Topological Commit Graph tab: responsive split layout pairing the SVG
  // commit graph list with CommitInspector (side-by-side when wide, stacked
  // split pane when narrow).
  interface Props {
    readonly panel: RepositoryPanel;
    readonly wide: boolean;
  }

  let { panel, wide }: Props = $props();
</script>

{#if panel.chosenCommit !== null}
  <Resizable.PaneGroup class="min-h-0 flex-1" direction={wide ? 'horizontal' : 'vertical'}>
    <Resizable.Pane defaultSize={wide ? 48 : 44} minSize={24}>
      <GraphListPane {panel} />
    </Resizable.Pane>
    <Resizable.Handle
      aria-label={m.git_panel_split_resize()}
      aria-orientation={wide ? 'vertical' : 'horizontal'}
      withHandle
    />
    <Resizable.Pane defaultSize={wide ? 52 : 56} minSize={25}>
      <CommitInspector {panel} onClose={() => panel.closeCommit()} />
    </Resizable.Pane>
  </Resizable.PaneGroup>
{:else}
  <GraphListPane {panel} />
{/if}
