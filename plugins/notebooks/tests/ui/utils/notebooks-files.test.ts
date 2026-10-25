// The files that a drop adds: the plugin takes PDF, Markdown and text files
// by their extension, a drag counts only when it carries files, and the
// bytes cross IPC as base64, also for a file larger than one chunk.
import { describe, expect, it } from 'vitest';

import { carriesFiles, readAsBase64, takesDocument } from '../../../ui/utils/notebooks-files.js';

describe('the files of a drop', () => {
  it('are taken by their extension, without regard to case', () => {
    expect(['Notes.PDF', 'a.md', 'b.markdown', 'c.txt'].map(takesDocument)).toEqual([true, true, true, true]);
    expect(['report.docx', 'pdf', 'image.png'].map(takesDocument)).toEqual([false, false, false]);
  });

  it('count only when the drag carries files', () => {
    expect(carriesFiles(null)).toBe(false);
    expect(carriesFiles({ types: ['Files'] } as unknown as DataTransfer)).toBe(true);
    expect(carriesFiles({ types: ['text/plain'] } as unknown as DataTransfer)).toBe(false);
  });

  it('cross IPC as base64', async () => {
    const bytes = Uint8Array.from({ length: 100_000 }, (_value, index) => (index * 31) % 256);

    const encoded = await readAsBase64(new Blob([bytes]));

    expect(encoded).toBe(Buffer.from(bytes).toString('base64'));
  });
});
