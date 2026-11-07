<script lang="ts">
  import { Checkbox } from '#lib/components/ui/checkbox/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { AgentForm } from '#lib/state/agent-form.svelte.js';
  import { agents } from '#lib/state/agents.svelte.js';
  import { agentName } from '#lib/utils/agents-messages.js';

  // The child agents that an agent may give tasks to (docs/features/
  // custom-agents.md). An agent that only reads has only those that only read,
  // which the core enforces for every run (ADR-0059, decision 5).
  interface Props {
    form: AgentForm;
    /** Whether the core refused this field, for the group's `data-invalid`. */
    invalid: true | undefined;
  }

  let { form, invalid }: Props = $props();

  const taskGroup = $derived(agents.options?.groups.find((group) => group.id === 'tasks'));
  const givesTasks = $derived.by(() => {
    if (taskGroup === undefined) {
      return false;
    }
    return form.hasGroup(taskGroup);
  });
  const children = $derived(
    agents.childAgents.filter((child) => {
      const onlyReads = child.draft.readOnly;
      return onlyReads || !form.readOnly;
    }),
  );
  const keys = $derived(children.map((child) => child.key));
  const every = $derived(form.children === null);
</script>

{#if givesTasks}
  <Field.Set data-agent-children="" data-invalid={invalid}>
    <Field.Legend>{m.agents_children_title()}</Field.Legend>
    <Field.Description>
      {form.readOnly ? m.agents_children_help_read_only() : m.agents_children_help()}
    </Field.Description>
    <Field.Group class="gap-3">
      <Field.Field orientation="horizontal">
        <Checkbox
          id="agent-children-every"
          checked={every}
          data-agent-children-every=""
          onCheckedChange={(on: boolean) => form.setEveryChild(on, keys)}
        />
        <Field.Label class="font-normal" for="agent-children-every">{m.agents_children_every()}</Field.Label>
      </Field.Field>
      {#if !every}
        {#each children as child (child.key)}
          <Field.Field orientation="horizontal">
            <Checkbox
              id="agent-child-{child.key}"
              checked={form.children?.includes(child.key) ?? false}
              data-agent-child={child.key}
              onCheckedChange={(on: boolean) => form.setChild(child.key, on)}
            />
            <Field.Label class="font-normal" for="agent-child-{child.key}">{agentName(child)}</Field.Label>
          </Field.Field>
        {/each}
      {/if}
    </Field.Group>
  </Field.Set>
{/if}
