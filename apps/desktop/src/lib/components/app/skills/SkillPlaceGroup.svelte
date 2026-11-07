<script lang="ts">
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import InfoIcon from '@lucide/svelte/icons/info';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Collapsible from '#lib/components/ui/collapsible/index.js';
  import * as Item from '#lib/components/ui/item/index.js';
  import type { SkillPlaceView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { skillPlaceFailureMessage, skillProblemMessage } from '#lib/utils/skills-messages.js';

  import SkillRow from './SkillRow.svelte';

  // One place of skills (docs/features/skills.md, the settings section): this
  // computer or a project, with its folder, its skills, the folders that are
  // not skills and why, and what keeps agents from seeing some of them.
  let { place }: { place: SkillPlaceView } = $props();

  const heading = $derived(place.label ?? m.skills_place_computer());
  const headingId = $derived(
    place.place.kind === 'computer' ? 'skills-place-computer' : `skills-place-${place.place.projectId}`,
  );
</script>

<section class="flex flex-col gap-2" aria-labelledby={headingId} data-skill-place={place.place.kind}>
  <div class="flex flex-col gap-0.5">
    <h3 id={headingId} class="text-sm font-semibold" data-content={place.label === null ? undefined : ''}>{heading}</h3>
    <p class="font-mono text-xs wrap-break-word text-muted-foreground" data-content>{place.folder}</p>
    {#if place.place.kind === 'project'}
      <p class="text-sm text-muted-foreground">{m.skills_place_project_note()}</p>
    {/if}
  </div>
  {#if place.failure !== null}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Description>{skillPlaceFailureMessage(place.failure, place.folder)}</Alert.Description>
    </Alert.Root>
  {/if}
  {#if place.leftOut > 0}
    <Alert.Root>
      <InfoIcon />
      <Alert.Description>{m.skills_place_left_out({ count: place.leftOut })}</Alert.Description>
    </Alert.Root>
  {/if}
  {#if place.incomplete}
    <p class="text-sm text-muted-foreground">{m.skills_place_incomplete()}</p>
  {/if}
  {#if place.skills.length > 0}
    <Item.Group aria-label={heading}>
      {#each place.skills as skill (skill.name)}
        <SkillRow place={place.place} {skill} />
      {/each}
    </Item.Group>
  {:else if place.failure === null}
    <p class="text-sm text-muted-foreground">{m.skills_place_empty()}</p>
  {/if}
  {#if place.problems.length > 0}
    <Collapsible.Root>
      <Collapsible.Trigger>
        {#snippet child({ props })}
          <Button {...props} class="-ms-2" data-skill-problems="" size="sm" type="button" variant="ghost">
            {m.skills_problems_title({ count: place.problems.length })}
            <ChevronDownIcon data-icon="inline-end" />
          </Button>
        {/snippet}
      </Collapsible.Trigger>
      <Collapsible.Content>
        <ul class="flex flex-col gap-1 ps-2 text-sm text-muted-foreground">
          {#each place.problems as problem (problem.folder)}
            <li>{m.skills_problem_item({ folder: problem.folder, reason: skillProblemMessage(problem.problem) })}</li>
          {/each}
        </ul>
      </Collapsible.Content>
    </Collapsible.Root>
  {/if}
</section>
