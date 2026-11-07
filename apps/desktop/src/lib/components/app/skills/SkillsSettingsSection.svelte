<script lang="ts">
  import BookOpenIcon from '@lucide/svelte/icons/book-open';
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
  import { onMount } from 'svelte';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Empty from '#lib/components/ui/empty/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import type { SkillPlaceView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { skills } from '#lib/state/skills.svelte.js';
  import { skillsFailureMessage } from '#lib/utils/skills-messages.js';

  import SkillCommands from './SkillCommands.svelte';
  import SkillPlaceGroup from './SkillPlaceGroup.svelte';

  // The Skills section of the settings (docs/features/skills.md; ADR-0061):
  // the commands of the skills CLI to copy, then this computer's skills and
  // each project's, with a switch each. The core reads the folders again when
  // the section opens, when the window gets the focus, and with "Refresh", so
  // a skill that the owner just installed shows without a restart.
  const overview = $derived(skills.overview);

  const places: readonly SkillPlaceView[] = $derived.by(() => {
    if (overview === null) {
      return [];
    }
    return overview.computer === null ? overview.projects : [overview.computer, ...overview.projects];
  });

  const hasSkillsOrProblems = $derived(places.some(hasContent));

  /** Whether a place has something to show: skills, folders that are not skills, or a failure. */
  function hasContent(place: SkillPlaceView): boolean {
    const hasSkills = place.skills.length > 0;
    const hasProblems = place.problems.length > 0;
    const failed = place.failure !== null;
    return hasSkills || hasProblems || failed;
  }

  function placeKey(place: SkillPlaceView): string {
    return place.place.kind === 'computer' ? 'computer' : place.place.projectId;
  }

  onMount(() => {
    void skills.load();
  });
</script>

<svelte:window onfocus={() => void skills.load()} />

<div class="flex flex-col gap-4">
  <p class="text-sm text-muted-foreground">{m.skills_settings_intro()}</p>
  <SkillCommands />
  <div>
    <Button
      data-skills-refresh=""
      disabled={skills.reading}
      size="sm"
      variant="outline"
      onclick={() => void skills.load()}
    >
      <RefreshCwIcon data-icon="inline-start" />
      {m.skills_refresh()}
    </Button>
  </div>
  {#if skills.readFailure !== null}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.skills_load_failed_title()}</Alert.Title>
      <Alert.Description>{skillsFailureMessage(skills.readFailure)}</Alert.Description>
      <Alert.Action>
        <Button size="sm" variant="outline" onclick={() => void skills.load()}>{m.skills_try_again()}</Button>
      </Alert.Action>
    </Alert.Root>
  {/if}
  {#if overview === null}
    {#if skills.readFailure === null}
      <div class="flex flex-col gap-3" aria-busy="true">
        <span class="sr-only" role="status">{m.skills_loading()}</span>
        <Skeleton class="h-20 w-full" />
        <Skeleton class="h-20 w-full" />
      </div>
    {/if}
  {:else if hasSkillsOrProblems}
    {#each places as place (placeKey(place))}
      <SkillPlaceGroup {place} />
    {/each}
  {:else}
    <Empty.Root class="border" data-skills-empty="">
      <Empty.Header>
        <Empty.Media variant="icon">
          <BookOpenIcon />
        </Empty.Media>
        <Empty.Title>{m.skills_empty_title()}</Empty.Title>
        <Empty.Description>{m.skills_empty_description()}</Empty.Description>
      </Empty.Header>
    </Empty.Root>
  {/if}
</div>
