// Types of the system tools feature that the generated bindings do not cover.
import type { ToolsError } from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';

/** Why a command of the system tools feature failed. */
export type ToolsFailure = Failure<ToolsError>;
