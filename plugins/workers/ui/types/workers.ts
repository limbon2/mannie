// The workers feature's types in the UI (plugins/workers/docs/
// worker-activity.md).
import type { PluginCallFailure } from '@mannie/plugin-sdk/ipc';

import type { WorkersError } from '../generated/bindings.js';

/** Why a command of the workers feature failed: the plugin's error or the host's refusal. */
export type WorkersFailure = PluginCallFailure<WorkersError>;
