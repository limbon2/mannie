// The draft conversation that the chat view shows while the address names no
// conversation (ADR-0068), so that the commands and the plugins that act on
// the open conversation act on it, such as a terminal's key.

/** The draft that the chat view shows. */
class ShownDraft {
  /** Its ID, or `null` while the view shows no draft. */
  id: string | null = $state(null);
}

/** The window's shown draft. */
export const shownDraft = new ShownDraft();
