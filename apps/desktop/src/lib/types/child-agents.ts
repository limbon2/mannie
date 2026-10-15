// Types of the child agents feature that the generated bindings do not cover.
import type { ChildAgentsError } from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';

/** Why a command of the child agents feature failed. */
export type ChildAgentsFailure = Failure<ChildAgentsError>;
