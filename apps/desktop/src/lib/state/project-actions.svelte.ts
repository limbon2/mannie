// What the owner does with projects (docs/features/chat/projects.md): adding
// a folder through the system's folder dialog or a typed path, renaming and
// removing. The dialogs these actions open are mounted once, with the chat
// feature's other dialogs.
import { type ChatError, commands, type ProjectView } from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';
import { call } from '#lib/utils/calls.js';
import { newConversationProjectId, openConversationId, startNewConversation } from '#lib/utils/chat-urls.js';

import { conversationList } from './conversation-list.svelte.js';
import { sidebarCollapse } from './sidebar-collapse.svelte.js';

/** One opening of the dialog where the owner types a project's folder. */
export class AddPathRequest {
  /** Why the core refused the latest folder, or why the folder dialog did not give one. */
  failure: Failure<ChatError> | null = $state(null);
  folder = $state('');
  /** Whether the core is checking the folder. */
  saving = $state(false);

  constructor(failure: Failure<ChatError> | null) {
    this.failure = failure;
  }
}

/** One opening of the rename dialog, which starts from the current name. */
export class RenameProjectRequest {
  /** Why the core refused the latest name. */
  failure: Failure<ChatError> | null = $state(null);
  name = $state('');
  readonly project: ProjectView;
  saving = $state(false);

  constructor(project: ProjectView) {
    this.project = project;
    this.name = project.name;
  }
}

/** One opening of the removal confirmation. */
export class RemoveProjectRequest {
  /** How many conversations the owner confirms to delete with the project. */
  conversations = $state(0);
  /** Why the core refused the latest attempt. */
  failure: Failure<ChatError> | null = $state(null);
  readonly project: ProjectView;
  removing = $state(false);

  constructor(project: ProjectView, conversations: number) {
    this.project = project;
    this.conversations = conversations;
  }
}

/** The owner's actions on projects, and the dialogs they open. */
export class ProjectActions {
  /** Whether the folder dialog of "+" is open. */
  adding = $state(false);
  /** The latest dialog for a typed folder, which stays while it closes. */
  addPath: AddPathRequest | null = $state(null);
  addPathOpen = $state(false);
  /** The latest removal confirmation, which stays while it closes. */
  removing: RemoveProjectRequest | null = $state(null);
  removeOpen = $state(false);
  /** The latest rename dialog, which stays while it closes. */
  renaming: RenameProjectRequest | null = $state(null);
  renameOpen = $state(false);

  /**
   * Opens the system's folder dialog in the core and adds the chosen folder
   * as a project, which then opens with a new conversation. Closing the
   * dialog changes nothing. A folder that Mannie refuses, or a dialog that
   * cannot open, leads to the dialog for a typed folder with the cause.
   */
  async add(): Promise<void> {
    if (this.adding) {
      return;
    }
    this.adding = true;
    const outcome = await call(() => commands.addProject());
    this.adding = false;
    if (!outcome.ok) {
      this.startAddFromPath(outcome.failure);
      return;
    }
    if (outcome.data !== null) {
      await this.#open(outcome.data);
    }
  }

  /** Removes the project of a removal confirmation, which closes when the core removed it. */
  async confirmRemove(request: RemoveProjectRequest): Promise<void> {
    const projectId = request.project.id;
    const deleted = conversationList.conversations
      .filter((conversation) => conversation.projectId === projectId)
      .map((conversation) => conversation.id);
    request.removing = true;
    const outcome = await call(() => commands.removeProject({ conversations: request.conversations, projectId }));
    request.removing = false;
    if (!outcome.ok) {
      const { failure } = outcome;
      const changed = failure.kind === 'core' && failure.error.code === 'project_changed';
      if (changed) {
        // The owner confirms the number again.
        request.conversations = failure.error.conversations;
      }
      request.failure = failure;
      return;
    }
    if (this.removing === request) {
      this.removeOpen = false;
    }
    sidebarCollapse.forget(projectId);
    const openId = openConversationId();
    const showsDeleted = openId !== null && deleted.includes(openId);
    const showsProject = newConversationProjectId() === projectId;
    if (showsDeleted || showsProject) {
      await startNewConversation();
    }
  }

  /** Adds the typed folder of a dialog as a project, and closes the dialog when the core accepted it. */
  async saveAddFromPath(request: AddPathRequest): Promise<void> {
    request.saving = true;
    const outcome = await call(() => commands.addProjectFromPath({ folder: request.folder }));
    request.saving = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }
    request.failure = null;
    if (this.addPath === request) {
      this.addPathOpen = false;
    }
    await this.#open(outcome.data);
  }

  /** Saves the name of a rename dialog, which closes when the core accepted it. */
  async saveRename(request: RenameProjectRequest): Promise<void> {
    request.saving = true;
    const outcome = await call(() => commands.renameProject({ name: request.name, projectId: request.project.id }));
    request.saving = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }
    request.failure = null;
    conversationList.upsertProject(outcome.data);
    if (this.renaming === request) {
      this.renameOpen = false;
    }
  }

  /** Opens the dialog where the owner types a project's folder, with why the folder dialog failed, if it did. */
  startAddFromPath(failure: Failure<ChatError> | null = null): void {
    this.addPath = new AddPathRequest(failure);
    this.addPathOpen = true;
  }

  /** Asks the owner to confirm the removal of a project and of its conversations. */
  startRemove(project: ProjectView, conversations: number): void {
    this.removing = new RemoveProjectRequest(project, conversations);
    this.removeOpen = true;
  }

  /** Opens the rename dialog for a project. */
  startRename(project: ProjectView): void {
    this.renaming = new RenameProjectRequest(project);
    this.renameOpen = true;
  }

  /** Shows a project that was added, or that had the folder already, with a new conversation in it. */
  async #open(project: ProjectView): Promise<void> {
    conversationList.upsertProject(project);
    sidebarCollapse.reveal(project.id);
    await startNewConversation(project.id);
  }
}

/** The window's project actions. */
export const projectActions = new ProjectActions();
