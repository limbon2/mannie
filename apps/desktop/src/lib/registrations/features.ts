// The UI's features (docs/architecture/extension-points.md, UI shell slots).
// Each feature registers its entries in `registrations/<feature>.ts` and adds
// one line to this list; the shell knows only the slots, never a feature.
// Plugins come from the generated plugin list, `#plugin-list` (ADR-0035,
// decision 7).
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';

import { registerAgents } from './agents.js';
import { registerChat } from './chat.js';
import { registerChildAgents } from './child-agents.js';
import { registerExport } from './export.js';
import { registerPlans } from './plans.js';
import { registerPlugins } from './plugins.js';
import { registerSkills } from './skills.js';
import { registerTools } from './tools.js';
import { registerUpdates } from './updates.js';
import { registerWindowAndTray } from './window-and-tray.js';

/** A feature's registration: it adds the feature's entries to the shell's slots. */
export type FeatureRegistration = (slots: ShellSlots) => void;

/** Every feature, in the order it registers. Chat comes first, so Mannie opens into its view. */
export const features: readonly FeatureRegistration[] = [
  registerChat,
  registerWindowAndTray,
  registerTools,
  registerChildAgents,
  registerAgents,
  registerPlans,
  registerSkills,
  registerExport,
  registerUpdates,
  registerPlugins,
];
