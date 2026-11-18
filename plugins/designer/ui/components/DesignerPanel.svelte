<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Sidebar from '@mannie/plugin-sdk/components/sidebar';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';

  import { m } from '../generated/messages.js';
  import { projectActions } from '../state/project-actions.svelte.js';
  import { projectLibrary } from '../state/projects.svelte.js';
  import { designerFailureMessage } from '../utils/designer-messages.js';

  import ProjectRow from './ProjectRow.svelte';

  // The plugin's group of the sidebar (ADR-0045, decision 3; plugins/designer/
  // docs/README.md, the sidebar), which shows while the activity bar selects
  // Designer: "Projects" with the plus that names a new one, and the projects,
  // the newest first, each of which switches the view to it. While the list is
  // not known it says so, and when it cannot be read it says why, with "Try
  // again".
</script>

<Sidebar.Group class="group-data-[collapsible=icon]:hidden" data-designer-projects="">
  <Sidebar.GroupLabel>{m.designer_projects_title()}</Sidebar.GroupLabel>
  <Sidebar.GroupAction
    data-designer-projects-add=""
    title={m.designer_projects_add()}
    onclick={() => projectActions.startCreate()}
  >
    <PlusIcon />
    <span class="sr-only">{m.designer_projects_add()}</span>
  </Sidebar.GroupAction>
  <Sidebar.GroupContent>
    {#if projectLibrary.status === 'loading'}
      <p class="px-2 text-xs text-muted-foreground" data-designer-projects-loading="">
        {m.designer_projects_loading()}
      </p>
    {:else if projectLibrary.status === 'error'}
      <div class="flex flex-col items-start gap-2 px-2 text-xs text-muted-foreground" role="alert">
        <p>
          {m.designer_projects_failed()}
          {projectLibrary.failure === null ? '' : designerFailureMessage(projectLibrary.failure)}
        </p>
        <Button size="sm" variant="outline" onclick={() => projectLibrary.retry()}>{m.designer_try_again()}</Button>
      </div>
    {:else if projectLibrary.projects.length === 0}
      <p class="px-2 text-xs text-muted-foreground" data-designer-projects-empty="">{m.designer_projects_empty()}</p>
    {:else}
      <Sidebar.Menu>
        {#each projectLibrary.projects as project (project.id)}
          <ProjectRow {project} />
        {/each}
      </Sidebar.Menu>
    {/if}
  </Sidebar.GroupContent>
</Sidebar.Group>
