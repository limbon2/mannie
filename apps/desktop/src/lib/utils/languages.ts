// The languages the window can show (ADR-0019): the locales that have a
// catalog, the system's choice among them, and their names from `Intl`.
import { baseLocale, extractLocaleFromNavigator, getLocale, locales } from '#lib/paraglide/runtime.js';

/** The locales that have a catalog. */
export function availableLocales(): readonly string[] {
  return locales;
}

/** The locale the window shows when the owner chose none. */
export function systemLocale(): string {
  return extractLocaleFromNavigator() ?? baseLocale;
}

/** The name of a language, written in the current locale, from `Intl`. */
export function languageName(locale: string): string {
  return new Intl.DisplayNames([getLocale()], { type: 'language' }).of(locale) ?? locale;
}
