// The types of the git plugin's UI.
import type { PluginCallFailure } from '@mannie/plugin-sdk/ipc';

import type { GitError } from '../generated/bindings.js';

/** Why a command of the plugin failed: the plugin's error, the host's refusal or no answer. */
export type GitFailure = PluginCallFailure<GitError>;
