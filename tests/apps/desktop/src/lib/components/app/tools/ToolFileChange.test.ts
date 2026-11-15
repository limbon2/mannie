import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import ToolFileChange from '../../../../../../../../apps/desktop/src/lib/components/app/tools/ToolFileChange.svelte';
import ToolFileSummary from '../../../../../../../../apps/desktop/src/lib/components/app/tools/ToolFileSummary.svelte';
import type { SavedFileChange } from '../../../../../../../../apps/desktop/src/lib/utils/tools-file-changes.js';

function file(): SavedFileChange {
  return {
    path: 'C:\\work\\main.rs',
    change: {
      added: 13,
      removed: 8,
      truncated: false,
      hunks: [
        {
          oldStart: 10,
          oldLines: 1,
          newStart: 10,
          newLines: 1,
          lines: [
            {
              kind: 'removed',
              old: 10,
              new: null,
              text: '<script>old()</script>',
              noNewline: false,
              textTruncated: false,
            },
            { kind: 'added', old: null, new: 10, text: 'new()', noNewline: true, textTruncated: false },
          ],
        },
      ],
    },
  };
}

describe('saved file changes', () => {
  it('shows the filename and separate counts with accessible descriptions', () => {
    const body = render(ToolFileSummary, { props: { file: file() } }).body;
    expect(body).toContain('main.rs');
    expect(body).toContain('+13');
    expect(body).toContain('−8');
    expect(body).toContain('13 lines added');
    expect(body).toContain('8 lines removed');
    expect(body).toContain('title="File: C:\\work\\main.rs"');
  });

  it('renders historical text safely with both line numbers and newline markers', () => {
    const body = render(ToolFileChange, { props: { file: file() } }).body;
    expect(body).toContain('C:\\work\\main.rs');
    expect(body).toContain('data-file-line="removed"');
    expect(body).toContain('data-file-line="added"');
    expect(body).toContain('Old line 10');
    expect(body).toContain('New line 10');
    expect(body).toContain('&lt;script>old()&lt;/script>');
    expect(body).not.toContain('<script>old()');
    expect(body).toContain('No newline at end of file');
    expect(body).toContain('tabindex="0"');
    expect(body).not.toContain('This preview is shortened');
  });

  it('distinguishes unchanged files from shortened previews', () => {
    const empty = file();
    empty.change = { added: 0, removed: 0, hunks: [], truncated: false };
    const body = render(ToolFileChange, { props: { file: empty } }).body;
    expect(body).toContain('No text changes');
    expect(body).not.toContain('role="region"');
    const shortened = file();
    shortened.change.truncated = true;
    shortened.change.hunks[0].lines[1].textTruncated = true;
    const preview = render(ToolFileChange, { props: { file: shortened } }).body;
    expect(preview).toContain('Line shortened');
    expect(preview).toContain('The counts include the entire change');
  });
});
