<script lang="ts">
  import ArrowDownIcon from '@lucide/svelte/icons/arrow-down';
  import ArrowUpIcon from '@lucide/svelte/icons/arrow-up';
  import PlusIcon from '@lucide/svelte/icons/plus';
  import Trash2Icon from '@lucide/svelte/icons/trash-2';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import { Input } from '#lib/components/ui/input/index.js';
  import * as Select from '#lib/components/ui/select/index.js';
  import { Switch } from '#lib/components/ui/switch/index.js';
  import { Textarea } from '#lib/components/ui/textarea/index.js';
  import type { AgentToolGroup, AgentView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { AgentForm } from '#lib/state/agent-form.svelte.js';
  import { agents } from '#lib/state/agents.svelte.js';
  import { agentName } from '#lib/utils/agents-messages.js';

  // An agent's steps (docs/features/custom-agents.md). The model follows them as
  // instructions, and nothing enforces them (ADR-0059, decision 9). Move up and
  // Move down keep the order changeable from the keyboard.
  interface Props {
    form: AgentForm;
    /** Whether the core refused the steps, for the group's `data-invalid`. */
    invalid: true | undefined;
  }

  let { form, invalid }: Props = $props();

  /** The menu's value for "the agent itself". No agent's key can be `*`. */
  const SELF = '*';

  const groups = $derived(agents.options?.groups ?? []);
  const taskGroup = $derived(groups.find((group) => group.id === 'tasks'));
  const questionGroup = $derived(groups.find((group) => group.id === 'questions'));
  const givesTasks = $derived(hasOptionalGroup(taskGroup));
  const asks = $derived(hasOptionalGroup(questionGroup));
  /** The child agents that a step can name: those that the agent may give tasks to. */
  const roster = $derived(agents.childAgents.filter((child) => stepCanName(child)));

  /** Whether the agent has a group that the options may not hold. */
  function hasOptionalGroup(group: AgentToolGroup | undefined): boolean {
    return group === undefined ? false : form.hasGroup(group);
  }

  /** Whether a step can name a child: the agent may give it tasks, and it only reads when the agent does. */
  function stepCanName(child: AgentView): boolean {
    const named = form.children?.includes(child.key) ?? true;
    const allowed = child.draft.readOnly ? true : !form.readOnly;
    return named && allowed;
  }

  function chosenName(agent: string | null): string {
    const chosen = agents.find(agent);
    return chosen === undefined ? m.agents_step_self() : agentName(chosen);
  }
</script>

<Field.Set data-agent-steps="" data-invalid={invalid}>
  <Field.Legend>{m.agents_steps_title()}</Field.Legend>
  <Field.Description>{m.agents_steps_help()}</Field.Description>
  {#if form.steps.length === 0}
    <p class="text-sm text-muted-foreground">{m.agents_steps_empty()}</p>
  {/if}
  <ol class="flex flex-col gap-4">
    {#each form.steps as step, index (index)}
      <li class="flex flex-col gap-3 rounded-lg border p-3" data-agent-step={index}>
        <div class="flex items-center gap-1">
          <span class="me-auto text-sm font-medium">{m.agents_step_number({ number: String(index + 1) })}</span>
          <Button
            aria-label={m.agents_step_up({ number: String(index + 1) })}
            data-agent-step-up=""
            disabled={index === 0}
            size="icon-sm"
            title={m.agents_step_up({ number: String(index + 1) })}
            type="button"
            variant="ghost"
            onclick={() => form.moveStep(index, -1)}
          >
            <ArrowUpIcon />
          </Button>
          <Button
            aria-label={m.agents_step_down({ number: String(index + 1) })}
            data-agent-step-down=""
            disabled={index === form.steps.length - 1}
            size="icon-sm"
            title={m.agents_step_down({ number: String(index + 1) })}
            type="button"
            variant="ghost"
            onclick={() => form.moveStep(index, 1)}
          >
            <ArrowDownIcon />
          </Button>
          <Button
            aria-label={m.agents_step_remove({ number: String(index + 1) })}
            data-agent-step-remove=""
            size="icon-sm"
            title={m.agents_step_remove({ number: String(index + 1) })}
            type="button"
            variant="ghost"
            onclick={() => form.removeStep(index)}
          >
            <Trash2Icon />
          </Button>
        </div>
        <Field.Field>
          <Field.Label for="agent-step-{index}-title">{m.agents_step_title()}</Field.Label>
          <Input id="agent-step-{index}-title" data-agent-step-title="" maxlength={80} bind:value={step.title} />
        </Field.Field>
        <Field.Field>
          <Field.Label for="agent-step-{index}-text">{m.agents_step_text()}</Field.Label>
          <Textarea
            id="agent-step-{index}-text"
            class="min-h-20"
            data-agent-step-text=""
            bind:value={step.instruction}
          />
        </Field.Field>
        {#if givesTasks}
          <Field.Field>
            <Field.Label for="agent-step-{index}-agent">{m.agents_step_agent()}</Field.Label>
            <Select.Root
              type="single"
              value={step.agent ?? SELF}
              onValueChange={(value: string) => {
                step.agent = value === SELF ? null : value;
                if (step.agent === null) {
                  step.several = false;
                }
              }}
            >
              <Select.Trigger id="agent-step-{index}-agent" class="w-full" data-agent-step-agent="">
                <span class="truncate">{chosenName(step.agent)}</span>
              </Select.Trigger>
              <Select.Content>
                <Select.Item label={m.agents_step_self()} value={SELF}>{m.agents_step_self()}</Select.Item>
                {#each roster as child (child.key)}
                  <Select.Item label={agentName(child)} value={child.key}>{agentName(child)}</Select.Item>
                {/each}
              </Select.Content>
            </Select.Root>
          </Field.Field>
        {/if}
        <div class="flex flex-wrap gap-x-6 gap-y-2">
          {#if step.agent !== null}
            <Field.Field class="w-auto" orientation="horizontal">
              <Switch id="agent-step-{index}-several" data-agent-step-several="" bind:checked={step.several} />
              <Field.Label class="font-normal" for="agent-step-{index}-several">{m.agents_step_several()}</Field.Label>
            </Field.Field>
          {/if}
          {#if asks}
            <Field.Field class="w-auto" orientation="horizontal">
              <Switch id="agent-step-{index}-ask" data-agent-step-ask="" bind:checked={step.askFirst} />
              <Field.Label class="font-normal" for="agent-step-{index}-ask">{m.agents_step_ask_first()}</Field.Label>
            </Field.Field>
          {/if}
        </div>
      </li>
    {/each}
  </ol>
  <div>
    <Button data-agent-step-add="" size="sm" type="button" variant="outline" onclick={() => form.addStep()}>
      <PlusIcon data-icon="inline-start" />
      {m.agents_step_add()}
    </Button>
  </div>
</Field.Set>
