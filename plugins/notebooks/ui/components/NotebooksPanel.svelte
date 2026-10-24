<script lang="ts">
  import * as Sidebar from '@mannie/plugin-sdk/components/sidebar';
  import { followConversations } from '@mannie/plugin-sdk/conversations';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import { onMount } from 'svelte';

  import { m } from '../generated/messages.js';
  import { notebookActions } from '../state/notebook-actions.svelte.js';
  import { notebookLibrary } from '../state/notebooks.svelte.js';

  import NotebookPanelRow from './NotebookPanelRow.svelte';

  // The plugin's group of the sidebar (ADR-0045, decision 3; plugins/notebooks/
  // docs/README.md, interface), which shows while the activity bar selects
  // Notebooks: "+", which starts a new notebook, and the notebooks, the newest
  // first, each with its sources and the chats that use it.
  onMount(() => {
    followConversations();
  });
</script>

<Sidebar.Group class="group-data-[collapsible=icon]:hidden" data-notebooks-panel="">
  <Sidebar.GroupLabel>{m.notebooks_panel_title()}</Sidebar.GroupLabel>
  <Sidebar.GroupAction
    data-notebooks-panel-new=""
    title={m.notebooks_new_notebook()}
    onclick={() => notebookActions.startCreate()}
  >
    <PlusIcon />
    <span class="sr-only">{m.notebooks_new_notebook()}</span>
  </Sidebar.GroupAction>
  <Sidebar.GroupContent>
    {#if notebookLibrary.status === 'loading'}
      <Sidebar.Menu aria-busy="true">
        {#each [0, 1] as placeholder (placeholder)}
          <Sidebar.MenuItem>
            <Sidebar.MenuSkeleton showIcon />
          </Sidebar.MenuItem>
        {/each}
      </Sidebar.Menu>
    {:else if notebookLibrary.status === 'error'}
      <p class="px-2 text-xs text-destructive">{m.notebooks_panel_failed()}</p>
    {:else if notebookLibrary.notebooks.length === 0}
      <p class="px-2 text-xs text-muted-foreground">{m.notebooks_panel_empty()}</p>
    {:else}
      <Sidebar.Menu>
        {#each notebookLibrary.notebooks as notebook (notebook.id)}
          <NotebookPanelRow {notebook} />
        {/each}
      </Sidebar.Menu>
    {/if}
  </Sidebar.GroupContent>
</Sidebar.Group>
