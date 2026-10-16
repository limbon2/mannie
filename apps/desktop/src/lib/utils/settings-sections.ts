// Which section the settings page shows (docs/architecture/extension-points.md,
// settings sections; ADR-0020): each section is a page of its own, which the
// `section` search parameter names. Without one, the page shows the first
// section by order.
import { goto } from '$app/navigation';
import { resolve } from '$app/paths';

import type { Slot } from '#lib/state/shell-slots.svelte.js';
import type { SettingsSection } from '#lib/types/shell-slots.js';

import { sortedByOrder } from './order.js';

/** The outcome of choosing the settings section for a URL. */
export type ActiveSettingsSection =
  | { readonly kind: 'missing'; readonly requested: string }
  | { readonly kind: 'none' }
  | { readonly kind: 'plugin_off'; readonly plugin: string }
  | { readonly kind: 'section'; readonly section: SettingsSection };

/** The search parameter that names the settings section. */
export const SECTION_PARAMETER = 'section';

/** The part of a URL that names the section; SvelteKit's `page.url` is read-only. */
interface SectionUrl {
  readonly searchParams: { get(name: string): string | null };
}

/** Chooses the settings section the URL asks for. */
export function activeSettingsSection(sections: Slot<SettingsSection>, url: SectionUrl): ActiveSettingsSection {
  const requested = url.searchParams.get(SECTION_PARAMETER);
  if (requested === null) {
    const first = sortedByOrder(sections.entries).at(0);
    return first === undefined ? { kind: 'none' } : { kind: 'section', section: first };
  }
  const section = sections.find(requested);
  if (section !== undefined) {
    return { kind: 'section', section };
  }
  // A section of a plugin that is off says so (ADR-0030, decision 11).
  const plugin = sections.pluginOff(requested);
  return plugin === null ? { kind: 'missing', requested } : { kind: 'plugin_off', plugin };
}

/** The search parameters of a settings section's page. */
export function settingsSectionQuery(id: string): string {
  return new URLSearchParams({ [SECTION_PARAMETER]: id }).toString();
}

/** Opens the settings at the page of the section `id`, or at the first section without one. */
export async function openSettings(id?: string): Promise<void> {
  if (id === undefined) {
    await goto(resolve('/settings'));
    return;
  }
  // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve('/settings'); only the search parameters change, which resolve() does not take
  await goto(`${resolve('/settings')}?${settingsSectionQuery(id)}`);
}
