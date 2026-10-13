// The UI's locale (ADR-0019). The core stores the owner's choice; this module
// keeps it for the window and gives it to Paraglide through the `custom-core`
// strategy. When the owner chose none, Paraglide falls through to the
// operating system's language when a catalog exists for it, and to English
// otherwise (project.inlang).
import { defineCustomClientStrategy, getLocale, getTextDirection, isLocale } from '#lib/paraglide/runtime.js';

let storedLocale: string | undefined;

defineCustomClientStrategy('custom-core', {
  getLocale: () => {
    const hasCatalog = storedLocale !== undefined && isLocale(storedLocale);
    return hasCatalog ? storedLocale : undefined;
  },
  setLocale: (locale) => {
    storedLocale = locale;
  },
});

/**
 * Uses the locale from the core's settings, or the system's when it is `null`,
 * and marks the document with its language and direction. Call it before
 * anything renders a message.
 */
export function useStoredLocale(locale: string | null): void {
  storedLocale = locale ?? undefined;
  const shown = getLocale();
  document.documentElement.lang = shown;
  document.documentElement.dir = getTextDirection(shown);
}
