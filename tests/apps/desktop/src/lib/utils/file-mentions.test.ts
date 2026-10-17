// The words of the core's mentions of files and folders (docs/features/chat/
// mentions.md): what the model got of a mention, the warning, the refusals
// and the variants.
import { describe, expect, it } from 'vitest';

import type { MentionSummary } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  fileMentionRefusal,
  fileMentionSummary,
  fileMentionVariantName,
  fileMentionWarning,
} from '../../../../../../apps/desktop/src/lib/utils/file-mentions.js';

function summary(code: string, values: Record<string, number> = {}): MentionSummary {
  return { code, values };
}

describe('fileMentionSummary', () => {
  it('says which lines of a file the model got', () => {
    expect(fileMentionSummary(summary('lines', { first: 1, last: 1, total: 1 }))).toBe('1 line');
    expect(fileMentionSummary(summary('lines', { first: 1, last: 120, total: 120 }))).toBe('120 lines');
    expect(fileMentionSummary(summary('lines', { first: 1, last: 1600, total: 5000 }))).toBe(
      'lines 1 to 1,600 of 5,000',
    );
    expect(fileMentionSummary(summary('lines', { first: 1, last: 0, total: 30 }))).toBe('none of its 30 lines fit');
    expect(fileMentionSummary(summary('lines', { first: 1, last: 0, total: 0 }))).toBe('empty file');
  });

  it('says how many entries of a folder the model got', () => {
    expect(fileMentionSummary(summary('entries', { shown: 34 }))).toBe('34 entries');
    expect(fileMentionSummary(summary('entries', { shown: 0 }))).toBe('empty folder');
    expect(fileMentionSummary(summary('entries_cut', { shown: 300 }))).toBe('the first 300 entries');
  });

  it('says why the model got only the name', () => {
    expect(fileMentionSummary(summary('not_text'))).toBe('not text, so only its name was sent');
    expect(fileMentionSummary(summary('too_large', { limitMebibytes: 16 }))).toBe(
      'larger than 16 MiB, so only its name was sent',
    );
    expect(fileMentionSummary(summary('needs_approval'))).toBe(
      'reading it needs your approval, so only its name was sent',
    );
    expect(fileMentionSummary(summary('unreadable'))).toBe('could not be read, so only its name was sent');
    expect(fileMentionSummary(summary('not_found'))).toBe('does not exist');
    expect(fileMentionSummary({ code: 'lines' })).toBe('empty file');
    expect(fileMentionSummary(summary('from_the_future'))).toBeNull();
  });
});

describe('the words of the files kind', () => {
  it('warns about secrets and names the variants', () => {
    expect(fileMentionWarning('secret')).toBe('may hold secrets');
    expect(fileMentionWarning('other')).toBeNull();
    expect(fileMentionVariantName('file')).toBe('File');
    expect(fileMentionVariantName('folder')).toBe('Folder');
    expect(fileMentionVariantName('note')).toBeNull();
  });

  it('words the refusals of the files provider', () => {
    expect(fileMentionRefusal('outside_workspace')).toBe("The mention names a path outside the project's folder.");
    expect(fileMentionRefusal('no_workspace')).toBe('Files can be mentioned only in the conversations of a project.');
    expect(fileMentionRefusal('folder_unreadable')).toBe(
      "The project's folder could not be read. Check that it still exists.",
    );
    expect(fileMentionRefusal('unknown')).toBeNull();
  });
});
