<script lang="ts">
  import CircleArrowUpIcon from '@lucide/svelte/icons/circle-arrow-up';
  import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
  import KeyRoundIcon from '@lucide/svelte/icons/key-round';
  import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
  import UnlinkIcon from '@lucide/svelte/icons/unlink';
  import WifiOffIcon from '@lucide/svelte/icons/wifi-off';

  import type { WorkerStatus } from '#lib/generated/bindings.js';
  import { needsAttention, workerStatusLabel } from '#lib/utils/workers-messages.js';

  // A worker's status as an icon, with its name for screen readers
  // (docs/features/workers/worker-activity.md, workers in the sidebar). A
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
