<script lang="ts">
  import * as Sidebar from '@mannie/plugin-sdk/components/sidebar';
  import FolderIcon from '@mannie/plugin-sdk/icons/folder';
  import FolderOpenIcon from '@mannie/plugin-sdk/icons/folder-open';

  import type { ProjectView } from '../generated/bindings.js';
  import { projectSelection } from '../state/project-selection.svelte.js';

  // One project in the sidebar (plugins/designer/docs/README.md, the sidebar):
  // its name, which the owner gave and so is content, and the mark that the
  // view shows it. Activating the row switches the view to this project.
  let { project }: { project: ProjectView } = $props();

  const isShown = $derived(projectSelection.shown()?.id === project.id);
</script>

<Sidebar.MenuItem>
  <Sidebar.MenuButton
    aria-current={isShown ? 'true' : undefined}
    data-designer-project={project.id}
    isActive={isShown}
    onclick={() => projectSelection.select(project.id)}
  >
    {#if isShown}
      <FolderOpenIcon />
    {:else}
      <FolderIcon />
    {/if}
    <span class="truncate" data-content dir="auto">{project.name}</span>
  </Sidebar.MenuButton>
</Sidebar.MenuItem>
