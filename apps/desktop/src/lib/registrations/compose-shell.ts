// The UI's composition root: it creates the shell's slots and registers the
// shell's own entries, then every feature's (docs/architecture/
// extension-points.md, rules for every extension point).
import { ShellSlots } from '#lib/state/shell-slots.svelte.js';

import { type FeatureRegistration, features } from './features.js';
import { registerShell } from './shell.js';

/** The shell's slots with the shell's entries and every feature's registered. */
export function composeShell(registrations: readonly FeatureRegistration[] = features): ShellSlots {
  const slots = new ShellSlots();
  registerShell(slots);
  for (const register of registrations) {
    register(slots);
  }
  return slots;
}
