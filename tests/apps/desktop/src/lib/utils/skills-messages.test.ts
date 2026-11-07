// The words of skills (docs/features/skills.md): every code of the core has a
// sentence from the catalog, and the mention kind words its warning, its
// refusals and what the model got.
import { describe, expect, it } from 'vitest';

import type {
  SkillError,
  SkillPlaceFailure,
  SkillProblem,
  SkillWarning,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  skillMentionRefusal,
  skillMentionSummary,
  skillMentionWarning,
  skillPlaceFailureMessage,
  skillProblemMessage,
  skillsFailureMessage,
  skillWarningMessage,
} from '../../../../../../apps/desktop/src/lib/utils/skills-messages.js';

const ERRORS: readonly SkillError[] = [
  { code: 'field_not_valid', field: 'name' },
  { code: 'not_found' },
  { code: 'project_not_found' },
  { code: 'storage_failed' },
  { code: 'store_unavailable' },
];

const PROBLEMS: readonly SkillProblem[] = [
  'around_place',
  'description_missing',
  'frontmatter_missing',
  'frontmatter_not_closed',
  'in_protected_folder',
  'name_not_valid',
  'name_taken',
  'not_text',
  'outside_project',
  'outside_skill',
  'read_failed',
];

const WARNINGS: readonly SkillWarning[] = ['description_cut', 'name_differs', 'name_from_folder'];

const PLACE_FAILURES: readonly SkillPlaceFailure[] = ['not_a_folder', 'read_failed'];

describe('skills messages', () => {
  it('words every error, problem, warning and failure of a place differently', () => {
    const errors = ERRORS.map((error) => skillsFailureMessage({ error, kind: 'core' }));
    const problems = PROBLEMS.map((problem) => skillProblemMessage(problem));
    const warnings = WARNINGS.map((warning) => skillWarningMessage(warning));
    const failures = PLACE_FAILURES.map((failure) => skillPlaceFailureMessage(failure, '/home/owner/.agents/skills'));

    for (const words of [errors, problems, warnings, failures]) {
      expect(new Set(words).size).toBe(words.length);
      for (const sentence of words) {
        expect(sentence.trim()).not.toBe('');
      }
    }
    expect(failures[0]).toContain('/home/owner/.agents/skills');
  });

  it('words the mention kind and leaves codes it does not know to the shell', () => {
    expect(skillMentionWarning('off')).not.toBeNull();
    expect(skillMentionWarning('secret')).toBeNull();
    expect(skillMentionRefusal('skill_not_found')).not.toBeNull();
    expect(skillMentionRefusal('skill_unreadable')).not.toBeNull();
    expect(skillMentionRefusal('other')).toBeNull();
    expect(skillMentionSummary({ code: 'cut', values: { shown: 10, total: 20 } })).toContain('20');
    expect(skillMentionSummary({ code: 'whole', values: { total: 20 } })).not.toBeNull();
  });
});
