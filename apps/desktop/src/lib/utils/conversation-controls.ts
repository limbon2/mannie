// The places of conversation controls (ADR-0042): which controls show in the
// toolbar, in the conversation's menu and in the message box, and what the
// menu's button counts.
import type {
  ConversationControl,
  ConversationControlPlace,
  ConversationControlProps,
} from '#lib/types/shell-slots.js';

import { sortedByOrder } from './order.js';

/** The controls of one place, by their order. A control that names no place is the toolbar's. */
export function controlsIn(
  controls: readonly ConversationControl[],
  place: ConversationControlPlace,
): ConversationControl[] {
  return sortedByOrder(controls.filter((control) => (control.place ?? 'toolbar') === place));
}

/**
 * How many things the controls of the conversation's menu report as
 * working, such as running commands and working child agents, which the
 * menu's button shows while the menu is closed (ADR-0042, decision 3).
 */
export function runningInMenu(controls: readonly ConversationControl[], props: ConversationControlProps): number {
  let running = 0;
  for (const control of controlsIn(controls, 'menu')) {
    running += control.running?.(props) ?? 0;
  }
  return running;
}
