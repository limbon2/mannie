<script lang="ts">
  import * as Empty from '@mannie/plugin-sdk/components/empty';

  import { m } from '../generated/messages.js';
  import { projectSelection } from '../state/project-selection.svelte.js';

  const shown = $derived(projectSelection.shown());
</script>

<Empty.Root class="flex-1" data-designer-blank="">
  <Empty.Description>
    {shown === undefined ? m.designer_blank_state() : m.designer_greeting({ name: shown.name })}
  </Empty.Description>
</Empty.Root>
