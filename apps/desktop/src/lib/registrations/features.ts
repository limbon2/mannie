// The UI's features (docs/architecture/extension-points.md, UI shell slots).
// Each feature registers its entries in `registrations/<feature>.ts` and adds
// one line to this list; the shell knows only the slots, never a feature.
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';

import { registerChat } from './chat.js';
import { registerWindowAndTray } from './window-and-tray.js';

/** A feature's registration: it adds the feature's entries to the shell's slots. */
export type FeatureRegistration = (slots: ShellSlots) => void;

/** Every feature, in the order it registers. Chat comes first, so Mannie opens into its view. */
export const features: readonly FeatureRegistration[] = [registerChat, registerWindowAndTray];
