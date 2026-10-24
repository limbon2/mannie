// The form of the Notebooks settings: "None" comes first among the profiles,
// a stored profile that the owner deleted reads as "None", and the number of
// passages becomes preferences only while it is whole and within its range.
import { describe, expect, it } from 'vitest';

import type { ProfileOption } from '../../../ui/generated/bindings.js';
import {
  chosenProfile,
  formOf,
  NO_PROFILE,
  preferencesOf,
  profileChoices,
  type SettingsForm,
} from '../../../ui/utils/notebooks-settings.js';

const EMBEDDINGS: ProfileOption[] = [{ id: 'profile-1', model: 'qwen3-embedding-0.6b', name: 'GPU box' }];

const FORM: SettingsForm = { embedding: 'profile-1', maxPassages: 6, rerank: NO_PROFILE };

describe('the settings form', () => {
  it('offers "None", then each profile with its model', () => {
    expect(profileChoices(EMBEDDINGS)).toEqual([
      { label: 'None', value: NO_PROFILE },
      { label: 'GPU box (qwen3-embedding-0.6b)', value: 'profile-1' },
    ]);
  });

  it('reads a stored profile that is gone as "None"', () => {
    expect(chosenProfile('profile-1', EMBEDDINGS)).toBe('profile-1');
    expect(chosenProfile('deleted', EMBEDDINGS)).toBe(NO_PROFILE);
    expect(chosenProfile(null, EMBEDDINGS)).toBe(NO_PROFILE);
    expect(formOf({ embeddingProfileId: 'deleted', maxPassages: 4, rerankProfileId: null }, EMBEDDINGS, [])).toEqual({
      embedding: NO_PROFILE,
      maxPassages: 4,
      rerank: NO_PROFILE,
    });
  });

  it('makes preferences of a whole number of passages within its range, and "None" no profile', () => {
    expect(preferencesOf(FORM)).toEqual({
      embeddingProfileId: 'profile-1',
      maxPassages: 6,
      rerankProfileId: null,
    });
    for (const maxPassages of [0, 13, 2.5, undefined]) {
      expect(preferencesOf({ ...FORM, maxPassages })).toBeNull();
    }
  });
});
