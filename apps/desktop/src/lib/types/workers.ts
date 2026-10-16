// The workers feature's types in the UI (docs/features/workers/
// worker-activity.md).
import type { WorkersError } from '#lib/generated/bindings.js';

import type { Failure } from './calls.js';

/** Why a command of the workers feature failed. */
export type WorkersFailure = Failure<WorkersError>;
