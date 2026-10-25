// The source that the viewer shows (plugins/notebooks/docs/README.md,
// sources and answers): a dialog that fills the window, which a source's row,
// its menu and "Open source" of a citation open, at the start of the source
// or at a passage, scrolled to it and marked. The plugin's overlay holds the
// dialog, so a citation opens it in the chat view too.

/** One request to show a source; a new request reads it again, also for the same passage. */
export interface ViewerRequest {
  /** The passage to scroll to and mark, `<source ID>:<passage number>`, or `null` for the source's start. */
  readonly key: string | null;
  /** The element that takes the focus back when the viewer closes, such as a menu's button, or `null` for the one that had it. */
  readonly opener: HTMLElement | null;
  /** The source. */
  readonly sourceId: string;
}

/** The window's viewer of sources. */
export class SourceViewer {
  open = $state(false);
  /** The latest request, which stays while the dialog closes. */
  request: ViewerRequest | null = $state(null);

  /** Closes the viewer. */
  close(): void {
    this.open = false;
  }

  /** Shows a source from its start, or at a passage. */
  show(sourceId: string, key: string | null = null, opener: HTMLElement | null = null): void {
    this.request = { key, opener, sourceId };
    this.open = true;
  }
}

/** The window's viewer of sources. */
export const sourceViewer = new SourceViewer();
