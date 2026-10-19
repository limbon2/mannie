<script lang="ts">
  import * as Sidebar from '@mannie/plugin-sdk/components/sidebar';
  import NotepadTextIcon from '@mannie/plugin-sdk/icons/notepad-text';
  import { openSettings } from '@mannie/plugin-sdk/navigation';

  import { m } from '../generated/messages.js';
  import { noteList, SETTINGS_SECTION } from '../state/notes.svelte.js';

  // The plugin's group of the sidebar (ADR-0020, a sidebar panel):
  // the notes that it kept since it turned on, each of which opens the
  // plugin's settings section. Without notes the group is not shown.
</script>

{#if noteList.notes.length > 0}
  <Sidebar.Group class="group-data-[collapsible=icon]:hidden" data-example-panel="">
    <Sidebar.GroupLabel>{m.example_panel_title()}</Sidebar.GroupLabel>
    <Sidebar.GroupContent>
      <Sidebar.Menu>
        {#each noteList.notes as note (note.id)}
          <Sidebar.MenuItem>
            <Sidebar.MenuButton data-example-panel-note={note.id} onclick={() => void openSettings(SETTINGS_SECTION)}>
              <NotepadTextIcon />
              <span class="min-w-0 truncate" data-content dir="auto">{note.text}</span>
            </Sidebar.MenuButton>
          </Sidebar.MenuItem>
        {/each}
      </Sidebar.Menu>
    </Sidebar.GroupContent>
  </Sidebar.Group>
{/if}
