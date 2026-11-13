// @ts-check
// Applies the mode and palettes before the first paint. mode-watcher keeps the
// owner's mode under the key below and takes over once the app runs.
// This is a file and not an inline script because the Content Security Policy
// allows no inline script (ADR-0004).
(() => {
  /** @param {string} key */
  function readPreference(key) {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  }

  const root = document.documentElement;
  const stored = readPreference('mode-watcher-mode');
  const systemPrefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
  const followsSystem = stored !== 'light' && stored !== 'dark';
  const dark = followsSystem ? systemPrefersDark : stored === 'dark';
  root.classList.toggle('dark', dark);
  root.style.colorScheme = dark ? 'dark' : 'light';

  // Keep these IDs in sync with InterfacePaletteSettings.svelte. A default
  // or unknown ID leaves the root on its original tokens for that group.
  const palettes = {
    primary: ['blue', 'teal', 'green', 'amber', 'violet', 'rose'],
    surface: ['slate', 'stone', 'olive'],
  };
  for (const [group, choices] of Object.entries(palettes)) {
    const value = readPreference(`mannie.theme.${group}`) ?? '';
    if (choices.includes(value)) {
      root.classList.add(`palette-${group}-${value}`);
    }
  }
})();
