// Pure path splitting and compressed folder-tree builder for file lists
// (Changes tab and Compare/Commit file trees), matching VS Code / GitLens
// compact directory chains (e.g. `plugins/git/ui`).

/** A file's basename and optional parent directory. */
export interface SplitPath {
  readonly directory: string;
  readonly fileName: string;
}

/** A folder node in the compressed tree. */
export interface FolderTreeNode<T> {
  readonly children: readonly FileTreeNode<T>[];
  readonly fileCount: number;
  readonly kind: 'folder';
  readonly label: string;
  readonly path: string;
}

/** A leaf file node in the compressed tree. */
export interface LeafTreeNode<T> {
  readonly item: T;
  readonly kind: 'file';
  readonly label: string;
  readonly path: string;
}

/** Either a compressed folder or a file entry. */
export type FileTreeNode<T> = FolderTreeNode<T> | LeafTreeNode<T>;

interface MutableFolder<T> {
  readonly files: LeafTreeNode<T>[];
  readonly folders: Map<string, MutableFolder<T>>;
  readonly name: string;
  readonly path: string;
}

/** Splits a repository-relative path into its filename and parent directory. */
export function splitPath(path: string): SplitPath {
  const slash = path.lastIndexOf('/');
  if (slash === -1) {
    return { directory: '', fileName: path };
  }
  return {
    directory: path.slice(0, slash),
    fileName: path.slice(slash + 1),
  };
}

/** Compresses single-child folder chains (`a` -> `b` -> `c` becomes `a/b/c`). */
function finalizeFolder<T>(folder: MutableFolder<T>): FolderTreeNode<T> {
  let current = folder;
  const labels = [current.name];

  while (current.files.length === 0) {
    if (current.folders.size !== 1) {
      break;
    }
    const only = current.folders.values().next().value;
    if (only === undefined) {
      break;
    }
    labels.push(only.name);
    current = only;
  }

  const childFolders = [...current.folders.values()]
    .map(finalizeFolder)
    .toSorted((a, b) => a.label.localeCompare(b.label));
  const childFiles = current.files.toSorted((a, b) => a.label.localeCompare(b.label));
  const children: FileTreeNode<T>[] = [...childFolders, ...childFiles];

  let fileCount = childFiles.length;
  for (const child of childFolders) {
    fileCount += child.fileCount;
  }

  return {
    children,
    fileCount,
    kind: 'folder',
    label: labels.join('/'),
    path: current.path,
  };
}

/** Builds a sorted, path-compressed tree from repository-relative items. */
export function buildFileTree<T>(items: readonly T[], pathOf: (item: T) => string): FileTreeNode<T>[] {
  const root: MutableFolder<T> = {
    files: [],
    folders: new Map(),
    name: '',
    path: '',
  };

  for (const item of items) {
    const fullPath = pathOf(item);
    const parts = fullPath.split('/').filter((part) => part.length > 0);
    if (parts.length === 0) {
      continue;
    }
    let cursor = root;
    for (let index = 0; index < parts.length - 1; index += 1) {
      const part = parts[index];
      const nextPath = cursor.path.length === 0 ? part : `${cursor.path}/${part}`;
      let child = cursor.folders.get(part);
      if (child === undefined) {
        child = { files: [], folders: new Map(), name: part, path: nextPath };
        cursor.folders.set(part, child);
      }
      cursor = child;
    }
    const fileName = parts.at(-1) ?? fullPath;
    cursor.files.push({
      item,
      kind: 'file',
      label: fileName,
      path: fullPath,
    });
  }

  const topFolders = [...root.folders.values()].map(finalizeFolder).toSorted((a, b) => a.label.localeCompare(b.label));
  const topFiles = root.files.toSorted((a, b) => a.label.localeCompare(b.label));
  return [...topFolders, ...topFiles];
}

/** Collects all file paths under a tree node, for folder-level stage/unstage actions. */
export function collectTreePaths<T>(node: FileTreeNode<T>): string[] {
  if (node.kind === 'file') {
    return [node.path];
  }
  const paths: string[] = [];
  for (const child of node.children) {
    paths.push(...collectTreePaths(child));
  }
  return paths;
}
