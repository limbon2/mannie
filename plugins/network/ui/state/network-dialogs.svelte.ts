// Coordinates modal dialogs and sidebar tree expansion state for the Network
// Client plugin UI.
import type { CollectionView, FolderView } from '../generated/bindings.js';
import type { CollectionDialogState, DeleteDialogState, FolderDialogState } from '../types/network.js';

/** Modal dialog and sidebar expansion state for the Network Client workbench. */
export class NetworkDialogsState {
  collectionDialog: CollectionDialogState | null = $state(null);
  cookieJarOpen = $state(false);
  deleteDialog: DeleteDialogState | null = $state(null);
  environmentsCollectionId: string | null = $state(null);
  folderDialog: FolderDialogState | null = $state(null);
  globalVariablesOpen = $state(false);
  importOpen = $state(false);
  openNodes: Partial<Record<string, boolean>> = $state({});

  isNodeOpen(id: string, fallback = true): boolean {
    const known = this.openNodes[id];
    return known ?? fallback;
  }

  setNodeOpen(id: string, open: boolean): void {
    this.openNodes = { ...this.openNodes, [id]: open };
  }

  startCreateCollection(): void {
    this.collectionDialog = { mode: 'create' };
  }

  startEditCollection(collection: CollectionView): void {
    this.collectionDialog = { collection, mode: 'edit' };
  }

  closeCollectionDialog(): void {
    this.collectionDialog = null;
  }

  openImport(): void {
    this.importOpen = true;
  }

  closeImport(): void {
    this.importOpen = false;
  }

  openEnvironments(collectionId: string): void {
    this.environmentsCollectionId = collectionId;
  }

  closeEnvironments(): void {
    this.environmentsCollectionId = null;
  }

  openGlobalVariables(): void {
    this.globalVariablesOpen = true;
  }

  closeGlobalVariables(): void {
    this.globalVariablesOpen = false;
  }

  openCookieJar(): void {
    this.cookieJarOpen = true;
  }

  closeCookieJar(): void {
    this.cookieJarOpen = false;
  }

  startCreateFolder(collectionId: string): void {
    this.folderDialog = { collectionId, mode: 'create' };
  }

  startRenameFolder(folder: FolderView): void {
    this.folderDialog = { folder, mode: 'rename' };
  }

  closeFolderDialog(): void {
    this.folderDialog = null;
  }

  confirmDelete(target: DeleteDialogState): void {
    this.deleteDialog = target;
  }

  closeDeleteDialog(): void {
    this.deleteDialog = null;
  }

  closeAll(): void {
    this.collectionDialog = null;
    this.cookieJarOpen = false;
    this.deleteDialog = null;
    this.environmentsCollectionId = null;
    this.folderDialog = null;
    this.globalVariablesOpen = false;
    this.importOpen = false;
  }
}

/** Shared dialog coordinator for the Network Client overlay. */
export const networkDialogs = new NetworkDialogsState();
