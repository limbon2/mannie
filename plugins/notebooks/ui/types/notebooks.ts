// The types of the Notebooks plugin's UI.
import type { PluginCallFailure } from '@mannie/plugin-sdk/ipc';

import type { NotebooksError, SourceRefusal } from '../generated/bindings.js';

/** Why a call of the plugin failed: the plugin's error, the host's refusal or no answer. */
export type NotebooksFailure = PluginCallFailure<NotebooksError>;

/** A file that was not added to a notebook, with why. */
export interface FileRefusal {
  /** The file's name, as the owner chose or dropped it. */
  readonly name: string;
  /** Why the plugin refused it, or why the call failed. */
  readonly reason: NotebooksFailure | SourceRefusal;
}
