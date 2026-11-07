<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Badge } from '#lib/components/ui/badge/index.js';
  import * as Item from '#lib/components/ui/item/index.js';
  import { Switch } from '#lib/components/ui/switch/index.js';
  import type { SkillPlace, SkillView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { skillKey, skills } from '#lib/state/skills.svelte.js';
  import { skillsFailureMessage, skillWarningMessage } from '#lib/utils/skills-messages.js';

  // One skill of a place (docs/features/skills.md, the settings section): its
  // name and description, which are content, what its author keeps it from,
  // what the owner may want to fix, and the switch that decides whether
  // agents see it.
  let { place, skill }: { place: SkillPlace; skill: SkillView } = $props();

  const key = $derived(skillKey(place, skill.name));
  const failure = $derived(skills.saveFailure?.key === key ? skills.saveFailure.failure : null);
  const switchId = $derived(`skill-switch-${key.replaceAll(/[^\w-]/gu, '-')}`);
</script>

<Item.Root data-skill={skill.name} data-skill-on={skill.isOn} role="listitem" variant="outline">
  <Item.Content class="min-w-0">
    <Item.Title class="flex flex-wrap items-center gap-2">
      <span class="font-mono" data-content>{skill.name}</span>
      {#if !skill.listed}
        <Badge variant="secondary">{m.skills_mark_only_mention()}</Badge>
      {/if}
      {#if !skill.mentionable}
        <Badge variant="secondary">{m.skills_mark_not_in_mention()}</Badge>
      {/if}
      {#if skill.changed}
        <Badge variant="outline">{m.skills_mark_changed()}</Badge>
      {/if}
    </Item.Title>
    <Item.Description class="line-clamp-none" data-content>{skill.description}</Item.Description>
    {#if skill.changed}
      <p class="text-sm text-muted-foreground">{m.skills_changed_note()}</p>
    {/if}
    {#each skill.warnings as warning (warning)}
      <p class="text-sm text-muted-foreground">{skillWarningMessage(warning)}</p>
    {/each}
    {#if failure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.skills_save_failed_title()}</Alert.Title>
        <Alert.Description>{skillsFailureMessage(failure)}</Alert.Description>
      </Alert.Root>
    {/if}
  </Item.Content>
  <Item.Actions>
    <Switch
      id={switchId}
      aria-label={m.skills_switch_label({ name: skill.name })}
      data-skill-switch=""
      disabled={skills.saving !== null}
      bind:checked={() => skill.isOn, (checked: boolean) => void skills.setOn(place, skill.name, checked)}
    />
  </Item.Actions>
</Item.Root>
