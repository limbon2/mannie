<script lang="ts">
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import CopyIcon from '@lucide/svelte/icons/copy';
  import MessageSquarePlusIcon from '@lucide/svelte/icons/message-square-plus';
  import Trash2Icon from '@lucide/svelte/icons/trash-2';
  import { onDestroy, untrack } from 'svelte';

  import { beforeNavigate, goto } from '$app/navigation';

  import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Badge } from '#lib/components/ui/badge/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Collapsible from '#lib/components/ui/collapsible/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import { Input } from '#lib/components/ui/input/index.js';
  import * as Select from '#lib/components/ui/select/index.js';
  import { Switch } from '#lib/components/ui/switch/index.js';
  import { Textarea } from '#lib/components/ui/textarea/index.js';
  import type { AgentError, AgentField, AgentView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { activities } from '#lib/state/activities.svelte.js';
  import { agentActions } from '#lib/state/agent-actions.svelte.js';
  import { AgentForm } from '#lib/state/agent-form.svelte.js';
  import { agents } from '#lib/state/agents.svelte.js';
  import { conversationDraft } from '#lib/state/conversation-draft.svelte.js';
  import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
  import type { Failure } from '#lib/types/calls.js';
  import { agentDescription, agentFailureMessage, agentName, DEFAULT_AGENT } from '#lib/utils/agents-messages.js';
  import { AGENTS_VIEW, openAgents } from '#lib/utils/agents-urls.js';
  import { CHAT_VIEW } from '#lib/utils/chat-urls.js';

  import AgentChildren from './AgentChildren.svelte';
  import AgentStepsEditor from './AgentStepsEditor.svelte';
  import AgentToolGroups from './AgentToolGroups.svelte';

  // Edits one agent (docs/features/custom-agents.md). A built-in agent shows
  // read-only with Duplicate. Saving applies from the next reply or task, and
  // leaving with changes that are not saved asks first.
  let { source }: { source: AgentForm | AgentView } = $props();

  // The page keys the editor by the agent, so one editor keeps one form.
  const form = untrack(() => (source instanceof AgentForm ? source : new AgentForm(source)));

  /** The menu's value for "no profile of its own". No profile ID can be `*`. */
  const INHERITED_PROFILE = '*';

  let savedNote = $state(false);
  let confirmingDelete = $state(false);
  let deleteFailure = $state<Failure<AgentError> | null>(null);
  /** Whether the owner already answered that the changes may go. */
  let leaving = false;
  /** Where the owner wanted to go, while the question about their changes shows. */
  let leavingTo = $state<URL | null>(null);

  const isChild = $derived(form.kind === 'child');
  const locked = $derived(form.builtin);
  const shownAgent = $derived(agents.find(form.key));
  const title = $derived.by(() => {
    if (form.key === null) {
      return m.agents_new_title();
    }
    return shownAgent === undefined ? form.name : agentName(shownAgent);
  });
  /** What the page says of a built-in agent, whose description the owner does not edit. */
  const builtinDescription = $derived(shownAgent === undefined ? form.description : agentDescription(shownAgent));
  const isSaved = $derived(form.key !== null);
  const canStartConversation = $derived(isSaved ? !isChild : false);
  const canDelete = $derived(isSaved ? !locked : false);
  const inheritedLabel = $derived(isChild ? m.agents_model_parent() : m.agents_model_conversation());
  const profileLabel = $derived(providerProfiles.find(form.profileId)?.name ?? inheritedLabel);
  const chosenProfile = $derived(providerProfiles.find(form.profileId));
  const withoutTools = $derived(chosenProfile !== undefined && !chosenProfile.useTools);

  function invalid(...fields: AgentField[]): true | undefined {
    const refused = form.refusedField;
    if (refused === null) {
      return undefined;
    }
    return fields.includes(refused) ? true : undefined;
  }

  async function save(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    savedNote = false;
    const saved = await form.save();
    if (saved === null) {
      return;
    }
    savedNote = true;
    if (form.key === null) {
      // The address changes first, so the page never shows no agent.
      await openAgents(saved.key);
      agentActions.discard();
    }
  }

  async function confirmDelete(): Promise<void> {
    if (form.key === null) {
      return;
    }
    const outcome = await agents.delete(form.key);
    confirmingDelete = false;
    if (!outcome.ok) {
      deleteFailure = outcome.failure;
      return;
    }
    leaving = true;
    await openAgents();
  }

  /** Opens a new conversation with this agent chosen. */
  async function startConversation(): Promise<void> {
    conversationDraft.reset();
    conversationDraft.agentKey = form.key === DEFAULT_AGENT ? null : form.key;
    await activities.openView(CHAT_VIEW);
  }

  /**
   * Stays on the page. The activity bar selected its entry before the navigation
   * that the question stopped, so the sidebar comes back too.
   */
  function stay(): void {
    leavingTo = null;
    agentActions.cancelPending();
    activities.selectView(AGENTS_VIEW);
  }

  async function leave(): Promise<void> {
    const destination = leavingTo;
    leavingTo = null;
    leaving = true;
    agentActions.discard();
    const pending = agentActions.pending;
    if (pending !== null) {
      await agentActions.startPending();
      return;
    }
    if (destination === null) {
      return;
    }
    // eslint-disable-next-line svelte/no-navigation-without-resolve -- the address is the one that the owner was already going to, which the router gave
    await goto(destination);
  }

  // A new agent that the owner left without touching it has nothing to keep.
  onDestroy(() => {
    const isUntouchedNewAgent = agentActions.newForm === form ? !form.dirty : false;
    if (isUntouchedNewAgent) {
      agentActions.discard();
    }
  });

  beforeNavigate((navigation) => {
    const destination = navigation.to?.url ?? null;
    if (destination === null) {
      return;
    }
    const mustAsk = form.dirty ? !leaving : false;
    if (mustAsk) {
      navigation.cancel();
      leavingTo = destination;
    }
  });
</script>

<form class="mx-auto flex w-full max-w-3xl flex-col gap-6 p-6" data-agent-editor="" onsubmit={save}>
  <header class="flex flex-wrap items-center gap-2">
    <h2 class="me-auto min-w-0 truncate text-lg font-semibold" data-content={locked ? undefined : ''} dir="auto">
      {title}
    </h2>
    {#if locked}
      <Badge variant="secondary">{m.agents_builtin_badge()}</Badge>
    {/if}
    {#if canStartConversation}
      <Button data-agent-start="" size="sm" type="button" variant="outline" onclick={() => void startConversation()}>
        <MessageSquarePlusIcon data-icon="inline-start" />
        {m.agents_new_conversation()}
      </Button>
    {/if}
    {#if shownAgent !== undefined}
      <Button
        data-agent-duplicate=""
        size="sm"
        type="button"
        variant="outline"
        onclick={() => void agentActions.duplicate(shownAgent)}
      >
        <CopyIcon data-icon="inline-start" />
        {m.agents_duplicate()}
      </Button>
    {/if}
    {#if canDelete}
      <Button
        data-agent-delete=""
        size="sm"
        type="button"
        variant="outline"
        onclick={() => {
          confirmingDelete = true;
        }}
      >
        <Trash2Icon data-icon="inline-start" />
        {m.agents_delete()}
      </Button>
    {/if}
  </header>

  {#if locked}
    <p class="text-sm text-muted-foreground">{m.agents_builtin_note()}</p>
  {/if}
  {#if deleteFailure !== null}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.agents_delete_failed_title()}</Alert.Title>
      <Alert.Description>{agentFailureMessage(deleteFailure)}</Alert.Description>
    </Alert.Root>
  {/if}

  <Field.Group>
    <!-- The heading names a built-in agent, whose stored name is its key. -->
    {#if !locked}
      <Field.Field data-invalid={invalid('name')}>
        <Field.Label for="agent-name">{m.agents_field_name()}</Field.Label>
        <Input
          id="agent-name"
          aria-invalid={invalid('name')}
          data-agent-name=""
          maxlength={60}
          bind:value={form.name}
        />
      </Field.Field>
    {/if}
    <Field.Field data-invalid={invalid('description')}>
      {#if locked}
        <Field.Title>{m.agents_field_description()}</Field.Title>
        <p class="text-sm">{builtinDescription}</p>
      {:else}
        <Field.Label for="agent-description">{m.agents_field_description()}</Field.Label>
        <Input
          id="agent-description"
          aria-describedby="agent-description-help"
          aria-invalid={invalid('description')}
          data-agent-description=""
          maxlength={300}
          bind:value={form.description}
        />
        <Field.Description id="agent-description-help">
          {isChild ? m.agents_field_description_help_child() : m.agents_field_description_help_conversation()}
        </Field.Description>
      {/if}
    </Field.Field>
    <Field.Field>
      <Field.Title>{m.agents_field_kind()}</Field.Title>
      <p class="text-sm" data-agent-kind={form.kind}>
        {isChild ? m.agents_kind_child() : m.agents_kind_conversation()}
      </p>
      <Field.Description>
        {isChild ? m.agents_kind_child_help() : m.agents_kind_conversation_help()}
      </Field.Description>
    </Field.Field>
    <Field.Field orientation="horizontal">
      <Switch
        id="agent-read-only"
        aria-describedby="agent-read-only-help"
        checked={form.readOnly}
        data-agent-read-only=""
        disabled={locked}
        onCheckedChange={(on: boolean) => form.setReadOnly(on, agents.options?.groups ?? [])}
      />
      <Field.Content>
        <Field.Label for="agent-read-only">{m.agents_field_read_only()}</Field.Label>
        <Field.Description id="agent-read-only-help">{m.agents_field_read_only_help()}</Field.Description>
      </Field.Content>
    </Field.Field>
    {#if !locked}
      <Field.Field data-invalid={invalid('profile')}>
        <Field.Label for="agent-model">{m.agents_field_model()}</Field.Label>
        <Select.Root
          type="single"
          value={form.profileId ?? INHERITED_PROFILE}
          onValueChange={(value: string) => {
            form.profileId = value === INHERITED_PROFILE ? null : value;
          }}
        >
          <Select.Trigger id="agent-model" class="w-full" data-agent-model="">
            <span class="truncate" data-content={form.profileId === null ? undefined : ''}>{profileLabel}</span>
          </Select.Trigger>
          <Select.Content>
            <Select.Item label={inheritedLabel} value={INHERITED_PROFILE}>{inheritedLabel}</Select.Item>
            {#each providerProfiles.profiles as profile (profile.id)}
              <Select.Item label={profile.name} value={profile.id}>
                <span data-content="">{profile.name}</span>
              </Select.Item>
            {/each}
          </Select.Content>
        </Select.Root>
        {#if withoutTools}
          <Field.Description>
            {isChild ? m.agents_model_no_tools_child() : m.agents_model_no_tools_conversation()}
          </Field.Description>
        {/if}
      </Field.Field>
      <Field.Field data-invalid={invalid('instructions')}>
        <Field.Label for="agent-instructions">{m.agents_field_instructions()}</Field.Label>
        <Textarea
          id="agent-instructions"
          class="min-h-40"
          aria-describedby="agent-instructions-help"
          aria-invalid={invalid('instructions')}
          data-agent-instructions=""
          bind:value={form.instructions}
        />
        <Field.Description id="agent-instructions-help">{m.agents_field_instructions_help()}</Field.Description>
      </Field.Field>
    {/if}
  </Field.Group>

  {#if !locked}
    <AgentStepsEditor {form} invalid={invalid('steps')} />
  {/if}
  <AgentToolGroups {form} invalid={invalid('tools', 'plugins')} {locked} />
  {#if !isChild && !locked}
    <AgentChildren {form} invalid={invalid('children')} />
  {/if}

  {#if !locked}
    <Collapsible.Root>
      <Collapsible.Trigger>
        {#snippet child({ props })}
          <Button {...props} class="-ms-2" data-agent-limits="" size="sm" type="button" variant="ghost">
            {m.agents_limits_title()}
            <ChevronDownIcon data-icon="inline-end" />
          </Button>
        {/snippet}
      </Collapsible.Trigger>
      <Collapsible.Content>
        <Field.Group class="mt-2 sm:flex-row">
          {#if !isChild}
            <Field.Field data-invalid={invalid('max_tasks')}>
              <Field.Label for="agent-max-tasks">{m.agents_limit_tasks()}</Field.Label>
              <Input id="agent-max-tasks" max={200} min={0} type="number" bind:value={form.maxTasks} />
            </Field.Field>
          {/if}
          <Field.Field data-invalid={invalid('max_tool_rounds')}>
            <Field.Label for="agent-max-rounds">{m.agents_limit_rounds()}</Field.Label>
            <Input id="agent-max-rounds" max={500} min={1} type="number" bind:value={form.maxToolRounds} />
          </Field.Field>
          <Field.Field data-invalid={invalid('max_tool_calls')}>
            <Field.Label for="agent-max-calls">{m.agents_limit_calls()}</Field.Label>
            <Input id="agent-max-calls" max={2000} min={1} type="number" bind:value={form.maxToolCalls} />
          </Field.Field>
        </Field.Group>
      </Collapsible.Content>
    </Collapsible.Root>

    {#if form.failure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.agents_save_failed_title()}</Alert.Title>
        <Alert.Description data-agent-failure="">{agentFailureMessage(form.failure)}</Alert.Description>
      </Alert.Root>
    {/if}
    <footer class="flex items-center gap-3 border-t pt-4">
      <Button data-agent-save="" disabled={form.saving} type="submit">{m.agents_save()}</Button>
      <p class="text-sm text-muted-foreground" role="status">
        {#if savedNote && !form.dirty}
          {m.agents_saved()}
        {/if}
      </p>
    </footer>
  {/if}
</form>

<AlertDialog.Root bind:open={confirmingDelete}>
  <AlertDialog.Content>
    <AlertDialog.Header>
      <AlertDialog.Title>{m.agents_delete_title()}</AlertDialog.Title>
      <AlertDialog.Description>{m.agents_delete_description({ name: form.name })}</AlertDialog.Description>
    </AlertDialog.Header>
    <AlertDialog.Footer>
      <AlertDialog.Cancel>{m.agents_cancel()}</AlertDialog.Cancel>
      <AlertDialog.Action data-agent-delete-confirm="" onclick={() => void confirmDelete()}>
        {m.agents_delete()}
      </AlertDialog.Action>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>

<AlertDialog.Root
  open={leavingTo !== null || agentActions.pending !== null}
  onOpenChange={(open: boolean) => {
    if (!open) {
      stay();
    }
  }}
>
  <AlertDialog.Content>
    <AlertDialog.Header>
      <AlertDialog.Title>{m.agents_unsaved_title()}</AlertDialog.Title>
      <AlertDialog.Description>{m.agents_unsaved_description()}</AlertDialog.Description>
    </AlertDialog.Header>
    <AlertDialog.Footer>
      <AlertDialog.Cancel>{m.agents_unsaved_stay()}</AlertDialog.Cancel>
      <AlertDialog.Action data-agent-leave="" onclick={() => void leave()}>
        {m.agents_unsaved_leave()}
      </AlertDialog.Action>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>
