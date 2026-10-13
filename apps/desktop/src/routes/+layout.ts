// The app is a static single-page app in Tauri's webview (ADR-0005): nothing
// renders on a server and nothing is prerendered, so load functions run in the
// webview, where the core is reachable.
import { commands } from '#lib/generated/bindings.js';
import { interfaceSettings } from '#lib/state/interface-settings.svelte.js';
import { useStoredLocale } from '#lib/state/locale.js';

import type { LayoutLoad } from './$types';

export const ssr = false;
export const prerender = false;

/**
 * What the window needs before it renders: whether the core started, and the
 * interface settings with the locale to show. It throws when the core does
 * not answer at all, which the root error page reports.
 */
export const load: LayoutLoad = async () => {
  // Validation builds (V1, V2, V3, V11) install their harness before anything
  // else; other builds drop this branch at build time.
  if (import.meta.env.MODE === 'validation') {
    await import('#lib/validation/harness.js');
  }
  const status = await commands.getStartupStatus();
  if (status.problem === null) {
    await interfaceSettings.load();
  } else {
    useStoredLocale(null);
  }
  return { problem: status.problem };
};
