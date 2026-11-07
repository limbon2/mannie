// The words of skills (docs/features/skills.md; ADR-0061): why a command
// failed, why a folder is not a skill, what the owner may want to fix in a
// skill, and the kind of mentions `skills`. Names, descriptions and folders
// are content and never pass through these functions.
import BookOpenIcon from '@lucide/svelte/icons/book-open';
import type { Component } from 'svelte';

import type { MentionSummary, SkillPlaceFailure, SkillProblem, SkillWarning } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import type { SkillsFailure } from '#lib/types/skills.js';

/** The ID of the Skills section of the settings. */
export const SKILLS_SETTINGS_SECTION = 'skills.settings';

/** The ID of the core's provider of skills' mentions. */
export const SKILLS_PROVIDER = 'skills';

/** Why a command of skills failed, and what the owner can do. */
export function skillsFailureMessage(failure: SkillsFailure): string {
  if (failure.kind === 'ipc') {
    return m.shell_error_ipc_failed();
  }
  const { error } = failure;
  switch (error.code) {
    case 'field_not_valid': {
      return m.skills_error_field_not_valid();
    }
    case 'not_found': {
      return m.skills_error_not_found();
    }
    case 'project_not_found': {
      return m.skills_error_project_not_found();
    }
    case 'storage_failed': {
      return m.skills_error_storage_failed();
    }
    case 'store_unavailable': {
      return m.shell_error_store_unavailable();
    }
  }
}

/** Why a folder of a place is not a skill. */
export function skillProblemMessage(problem: SkillProblem): string {
  switch (problem) {
    case 'around_place': {
      return m.skills_problem_around_place();
    }
    case 'description_missing': {
      return m.skills_problem_description_missing();
    }
    case 'frontmatter_missing': {
      return m.skills_problem_frontmatter_missing();
    }
    case 'frontmatter_not_closed': {
      return m.skills_problem_frontmatter_not_closed();
    }
    case 'in_protected_folder': {
      return m.skills_problem_in_protected_folder();
    }
    case 'name_not_valid': {
      return m.skills_problem_name_not_valid();
    }
    case 'name_taken': {
      return m.skills_problem_name_taken();
    }
    case 'not_text': {
      return m.skills_problem_not_text();
    }
    case 'outside_project': {
      return m.skills_problem_outside_project();
    }
    case 'outside_skill': {
      return m.skills_problem_outside_skill();
    }
    case 'read_failed': {
      return m.skills_problem_read_failed();
    }
  }
}

/** What the owner may want to fix in a skill that works. */
export function skillWarningMessage(warning: SkillWarning): string {
  switch (warning) {
    case 'description_cut': {
      return m.skills_warning_description_cut();
    }
    case 'name_differs': {
      return m.skills_warning_name_differs();
    }
    case 'name_from_folder': {
      return m.skills_warning_name_from_folder();
    }
  }
}

/** Why a place's folder could not be read. */
export function skillPlaceFailureMessage(failure: SkillPlaceFailure, folder: string): string {
  switch (failure) {
    case 'not_a_folder': {
      return m.skills_place_not_a_folder({ folder });
    }
    case 'read_failed': {
      return m.skills_place_read_failed({ folder });
    }
  }
}

/** The icon of a skill's mention. */
export function skillMentionIcon(): Component {
  return BookOpenIcon;
}

/** What an entry is, for screen readers. */
export function skillMentionVariantName(): string {
  return m.skills_mention_variant();
}

/** The words of an entry's warning: a project's skill that is off reaches agents for one message. */
export function skillMentionWarning(code: string): string | null {
  return code === 'off' ? m.skills_mention_warning_off() : null;
}

/** Why the provider refused a mention or listed nothing. */
export function skillMentionRefusal(code: string): string | null {
  switch (code) {
    case 'skill_not_found': {
      return m.skills_mention_not_found();
    }
    case 'skill_unreadable': {
      return m.skills_mention_unreadable();
    }
    default: {
      return null;
    }
  }
}

/** What the model got of a sent mention. */
export function skillMentionSummary(summary: MentionSummary): string | null {
  if (summary.code === 'cut') {
    return m.skills_mention_summary_cut({
      shown: String(summary.values?.shown ?? 0),
      total: String(summary.values?.total ?? 0),
    });
  }
  return m.skills_mention_summary_whole();
}
