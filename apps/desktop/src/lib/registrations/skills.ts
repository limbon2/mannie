// The skills feature's entries in the shell's slots (docs/features/skills.md;
// ADR-0061): the Skills section of the settings, after the owner's
// instructions, and the kind of mentions that gives a skill's instructions
// after "@". The transcript names calls of `read_skill` with the system
// tools' words.
import BookOpenIcon from '@lucide/svelte/icons/book-open';

import SkillsSettingsSection from '#lib/components/app/skills/SkillsSettingsSection.svelte';
import { m } from '#lib/paraglide/messages.js';
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';
import {
  skillMentionIcon,
  skillMentionRefusal,
  skillMentionSummary,
  skillMentionVariantName,
  skillMentionWarning,
  SKILLS_PROVIDER,
  SKILLS_SETTINGS_SECTION,
} from '#lib/utils/skills-messages.js';

/** Registers the skills feature. */
export function registerSkills(slots: ShellSlots): void {
  slots.settingsSections.register({
    component: SkillsSettingsSection,
    description: m.skills_settings_description,
    icon: BookOpenIcon,
    id: SKILLS_SETTINGS_SECTION,
    order: 22,
    title: m.skills_settings_title,
  });
  slots.mentionKinds.register({
    group: m.skills_mention_group,
    icon: skillMentionIcon,
    id: SKILLS_PROVIDER,
    order: 6,
    refusal: skillMentionRefusal,
    summary: skillMentionSummary,
    variantName: skillMentionVariantName,
    warning: skillMentionWarning,
  });
}
