// The form of the Notebooks settings (plugins/notebooks/docs/README.md, the
// models): the choices of search profiles after "None", and the number of
// passages as the owner types it, which the plugin checks again.
import type { Preferences, ProfileOption } from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import { FEWEST_PASSAGES, MOST_PASSAGES } from '../generated/plugin.js';

/** The value of the choice "None", which no profile's ID takes. */
export const NO_PROFILE = 'none';

/** A choice of a select. */
export interface ProfileChoice {
  readonly label: string;
  readonly value: string;
}

/** What the owner chose and typed in the settings; a number field that is empty or invalid holds `undefined`. */
export interface SettingsForm {
  embedding: string;
  maxPassages: number | undefined;
  rerank: string;
}

/** "None" and the profiles of a kind, each named with its model. */
export function profileChoices(profiles: readonly ProfileOption[]): ProfileChoice[] {
  const choices = profiles.map((profile) => ({
    label: m.notebooks_settings_profile_option({ model: profile.model, name: profile.name }),
    value: profile.id,
  }));
  return [{ label: m.notebooks_settings_none(), value: NO_PROFILE }, ...choices];
}

/** The choice of a stored profile: "None" for none, and for one that the owner deleted. */
export function chosenProfile(id: string | null, profiles: readonly ProfileOption[]): string {
  const exists = profiles.some((profile) => profile.id === id);
  if (id === null) {
    return NO_PROFILE;
  }
  return exists ? id : NO_PROFILE;
}

/** The form of stored preferences. */
export function formOf(
  preferences: Preferences,
  embeddings: readonly ProfileOption[],
  reranks: readonly ProfileOption[],
): SettingsForm {
  return {
    embedding: chosenProfile(preferences.embeddingProfileId, embeddings),
    maxPassages: preferences.maxPassages,
    rerank: chosenProfile(preferences.rerankProfileId, reranks),
  };
}

/** Whether a typed number is a whole number from `fewest` to `most`. */
function isWholeIn(value: number | undefined, fewest: number, most: number): value is number {
  if (value === undefined) {
    return false;
  }
  const isWhole = Number.isInteger(value);
  const isInRange = fewest <= value && value <= most;
  return isWhole && isInRange;
}

/** Whether a number of passages is a whole number within the plugin's range. */
export function isValidPassages(value: number | undefined): value is number {
  return isWholeIn(value, FEWEST_PASSAGES, MOST_PASSAGES);
}

/** The preferences of a form, or `null` while the number of passages is outside its range. */
export function preferencesOf(form: SettingsForm): Preferences | null {
  const { maxPassages } = form;
  if (!isValidPassages(maxPassages)) {
    return null;
  }
  return {
    embeddingProfileId: form.embedding === NO_PROFILE ? null : form.embedding,
    maxPassages,
    rerankProfileId: form.rerank === NO_PROFILE ? null : form.rerank,
  };
}
