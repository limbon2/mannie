// The skills as the settings show them (docs/features/skills.md; ADR-0061):
// the skills of this computer and of the projects, which the core reads fresh
// from their folders each time the section asks, and the switches, which the
// core checks against what the folders hold and applies from the next model
// call.
import { commands, type SkillPlace, type SkillsOverview } from '#lib/generated/bindings.js';
import type { SkillsFailure } from '#lib/types/skills.js';
import { call } from '#lib/utils/calls.js';

/** A skill by its place and name, which tells the switch that is being saved. */
export function skillKey(place: SkillPlace, name: string): string {
  return place.kind === 'computer' ? `computer:${name}` : `project:${place.projectId}:${name}`;
}

/** A switch that could not be saved, with the skill's key. */
export interface SkillSaveFailure {
  readonly failure: SkillsFailure;
  readonly key: string;
}

/** The skills and the outcome of the latest requests. */
export class SkillsState {
  /** The skills, once read. */
  overview: SkillsOverview | null = $state(null);
  /** Why the skills could not be read the last time, if they could not. */
  readFailure: SkillsFailure | null = $state(null);
  /** Whether the skills are being read. */
  reading = $state(false);
  /** Why the latest switch could not be saved, if it could not. */
  saveFailure: SkillSaveFailure | null = $state(null);
  /** The key of the skill whose switch is being saved. */
  saving: string | null = $state(null);

  /** Reads the skills from their folders. */
  async load(): Promise<void> {
    this.reading = true;
    const outcome = await call(() => commands.listSkills());
    this.reading = false;
    if (outcome.ok) {
      this.overview = outcome.data;
      this.readFailure = null;
    } else {
      this.readFailure = outcome.failure;
    }
  }

  /** Turns a skill on or off in its place. */
  async setOn(place: SkillPlace, name: string, on: boolean): Promise<void> {
    const key = skillKey(place, name);
    this.saving = key;
    this.saveFailure = null;
    const outcome = await call(() => commands.setSkillOn({ name, on, place }));
    this.saving = null;
    if (outcome.ok) {
      this.overview = outcome.data;
    } else {
      this.saveFailure = { failure: outcome.failure, key };
    }
  }
}

/** The window's skills. */
export const skills = new SkillsState();
