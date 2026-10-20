// A diff as a card shows it (ADR-0039; plugins/git/docs/panel.md),
// rendered on the server side: each file with its line counts, a binary
// file and a rename in words, and each line of a patch with its numbers, a
// marker in text as well as in color, and words for screen readers.
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import DiffView from '../../../../ui/components/cards/DiffView.svelte';
import type { FilePatch, FileStat, Hunk } from '../../../../ui/generated/bindings.js';

const HUNK: Hunk = {
  header: '@@ -1,2 +1,2 @@ fn main()',
  lines: [
    { kind: 'context', new: 1, noNewline: false, old: 1, text: 'fn main() {' },
    { kind: 'removed', new: null, noNewline: false, old: 2, text: '    old_call();' },
    { kind: 'added', new: 2, noNewline: true, old: null, text: '    new_call();' },
  ],
};

const PARSER: FileStat = { added: 1, original: null, path: 'src/parser.rs', removed: 1 };

function shown(files: readonly FileStat[], patches: readonly FilePatch[], open = true): string {
  return render(DiffView, { props: { files, open, patches } }).body;
}

describe('DiffView', () => {
  it('shows each line of a patch with its marker, its numbers and words for screen readers', () => {
    const body = shown([PARSER], [{ hunks: [HUNK], path: 'src/parser.rs' }]);

    expect(body).toContain('src/parser.rs');
    expect(body).toContain('+1 −1');
    expect(body).toContain('@@ -1,2 +1,2 @@ fn main()');
    expect(body).toMatch(/data-git-line="removed"[^]*?-<\/span>[^]*?removed<\/span>[^]*?old_call\(\);/u);
    expect(body).toMatch(/data-git-line="added"[^]*?\+<\/span>[^]*?added<\/span>[^]*?new_call\(\);/u);
    expect(body).toContain('no line break at the end');
  });

  it('says that a file is binary, and names a renamed file’s old path', () => {
    const body = shown(
      [
        { added: null, original: null, path: 'logo.png', removed: null },
        { added: 0, original: 'old.rs', path: 'new.rs', removed: 0 },
      ],
      [],
      false,
    );

    expect(body).toContain('Binary file');
    expect(body).toContain('from old.rs');
  });

  it('says how many files’ patches are only in the text that the agent read', () => {
    const files = [PARSER, { ...PARSER, path: 'src/lexer.rs' }, { ...PARSER, path: 'src/main.rs' }];

    const cut = shown(files, [{ hunks: [HUNK], path: 'src/parser.rs' }]);
    const statOnly = shown(files, []);

    expect(cut).toContain('The patches of 2 more files are in the text that the agent read.');
    expect(statOnly).not.toContain('in the text that the agent read');
  });
});
