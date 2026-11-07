<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import ClipboardListIcon from '@lucide/svelte/icons/clipboard-list';
  import { onMount, untrack } from 'svelte';

  import Markdown from '#lib/components/app/markdown/Markdown.svelte';
  import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Empty from '#lib/components/ui/empty/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import { Input } from '#lib/components/ui/input/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { Textarea } from '#lib/components/ui/textarea/index.js';
  import type { PlanError, PlanView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { plans } from '#lib/state/plans.svelte.js';
  import type { Failure } from '#lib/types/calls.js';
  import type { SidePanelProps } from '#lib/types/shell-slots.js';
  import { planFailureMessage } from '#lib/utils/plans-messages.js';

  import PlanRunButtons from './PlanRunButtons.svelte';

  // A plan in a side panel (docs/features/plans.md): its text as Markdown, the two
  // ways to run it, Edit and Delete. The core stores an edit and marks the plan,
  // so the model reads the edit before it saves the plan again (ADR-0060,
  // decision 5). The plan's text is model output, so it renders as other replies
  // do and runs no script.
  let { argument, close, setLabel }: SidePanelProps = $props();

  const number = untrack(() => Number(argument));

  let status = $state<'error' | 'gone' | 'loading' | 'ready'>('loading');
  let plan = $state<PlanView | null>(null);
  let failure = $state<Failure<PlanError> | null>(null);
  let editing = $state(false);
  let editTitle = $state('');
  let editBody = $state('');
  let saving = $state(false);
  let saveFailure = $state<Failure<PlanError> | null>(null);
  let confirmingDelete = $state(false);

  function isGone(reason: Failure<PlanError>): boolean {
    return reason.kind === 'core' ? reason.error.code === 'not_found' : false;
  }

  async function load(): Promise<void> {
    status = 'loading';
    const outcome = await plans.read(number);
    if (outcome.ok) {
      plan = outcome.data;
      status = 'ready';
      setLabel(outcome.data.summary.title, true);
    } else if (isGone(outcome.failure)) {
      status = 'gone';
    } else {
      failure = outcome.failure;
      status = 'error';
    }
  }

  function startEditing(): void {
    if (plan === null) {
      return;
    }
    editTitle = plan.summary.title;
    editBody = plan.body;
    saveFailure = null;
    editing = true;
  }

  async function save(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    saving = true;
    const outcome = await plans.update(number, editTitle, editBody);
    saving = false;
    if (outcome.ok) {
      plan = outcome.data;
      setLabel(outcome.data.summary.title, true);
      editing = false;
    } else {
      saveFailure = outcome.failure;
    }
  }

  async function confirmDelete(): Promise<void> {
    const outcome = await plans.delete(number);
    confirmingDelete = false;
    if (outcome.ok) {
      close();
    } else {
      saveFailure = outcome.failure;
    }
  }

  onMount(() => {
    void load();
  });
</script>

<div class="flex min-h-0 flex-1 flex-col overflow-y-auto p-4" data-plan-panel="">
  {#if status === 'loading'}
    <div class="flex flex-col gap-4" aria-busy="true">
      <span class="sr-only" role="status">{m.plans_loading()}</span>
      <Skeleton class="h-6 w-1/2" />
      <Skeleton class="h-32 w-full" />
    </div>
  {:else if status === 'gone'}
    <Empty.Root class="flex-1">
      <Empty.Header>
        <Empty.Media variant="icon">
          <ClipboardListIcon />
        </Empty.Media>
        <Empty.Title>{m.plans_gone_title()}</Empty.Title>
        <Empty.Description>{m.plans_gone_description()}</Empty.Description>
      </Empty.Header>
      <Empty.Content>
        <Button variant="outline" onclick={close}>{m.chat_close()}</Button>
      </Empty.Content>
    </Empty.Root>
  {:else if status === 'error'}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.plans_failed_title()}</Alert.Title>
      <Alert.Description>{failure === null ? '' : planFailureMessage(failure)}</Alert.Description>
      <Alert.Action>
        <Button size="sm" variant="outline" onclick={() => void load()}>{m.chat_try_again()}</Button>
      </Alert.Action>
    </Alert.Root>
  {:else if editing}
    <form class="flex flex-1 flex-col gap-4" data-plan-edit="" onsubmit={save}>
      <Field.Field>
        <Field.Label for="plan-title">{m.plans_field_title()}</Field.Label>
        <Input id="plan-title" data-plan-edit-title="" maxlength={80} bind:value={editTitle} />
      </Field.Field>
      <Field.Field class="flex-1">
        <Field.Label for="plan-body">{m.plans_field_body()}</Field.Label>
        <Textarea
          id="plan-body"
          class="min-h-80 flex-1 font-mono text-sm"
          data-plan-edit-body=""
          bind:value={editBody}
        />
      </Field.Field>
      {#if saveFailure !== null}
        <p class="text-sm text-destructive" role="alert">{planFailureMessage(saveFailure)}</p>
      {/if}
      <div class="flex items-center gap-2">
        <Button data-plan-save="" disabled={saving} type="submit">{m.plans_save()}</Button>
        <Button
          type="button"
          variant="outline"
          onclick={() => {
            editing = false;
          }}
        >
          {m.plans_cancel()}
        </Button>
      </div>
    </form>
  {:else if plan !== null}
    <div class="flex flex-col gap-4">
      <div class="flex flex-wrap items-center gap-2">
        <PlanRunButtons plan={plan.summary} />
        <Button data-plan-edit-start="" size="sm" variant="ghost" onclick={startEditing}>{m.plans_edit()}</Button>
        <Button
          data-plan-delete=""
          size="sm"
          variant="ghost"
          onclick={() => {
            confirmingDelete = true;
          }}
        >
          {m.plans_delete()}
        </Button>
      </div>
      {#if saveFailure !== null}
        <p class="text-sm text-destructive" role="alert">{planFailureMessage(saveFailure)}</p>
      {/if}
      <div data-plan-body="">
        <Markdown streaming={false} text={plan.body} />
      </div>
    </div>
  {/if}
</div>

<AlertDialog.Root bind:open={confirmingDelete}>
  <AlertDialog.Content>
    <AlertDialog.Header>
      <AlertDialog.Title>{m.plans_delete_title()}</AlertDialog.Title>
      <AlertDialog.Description>
        {m.plans_delete_description({ title: plan?.summary.title ?? '' })}
      </AlertDialog.Description>
    </AlertDialog.Header>
    <AlertDialog.Footer>
      <AlertDialog.Cancel>{m.plans_cancel()}</AlertDialog.Cancel>
      <AlertDialog.Action data-plan-delete-confirm="" onclick={() => void confirmDelete()}>
        {m.plans_delete()}
      </AlertDialog.Action>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>
