<script lang="ts">
  import XIcon from '@lucide/svelte/icons/x';

  import UpdateNoticeFailure from '#lib/components/app/updates/UpdateNoticeFailure.svelte';
  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { updates } from '#lib/state/updates.svelte.js';
  import { checkForUpdatesInSettings } from '#lib/utils/updates-section.js';

  // The notices about updates in the window's lower corner (docs/features/
  // updates.md): what became of the last install, and a version that is
  // ready to install. The region stays in the page while it is empty, so that
  // a screen reader announces a notice when one arrives.
  const outcome = $derived(updates.outcome);
  const readyVersion = $derived(updates.noticeVersion);

  /** The version that runs, which the notice names when the install failed. */
  const stillRunningVersion = $derived(outcome?.result === 'failed' ? (updates.status?.currentVersion ?? null) : null);

  /** The failed action that the notice about the ready version started. */
  const readyFailure = $derived.by(() => {
    const failed = updates.actionFailure;
    if (failed === null) {
      return null;
    }
    const fromThisNotice = failed.action === 'restart' || (failed.action === 'page' && failed.page === 'ready');
    return fromThisNotice ? failed : null;
  });

  /** The failed action that the notice about the last install started. */
  const outcomeFailure = $derived.by(() => {
    const failed = updates.actionFailure;
    if (failed === null) {
      return null;
    }
    const fromThisNotice = failed.action === 'page' && failed.page === 'running';
    return fromThisNotice ? failed : null;
  });
</script>

<section
  class="pointer-events-none fixed inset-e-4 bottom-4 z-40 flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-2"
  aria-label={m.updates_notice_label()}
  aria-live="polite"
  data-update-notices=""
>
  {#if outcome !== null}
    <div
      class="pointer-events-auto flex flex-col gap-3 rounded-lg border bg-popover p-3 text-sm text-popover-foreground shadow-lg"
      data-update-notice="outcome"
      data-update-result={outcome.result}
    >
      <div class="flex items-start gap-2">
        <p class="flex-1 font-medium wrap-break-word">
          {outcome.result === 'completed'
            ? m.updates_outcome_completed_title({ version: outcome.toVersion })
            : m.updates_outcome_failed_title({ version: outcome.toVersion })}
        </p>
        <Button
          class="-me-1 -mt-1 shrink-0"
          aria-label={m.updates_notice_close()}
          data-update-notice-close=""
          size="icon-sm"
          variant="ghost"
          onclick={() => void updates.closeOutcome()}
        >
          <XIcon />
        </Button>
      </div>
      {#if stillRunningVersion !== null}
        <p class="text-muted-foreground">{m.updates_outcome_failed_description({ version: stillRunningVersion })}</p>
      {/if}
      {#if outcomeFailure !== null}
        <UpdateNoticeFailure failed={outcomeFailure} />
      {/if}
      <div class="flex flex-wrap gap-2">
        {#if outcome.result === 'completed'}
          <Button size="sm" variant="outline" onclick={() => void updates.openReleasePage('running')}>
            {m.updates_whats_new()}
          </Button>
        {:else}
          <Button disabled={!updates.canCheck} size="sm" variant="outline" onclick={checkForUpdatesInSettings}>
            {m.updates_check_now()}
          </Button>
        {/if}
      </div>
    </div>
  {/if}

  {#if readyVersion !== null}
    <div
      class="pointer-events-auto flex flex-col gap-3 rounded-lg border bg-popover p-3 text-sm text-popover-foreground shadow-lg"
      data-update-notice="ready"
    >
      <div class="flex items-start gap-2">
        <p class="flex-1 font-medium wrap-break-word">{m.updates_notice_ready_title({ version: readyVersion })}</p>
        <Button
          class="-me-1 -mt-1 shrink-0"
          aria-label={m.updates_notice_close()}
          data-update-notice-close=""
          size="icon-sm"
          variant="ghost"
          onclick={() => updates.closeReadyNotice()}
        >
          <XIcon />
        </Button>
      </div>
      {#if readyFailure !== null}
        <UpdateNoticeFailure failed={readyFailure} />
      {/if}
      <div class="flex flex-wrap gap-2">
        <Button
          data-update-notice-restart=""
          disabled={!updates.canRestart}
          size="sm"
          onclick={() => void updates.restart()}
        >
          {m.updates_restart_and_update()}
        </Button>
        <Button size="sm" variant="outline" onclick={() => void updates.openReleasePage('ready')}>
          {m.updates_whats_new()}
        </Button>
      </div>
    </div>
  {/if}
</section>
