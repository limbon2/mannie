// Compressed folder-tree builder and path splitter tests for the Changes
// file list (plugins/git/docs/panel.md).
import { describe, expect, it } from 'vitest';

import { buildFileTree, collectTreePaths, splitPath } from '../../../ui/utils/git-tree.js';

describe('splitPath', () => {
  it('splits nested and root-level paths cleanly', () => {
    expect(splitPath('README.md')).toEqual({ directory: '', fileName: 'README.md' });
    expect(splitPath('plugins/git/ui/GitPanel.svelte')).toEqual({
      directory: 'plugins/git/ui',
      fileName: 'GitPanel.svelte',
    });
  });
});

describe('buildFileTree and collectTreePaths', () => {
  it('compresses single-child folder chains and sorts folders before files', () => {
    const paths = [
      'Cargo.toml',
      'plugins/git/ui/components/GitPanel.svelte',
      'plugins/git/ui/components/GitControl.svelte',
      'plugins/git/messages/en.json',
      'README.md',
    ];

    const tree = buildFileTree(paths, (path) => path);

    expect(tree.map((node) => `${node.kind}:${node.label}`)).toEqual([
      'folder:plugins/git',
      'file:Cargo.toml',
      'file:README.md',
    ]);

    const gitFolder = tree[0];
    expect(gitFolder.kind).toBe('folder');
    if (gitFolder.kind === 'folder') {
      expect(gitFolder.fileCount).toBe(3);
      expect(gitFolder.children.map((node) => `${node.kind}:${node.label}`)).toEqual([
        'folder:messages',
        'folder:ui/components',
      ]);
      expect(collectTreePaths(gitFolder)).toEqual([
        'plugins/git/messages/en.json',
        'plugins/git/ui/components/GitControl.svelte',
        'plugins/git/ui/components/GitPanel.svelte',
      ]);
    }
  });
});
