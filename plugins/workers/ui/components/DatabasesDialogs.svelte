<script lang="ts">
  import AddDatabaseDialog from './AddDatabaseDialog.svelte';
  import RemoveDatabaseDialog from './RemoveDatabaseDialog.svelte';
  import RenameDatabaseDialog from './RenameDatabaseDialog.svelte';

  // The dialogs of Databases (plugins/workers/docs/databases.md), mounted
  // once as the feature's overlay, so the panel, the page and the command
  // palette open the same ones.
</script>

<AddDatabaseDialog />
<RenameDatabaseDialog />
<RemoveDatabaseDialog />
