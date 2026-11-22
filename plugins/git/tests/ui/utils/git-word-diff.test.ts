// Pure intra-line word diff, split-row alignment and single-hunk patch
// builder tests (plugins/git/docs/panel.md).
import { describe, expect, it } from 'vitest';

import type { Hunk, Line } from '../../../ui/generated/bindings.js';
import { annotateHunkLines, buildHunkPatch, buildSplitRows, diffLineWords } from '../../../ui/utils/git-word-diff.js';

describe('diffLineWords', () => {
  it('highlights only the changed token between paired lines', () => {
    const result = diffLineWords('const value = oldCall(42);', 'const value = newCall(42);');

    expect(result.removed).toEqual([
      { emphasized: false, text: 'const value = ' },
      { emphasized: true, text: 'oldCall' },
      { emphasized: false, text: '(42);' },
    ]);
    expect(result.added).toEqual([
      { emphasized: false, text: 'const value = ' },
      { emphasized: true, text: 'newCall' },
      { emphasized: false, text: '(42);' },
    ]);
  });

  it('returns plain unemphasized segments for identical lines and very long lines', () => {
    const identical = diffLineWords('unchanged', 'unchanged');
    expect(identical.removed).toEqual([{ emphasized: false, text: 'unchanged' }]);
    expect(identical.added).toEqual([{ emphasized: false, text: 'unchanged' }]);

    const hugeLeft = 'a'.repeat(2500);
    const hugeRight = 'b'.repeat(2500);
    const capped = diffLineWords(hugeLeft, hugeRight);
    expect(capped.removed).toEqual([{ emphasized: false, text: hugeLeft }]);
    expect(capped.added).toEqual([{ emphasized: false, text: hugeRight }]);
  });
});

describe('annotateHunkLines and buildSplitRows', () => {
  const lines: Line[] = [
    { kind: 'context', new: 1, noNewline: false, old: 1, text: 'fn main() {' },
    { kind: 'removed', new: null, noNewline: false, old: 2, text: '    let a = 1;' },
    { kind: 'removed', new: null, noNewline: false, old: 3, text: '    extra();' },
    { kind: 'added', new: 2, noNewline: false, old: null, text: '    let a = 2;' },
    { kind: 'context', new: 3, noNewline: false, old: 4, text: '}' },
  ];

  it('annotates paired removed/added blocks while leaving unpaired lines unemphasized', () => {
    const annotated = annotateHunkLines(lines);

    expect(annotated).toHaveLength(5);
    const hasRemovedToken = (text: string) =>
      annotated[1]?.segments.some((segment) => {
        const isTarget = segment.text === text;
        return segment.emphasized && isTarget;
      });
    const hasAddedToken = (text: string) =>
      annotated[3]?.segments.some((segment) => {
        const isTarget = segment.text === text;
        return segment.emphasized && isTarget;
      });
    expect(hasRemovedToken('1')).toBe(true);
    expect(hasAddedToken('2')).toBe(true);
    // Second removed line has no paired added line.
    expect(annotated[2]?.segments).toEqual([{ emphasized: false, text: '    extra();' }]);
  });

  it('aligns context, paired changes and unpaired deletions into side-by-side rows', () => {
    const rows = buildSplitRows(lines);

    expect(rows).toHaveLength(4);
    expect(rows[0]?.left?.line.text).toBe('fn main() {');
    expect(rows[0]?.right?.line.text).toBe('fn main() {');
    expect(rows[1]?.left?.line.text).toBe('    let a = 1;');
    expect(rows[1]?.right?.line.text).toBe('    let a = 2;');
    expect(rows[2]?.left?.line.text).toBe('    extra();');
    expect(rows[2]?.right).toBeNull();
    expect(rows[3]?.left?.line.text).toBe('}');
  });
});

describe('buildHunkPatch', () => {
  it('builds a valid git apply patch including no-newline markers and rename paths', () => {
    const hunk: Hunk = {
      header: '@@ -1,2 +1,2 @@',
      lines: [
        { kind: 'context', new: 1, noNewline: false, old: 1, text: 'alpha' },
        { kind: 'removed', new: null, noNewline: false, old: 2, text: 'beta' },
        { kind: 'added', new: 2, noNewline: true, old: null, text: 'gamma' },
      ],
    };

    const patch = buildHunkPatch('src/new.txt', hunk, 'src/old.txt');

    expect(patch).toBe(
      [
        'diff --git a/src/old.txt b/src/new.txt',
        '--- a/src/old.txt',
        '+++ b/src/new.txt',
        '@@ -1,2 +1,2 @@',
        ' alpha',
        '-beta',
        '+gamma',
        '\\ No newline at end of file',
        '',
      ].join('\n'),
    );
  });
});
