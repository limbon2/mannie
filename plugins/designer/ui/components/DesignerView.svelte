<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Empty from '@mannie/plugin-sdk/components/empty';

  import { m } from '../generated/messages.js';
  import { projectSelection } from '../state/project-selection.svelte.js';
  import { projectLibrary } from '../state/projects.svelte.js';
  import { designerFailureMessage } from '../utils/designer-messages.js';

  import DesignerViewport from './DesignerViewport.svelte';

  const shown = $derived(projectSelection.shown());
</script>

{#if shown !== undefined}
  {#key shown.id}
    <DesignerViewport project={shown} />
  {/key}
{:else}
  <Empty.Root class="flex-1" data-designer-blank="">
    {#if projectLibrary.status === 'loading'}
      <Empty.Description role="status">{m.designer_projects_loading()}</Empty.Description>
    {:else if projectLibrary.failure !== null}
      <Empty.Description role="alert">{designerFailureMessage(projectLibrary.failure)}</Empty.Description>
      <Button size="sm" variant="outline" onclick={() => projectLibrary.retry()}>{m.designer_try_again()}</Button>
    {:else}
      <Empty.Description>{m.designer_blank_state()}</Empty.Description>
    {/if}
  </Empty.Root>
{/if}
