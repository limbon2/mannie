// The types of the plugin's UI.
import type { PluginCallFailure } from '@mannie/plugin-sdk/ipc';

import type { ExampleError } from '../generated/bindings.js';

/** Why a call of the plugin failed: the plugin's error, the host's refusal or no answer. */
export type NotesFailure = PluginCallFailure<ExampleError>;
