// The files of a status (plugins/git/docs/tools.md, git_status), rendered
// on the server side: the conflicted, staged, unstaged and untracked files
// under their headings, a file with staged and unstaged changes in both
// groups, the files left out, and buttons only where the panel offers a
// choice.
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import EntryList from '../../../../ui/components/cards/EntryList.svelte';
import type { Entry } from '../../../../ui/generated/bindings.js';

const ENTRIES: Entry[] = [
  { conflict: 'both_changed', kind: 'conflicted', path: 'src/merge.rs', staged: 'U', submodule: false, unstaged: 'U' },
  { kind: 'changed', path: 'src/lib.rs', staged: 'M', submodule: false, unstaged: 'M' },
  { copied: false, kind: 'renamed', original: 'old.rs', path: 'new.rs', staged: 'R', submodule: false, unstaged: '.' },
  { kind: 'untracked', path: 'notes.txt', staged: '.', submodule: false, unstaged: '.' },
];

describe('EntryList', () => {
  it('groups the files as git_status lists them for the model', () => {
    const body = render(EntryList, { props: { entries: ENTRIES, total: 4 } }).body;

    expect(body).toContain('Conflicted');
    expect(body).toContain('both changed: src/merge.rs');
    expect(body).toContain('Staged');
    expect(body).toContain('renamed: new.rs from old.rs');
    expect(body).toContain('Not staged');
    expect(body.match(/modified: src\/lib\.rs/gu)).toHaveLength(2);
    expect(body).toContain('Untracked');
    expect(body).toContain('notes.txt');
    expect(body).not.toContain('<button');
  });

  it('offers each file as a button where the panel chooses files, and counts the files left out', () => {
    const body = render(EntryList, { props: { choose: () => undefined, entries: ENTRIES, total: 7 } }).body;

    expect(body.match(/<button/gu)).toHaveLength(5);
    expect(body).toContain('and 3 more files');
  });

  it('says that nothing is to commit when no file changed', () => {
    const body = render(EntryList, { props: { entries: [], total: 0 } }).body;

    expect(body).toContain('Nothing to commit');
  });
});
