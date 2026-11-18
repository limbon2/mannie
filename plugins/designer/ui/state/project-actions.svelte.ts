// What the owner does with projects (plugins/designer/docs/README.md, the
// sidebar): opening the dialog that names a new one, and creating it. The
// dialog is mounted once, as the plugin's overlay, so that the sidebar opens
// it wherever the owner is. The plugin's command goes through its generated
// bindings (ADR-0034, decision 9).
import { call } from '@mannie/plugin-sdk/ipc';

import { commands } from '../generated/bindings.js';
import type { DesignerFailure } from '../types/designer.js';

import { projectSelection } from './project-selection.svelte.js';
import { projectLibrary } from './projects.svelte.js';

/** One opening of the name dialog. */
export class NameRequest {
  /** Why the latest attempt failed. */
  failure: DesignerFailure | null = $state(null);
  name = $state('');
  saving = $state(false);
}

/** The owner's actions on projects, and the dialog they open. */
export class ProjectActions {
  /** The latest name dialog, which stays while it closes. */
  name: NameRequest | null = $state(null);
  nameOpen = $state(false);

  /** Closes the dialog and forgets its failure, as the plugin turns off. */
  closeAll(): void {
    this.nameOpen = false;
  }

  /** Opens the dialog that adds a project. */
  startCreate(): void {
    this.name = new NameRequest();
    this.nameOpen = true;
  }

  /** Creates the project that the owner named, which the view then shows. */
  async confirmName(request: NameRequest): Promise<void> {
    if (request.saving) {
      return;
    }
    request.saving = true;
    request.failure = null;
    const outcome = await call(() => commands.createProject({ name: request.name }));
    request.saving = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }
    projectLibrary.keep(outcome.data);
    projectSelection.select(outcome.data.id);
    this.nameOpen = false;
  }
}

/** The owner's actions on the window's projects. */
export const projectActions = new ProjectActions();
