// The files of a paste or a drop on the message box (ADR-0033, decision 7):
// a paste that carries text stays a text paste, a drag counts only when it
// carries files, and each file goes to the first source that takes it.
import { describe, expect, it } from 'vitest';

import {
  assignFiles,
  carriesFiles,
  pastedFiles,
  type Transfer,
} from '../../../../../../apps/desktop/src/lib/utils/transfer-files.js';

function file(name: string, type: string): File {
  return new File([new Uint8Array([1, 2, 3])], name, { type });
}

function transfer(files: File[], text = ''): Transfer {
  return {
    files,
    getData: (format) => (format === 'text/plain' ? text : ''),
    types: files.length > 0 ? ['Files'] : ['text/plain'],
  };
}

describe('pastedFiles', () => {
  it('takes the files of a paste without text', () => {
    const screenshot = file('image.png', 'image/png');

    expect(pastedFiles(transfer([screenshot]))).toEqual([screenshot]);
  });

  it('takes no file from a paste that carries text, as an office application’s copy does', () => {
    const picture = file('image.png', 'image/png');

    expect(pastedFiles(transfer([picture], 'A table cell'))).toEqual([]);
  });
});

describe('carriesFiles', () => {
  it('counts only drags with files', () => {
    expect(carriesFiles({ types: ['Files'] })).toBe(true);
    expect(carriesFiles({ types: ['text/plain', 'text/uri-list'] })).toBe(false);
    expect(carriesFiles(null)).toBe(false);
  });
});

describe('assignFiles', () => {
  it('gives each file to the first source that takes it, in the order of the sources', () => {
    const images = { id: 'images', takes: (candidate: File) => candidate.type.startsWith('image/') };
    const pngs = { id: 'pngs', takes: (candidate: File) => candidate.type === 'image/png' };
    const png = file('a.png', 'image/png');
    const jpeg = file('b.jpg', 'image/jpeg');
    const text = file('notes.txt', 'text/plain');

    const { assigned, refused } = assignFiles([png, text, jpeg], [images, pngs]);

    expect(assigned.get(images)).toEqual([png, jpeg]);
    expect(assigned.has(pngs)).toBe(false);
    expect(refused).toEqual([text]);
  });
});
