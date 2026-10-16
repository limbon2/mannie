// The internal plugins of this version of Mannie, in the order that the
// Plugins page lists them (ADR-0030, decision 2). This list is the only core
// file that imports a plugin's UI code; the lint rule of plugin boundaries
// lets it (ADR-0030, decision 10).
import type { PluginRegistration } from '#lib/types/plugins.js';

import { workersPlugin } from './workers.js';

/** Every internal plugin. */
export const plugins: readonly PluginRegistration[] = [workersPlugin];
