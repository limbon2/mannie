// The types of the Designer plugin's UI.
import type { PluginCallFailure } from '@mannie/plugin-sdk/ipc';

import type { DesignerError } from '../generated/bindings.js';

/** Why a call of the plugin failed: the plugin's error, the host's refusal or no answer. */
export type DesignerFailure = PluginCallFailure<DesignerError>;
