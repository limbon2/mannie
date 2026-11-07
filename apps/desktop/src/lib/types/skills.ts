// Types of skills that the generated bindings do not cover (docs/features/
// skills.md).
import type { SkillError } from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';

/** Why a command of skills failed. */
export type SkillsFailure = Failure<SkillError>;
