// The selection is local to the window. The plugin's store holds the projects.
import type { ProjectView } from '../generated/bindings.js';

import { projectLibrary } from './projects.svelte.js';

/** Which project the window shows. */
export class ProjectSelection {
  /** The selected project's ID, or `null` while the owner has selected none. */
  id: string | null = $state(null);

  /**
   * The project that the view shows: the selected one, or the newest while it
   * has none or the selected one is gone.
   */
  shown(): ProjectView | undefined {
    return this.id === null ? projectLibrary.latest() : (projectLibrary.find(this.id) ?? projectLibrary.latest());
  }

  select(id: string): void {
    this.id = id;
  }
}

/** The window's selected project. */
export const projectSelection = new ProjectSelection();
