<script lang="ts">
  import { onMount } from 'svelte';

  import RestartToUpdateDialog from '#lib/components/app/updates/RestartToUpdateDialog.svelte';
  import UpdateNotice from '#lib/components/app/updates/UpdateNotice.svelte';
  import { updates } from '#lib/state/updates.svelte.js';

  // What updates show above every view (docs/features/updates.md): the notices
  // in the lower corner and the question before a restart. The subscription
  // starts here, so a version that becomes ready shows wherever the owner is.
  onMount(() => {
    updates.start();
  });
</script>

<UpdateNotice />
<RestartToUpdateDialog />
