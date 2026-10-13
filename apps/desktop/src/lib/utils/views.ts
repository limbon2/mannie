// Which view the main area shows: the one the `view` search parameter names,
// or the first registered view when the URL names none.
import type { Slot } from '#lib/state/shell-slots.svelte.js';
import type { ShellView } from '#lib/types/shell-slots.js';

/** The outcome of choosing the view for a URL. */
export type ActiveView =
  | { readonly kind: 'missing'; readonly requested: string }
  | { readonly kind: 'none' }
  | { readonly kind: 'view'; readonly view: ShellView };

/** The search parameter that names the view. */
export const VIEW_PARAMETER = 'view';

/** The part of a URL that names the view; SvelteKit's `page.url` is read-only. */
interface ViewUrl {
  readonly searchParams: { get(name: string): string | null };
}

/** Chooses the view the URL asks for. */
export function activeView(views: Slot<ShellView>, url: ViewUrl): ActiveView {
  const requested = url.searchParams.get(VIEW_PARAMETER);
  if (requested === null) {
    const first = views.entries.at(0);
    return first === undefined ? { kind: 'none' } : { kind: 'view', view: first };
  }
  const view = views.find(requested);
  return view === undefined ? { kind: 'missing', requested } : { kind: 'view', view };
}
