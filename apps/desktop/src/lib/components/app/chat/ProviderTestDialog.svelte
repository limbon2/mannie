<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
  import CircleMinusIcon from '@lucide/svelte/icons/circle-minus';
  import CircleXIcon from '@lucide/svelte/icons/circle-x';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Dialog from '#lib/components/ui/dialog/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import type { CheckOutcome } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { ProviderTestRun } from '#lib/state/provider-test-run.svelte.js';
  import {
    checkLabel,
    checkNoteMessage,
    formatCount,
    profileFailureMessage,
    runFailureMessage,
    skipReasonMessage,
  } from '#lib/utils/chat-errors.js';
  import { serverLabel } from '#lib/utils/provider-labels.js';

  // The provider test of a profile (docs/features/chat/providers.md, the
  // provider test): each check with its outcome, the server it found, and the
  // limits the server reports, which the owner can apply.
  interface Props {
    open?: boolean;
    /** The run of the latest opening, which stays while the dialog closes. */
    run: ProviderTestRun | null;
  }

  let { open = $bindable(false), run }: Props = $props();

  function outcomeText(outcome: CheckOutcome): string {
    switch (outcome.outcome) {
      case 'failed': {
        return runFailureMessage(outcome.failure);
      }
      case 'passed': {
        return m.chat_providers_check_passed();
      }
      case 'skipped': {
        return skipReasonMessage(outcome.reason);
      }
      case 'warning': {
        return checkNoteMessage(outcome.note);
      }
    }
  }

  function applyLimits(): void {
    if (run !== null) {
      void run.applyLimits();
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-lg" showCloseButton={false}>
    <Dialog.Header>
      <Dialog.Title>{m.chat_providers_test_title({ name: run?.profile.name ?? '' })}</Dialog.Title>
      <Dialog.Description>{m.chat_providers_test_description()}</Dialog.Description>
    </Dialog.Header>
    {#if run?.failure}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Description>{profileFailureMessage(run.failure)}</Alert.Description>
      </Alert.Root>
    {:else if run?.report}
      {const report = $derived(run.report)}
      <ul class="flex flex-col gap-2 text-sm">
        {#each report.checks as check (check.check)}
          <li class="flex items-start gap-2">
            {#if check.outcome.outcome === 'passed'}
              <CircleCheckIcon class="mt-0.5 size-4 shrink-0 text-primary" aria-hidden="true" />
            {:else if check.outcome.outcome === 'failed'}
              <CircleXIcon class="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden="true" />
            {:else if check.outcome.outcome === 'warning'}
              <CircleAlertIcon class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            {:else}
              <CircleMinusIcon class="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            {/if}
            <span class="flex flex-col">
              <span class="font-medium">{checkLabel(check.check)}</span>
              <span class="text-muted-foreground">{outcomeText(check.outcome)}</span>
            </span>
          </li>
        {/each}
      </ul>
      {#if report.server !== null}
        <p class="text-sm">{m.chat_providers_test_server({ server: serverLabel(report.server) })}</p>
      {/if}
      {#if report.reportedLimits !== null}
        <div class="flex flex-col gap-2 rounded-md border p-3 text-sm">
          <p>
            {report.reportedLimits.outputLimit === null
              ? m.chat_providers_test_reported_window({ window: formatCount(report.reportedLimits.contextWindow) })
              : m.chat_providers_test_reported_limits({
                  limit: formatCount(report.reportedLimits.outputLimit),
                  window: formatCount(report.reportedLimits.contextWindow),
                })}
          </p>
          <div class="flex items-center gap-3">
            <Button disabled={run.applied} size="sm" variant="outline" onclick={applyLimits}>
              {m.chat_providers_test_apply_limits()}
            </Button>
            <span class="text-muted-foreground" role="status"
              >{run.applied ? m.chat_providers_test_limits_applied() : ''}</span
            >
          </div>
          {#if run.applyFailure !== null}
            <p class="text-destructive">{profileFailureMessage(run.applyFailure)}</p>
          {/if}
        </div>
      {/if}
    {:else}
      <p class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
        <Spinner aria-label={m.chat_providers_testing()} />
        <span>{m.chat_providers_testing()}</span>
      </p>
    {/if}
    <Dialog.Footer>
      <Button variant="outline" onclick={() => (open = false)}>{m.chat_close()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
