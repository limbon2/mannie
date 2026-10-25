// Which view the main area shows: the one the `view` search parameter names,
// or the first registered view when the URL names none.
import type { Slot } from '#lib/state/shell-slots.svelte.js';
import type { ShellView } from '#lib/types/shell-slots.js';

/** The outcome of choosing the view for a URL. */
export type ActiveView =
  | { readonly kind: 'missing'; readonly requested: string }
  | { readonly kind: 'none' }
  | { readonly kind: 'plugin_off'; readonly plugin: string }
  | { readonly kind: 'view'; readonly view: ShellView };

/** The search parameter that names the view. */
export const VIEW_PARAMETER = 'view';

/** The part of a URL that names the view; SvelteKit's `page.url` is read-only. */
interface ViewUrl {
  readonly searchParams: { get(name: string): string | null };
}

/**
 * The search of a view's address with its parameters, with its `?`. The
 * view's own parameter goes last, so that the parameters cannot name another
 * view.
 */
export function viewSearch(view: string, parameters: Readonly<Record<string, string>> = {}): string {
  const search = new URLSearchParams({ ...parameters, [VIEW_PARAMETER]: view });
  return `?${search.toString()}`;
}

/** Chooses the view the URL asks for. */
export function activeView(views: Slot<ShellView>, url: ViewUrl): ActiveView {
  const requested = url.searchParams.get(VIEW_PARAMETER);
  if (requested === null) {
    const first = views.entries.at(0);
    return first === undefined ? { kind: 'none' } : { kind: 'view', view: first };
  }
  const view = views.find(requested);
  if (view !== undefined) {
    return { kind: 'view', view };
  }
  // A view of a plugin that is off says so (ADR-0030, decision 11).
  const plugin = views.pluginOff(requested);
  return plugin === null ? { kind: 'missing', requested } : { kind: 'plugin_off', plugin };
}
