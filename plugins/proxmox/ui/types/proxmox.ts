// Shared frontend types for the Proxmox plugin UI.
import type { PluginCallFailure } from '@mannie/plugin-sdk/ipc';

import type { ProxmoxError } from '../generated/bindings.js';

/** Failure returned by any Proxmox plugin IPC command or channel. */
export type ProxmoxFailure = PluginCallFailure<ProxmoxError>;
