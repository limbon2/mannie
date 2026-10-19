// The types of the SSH plugin's UI.
import type { PluginCallFailure } from '@mannie/plugin-sdk/ipc';

import type { SshError } from '../generated/bindings.js';

/** Why a call of the plugin failed: the plugin's error, the host's refusal or no answer. */
export type SshFailure = PluginCallFailure<SshError>;
