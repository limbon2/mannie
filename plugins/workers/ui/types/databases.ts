// The Databases feature's types in the UI (plugins/workers/docs/
// databases.md).
import type { PluginCallFailure } from '@mannie/plugin-sdk/ipc';

import type { DatabasesError } from '../generated/bindings.js';

/** Why a command of Databases failed: the plugin's error, the service's inside it, or the host's refusal. */
export type DatabasesFailure = PluginCallFailure<DatabasesError>;

/** A tab of a database's page. */
export type DatabaseTab = 'data' | 'sql' | 'structure' | 'tables';
