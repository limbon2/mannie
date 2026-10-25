<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
  import CircleMinusIcon from '@lucide/svelte/icons/circle-minus';
  import CircleXIcon from '@lucide/svelte/icons/circle-x';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Dialog from '#lib/components/ui/dialog/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import type { CheckOutcome, RerankProbe } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { SearchTestRun } from '#lib/state/search-test-run.svelte.js';
  import {
    checkNoteMessage,
    formatCount,
    profileFailureMessage,
    runFailureMessage,
    searchCheckLabel,
    skipReasonMessage,
  } from '#lib/utils/chat-errors.js';
  import { serverLabel } from '#lib/utils/provider-labels.js';

  // The test of a search profile (ADR-0047): each check with its outcome, the
  // server it found, and the dimensions of the vectors or the scores of a
  // passage that answers a question and one that does not.
  interface Props {
    open?: boolean;
    /** The run of the latest opening, which stays while the dialog closes. */
    run: SearchTestRun | null;
  }

  let { open = $bindable(false), run }: Props = $props();

  const scores = new Intl.NumberFormat(undefined, { maximumFractionDigits: 3, minimumFractionDigits: 2 });

  /** The scores of the rerank check, or `null` when it got no score for a passage. */
  function scoresText(probe: RerankProbe | null): string | null {
    if (probe === null) {
      return null;
    }
    const { answering, unrelated } = probe;
    if (answering === null) {
      return null;
    }
    if (unrelated === null) {
      return null;
    }
    return m.chat_search_test_scores({ answering: scores.format(answering), unrelated: scores.format(unrelated) });
  }

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
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-lg" showCloseButton={false}>
    <Dialog.Header>
      <Dialog.Title>{m.chat_providers_test_title({ name: run?.profile.name ?? '' })}</Dialog.Title>
      <Dialog.Description>{m.chat_search_test_description()}</Dialog.Description>
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
              <span class="font-medium">{searchCheckLabel(check.check)}</span>
              <span class="text-muted-foreground">{outcomeText(check.outcome)}</span>
            </span>
          </li>
        {/each}
      </ul>
      {#if report.server !== null}
        <p class="text-sm">{m.chat_providers_test_server({ server: serverLabel(report.server) })}</p>
      {/if}
      {#if report.dimensions !== null}
        <p class="text-sm">{m.chat_search_test_dimensions({ dimensions: formatCount(report.dimensions) })}</p>
      {/if}
      {const scoreLine = $derived(scoresText(report.scores))}
      {#if scoreLine !== null}
        <p class="text-sm">{scoreLine}</p>
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
