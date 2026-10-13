// @ts-check
// Applies the theme before the first paint, so that a dark window does not
// flash light while the app starts. mode-watcher keeps the owner's choice under
// the key below (src/routes/+layout.svelte) and takes over once the app runs.
// This is a file and not an inline script because the Content Security Policy
// allows no inline script (ADR-0004).
(() => {
  /** @type {string | null} */
  let stored = null;
  try {
    stored = localStorage.getItem('mode-watcher-mode');
  } catch {
    // Without storage, the window follows the system.
  }
  const systemPrefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
  const followsSystem = stored !== 'light' && stored !== 'dark';
  const dark = followsSystem ? systemPrefersDark : stored === 'dark';
  document.documentElement.classList.toggle('dark', dark);
  document.documentElement.style.colorScheme = dark ? 'dark' : 'light';
})();
