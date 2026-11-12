<script lang="ts">
  import type { UpdateActionFailure } from '#lib/types/updates.js';
  import { actionFailureTitle, updatesFailureMessage } from '#lib/utils/updates-messages.js';

  // Why an action that the owner chose in a notice failed. It shows in that
  // notice, whose live region announces it.
  let { failed }: { failed: UpdateActionFailure } = $props();
</script>

<div class="flex flex-col gap-0.5 text-destructive" data-update-notice-failed={failed.action}>
  <p class="font-medium">{actionFailureTitle(failed.action)}</p>
  <p>{updatesFailureMessage(failed.failure)}</p>
</div>
