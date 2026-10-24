// The types of the MCP plugin's UI.
import type { PluginCallFailure } from '@mannie/plugin-sdk/ipc';

import type { McpError } from '../generated/bindings.js';

/** Why a call of the plugin failed: the plugin's error, the host's refusal or no answer. */
export type McpFailure = PluginCallFailure<McpError>;
