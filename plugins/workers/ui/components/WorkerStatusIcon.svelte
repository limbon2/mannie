<script lang="ts">
  import CircleArrowUpIcon from '@mannie/plugin-sdk/icons/circle-arrow-up';
  import CircleCheckIcon from '@mannie/plugin-sdk/icons/circle-check';
  import KeyRoundIcon from '@mannie/plugin-sdk/icons/key-round';
  import LoaderCircleIcon from '@mannie/plugin-sdk/icons/loader-circle';
  import UnlinkIcon from '@mannie/plugin-sdk/icons/unlink';
  import WifiOffIcon from '@mannie/plugin-sdk/icons/wifi-off';

  import type { WorkerStatus } from '../generated/bindings.js';
  import { needsAttention, workerStatusLabel } from '../utils/workers-messages.js';

  // A worker's status as an icon, with its name for screen readers
  // (plugins/workers/docs/worker-activity.md, workers in the sidebar). A
  // status that asks the owner to act shows in the destructive color.
  let { class: className = '', status }: { class?: string; status: WorkerStatus } = $props();

  const label = $derived(workerStatusLabel(status));
  const tone = $derived(needsAttention(status) ? 'text-destructive' : 'text-muted-foreground');
</script>

<span
  class={['inline-flex shrink-0 items-center', tone, className]}
  aria-label={label}
  data-worker-status={status}
  role="img"
>
  {#if status === 'connected'}
    <CircleCheckIcon class="size-4" aria-hidden="true" />
  {:else if status === 'connecting'}
    <LoaderCircleIcon class="size-4 animate-spin" aria-hidden="true" />
  {:else if status === 'offline'}
    <WifiOffIcon class="size-4" aria-hidden="true" />
  {:else if status === 'incompatible'}
    <CircleArrowUpIcon class="size-4" aria-hidden="true" />
  {:else if status === 'key_changed'}
    <KeyRoundIcon class="size-4" aria-hidden="true" />
  {:else}
    <UnlinkIcon class="size-4" aria-hidden="true" />
  {/if}
</span>
