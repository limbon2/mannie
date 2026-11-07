<script lang="ts">
  import { Checkbox } from '#lib/components/ui/checkbox/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import type { AgentToolGroup } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { AgentForm } from '#lib/state/agent-form.svelte.js';
  import { agents } from '#lib/state/agents.svelte.js';
  import { pluginName } from '#lib/state/plugin-catalog.js';
  import { groupDescription, groupName } from '#lib/utils/agents-messages.js';

  // What an agent can do (docs/features/custom-agents.md): groups of the core's
  // tools, and the plugins whose tools it gets. A definition only narrows what
  // its base has, and its operations still ask (ADR-0059, decision 4).
  interface Props {
    form: AgentForm;
    /** Whether the core refused these fields, for the group's `data-invalid`. */
    invalid: true | undefined;
    /** Whether nothing changes, for a built-in agent. */
    locked: boolean;
  }

  let { form, invalid, locked }: Props = $props();

  const isChild = $derived(form.kind === 'child');
  /** The groups that this kind can have, those that act off while the agent only reads. */
  const groups = $derived(
    (agents.options?.groups ?? []).filter((group) => (isChild ? group.forChild : group.forConversation)),
  );
  const plugins = $derived(agents.options?.plugins ?? []);
  const pluginIds = $derived(plugins.map((plugin) => plugin.id));
  const everyPlugin = $derived(form.plugins === null);

  function isOff(group: AgentToolGroup): boolean {
    if (locked) {
      return true;
    }
    return group.forReadOnly ? false : form.readOnly;
  }
</script>

<Field.Set data-agent-tools="" data-invalid={invalid}>
  <Field.Legend>{m.agents_tools_title()}</Field.Legend>
  <Field.Description>{m.agents_tools_help()}</Field.Description>
  <Field.Group class="gap-3">
    {#each groups as group (group.id)}
      <Field.Field orientation="horizontal">
        <Checkbox
          id="agent-group-{group.id}"
          aria-describedby="agent-group-{group.id}-help"
          checked={form.hasGroup(group)}
          data-agent-group={group.id}
          disabled={isOff(group)}
          onCheckedChange={(on: boolean) => form.setGroup(group, on)}
        />
        <Field.Content>
          <Field.Label for="agent-group-{group.id}">{groupName(group.id)}</Field.Label>
          <Field.Description id="agent-group-{group.id}-help">{groupDescription(group.id)}</Field.Description>
        </Field.Content>
      </Field.Field>
    {/each}
  </Field.Group>
</Field.Set>

{#if !locked}
  <Field.Set data-agent-plugins="">
    <Field.Legend>{m.agents_plugins_title()}</Field.Legend>
    <Field.Description>
      {form.readOnly ? m.agents_plugins_help_read_only() : m.agents_plugins_help()}
    </Field.Description>
    <Field.Group class="gap-3">
      <Field.Field orientation="horizontal">
        <Checkbox
          id="agent-plugins-every"
          checked={everyPlugin}
          data-agent-plugins-every=""
          onCheckedChange={(on: boolean) => form.setEveryPlugin(on, pluginIds)}
        />
        <Field.Label class="font-normal" for="agent-plugins-every">{m.agents_plugins_every()}</Field.Label>
      </Field.Field>
      {#if !everyPlugin}
        {#if plugins.length === 0}
          <p class="text-sm text-muted-foreground">{m.agents_plugins_none()}</p>
        {/if}
        {#each plugins as plugin (plugin.id)}
          <Field.Field orientation="horizontal">
            <Checkbox
              id="agent-plugin-{plugin.id}"
              checked={form.plugins?.includes(plugin.id) ?? false}
              data-agent-plugin={plugin.id}
              onCheckedChange={(on: boolean) => form.setPlugin(plugin.id, on)}
            />
            <Field.Label class="font-normal" for="agent-plugin-{plugin.id}">{pluginName(plugin.id)}</Field.Label>
          </Field.Field>
        {/each}
      {/if}
    </Field.Group>
  </Field.Set>
{/if}
