import { describe, expect, it } from 'vitest';

import { isFileLink, isInlineFilePath } from '../../../../../../apps/desktop/src/lib/utils/file-links.js';
import { parseMarkdown } from '../../../../../../apps/desktop/src/lib/utils/markdown.js';

describe('local references in Markdown', () => {
  it.each([
    'src/main.rs',
    'src/',
    './src',
    '../main.rs',
    'README.md',
    '.gitignore',
    '.env.local',
    '/home/me/file.rs',
    'C:\\Projects\\app',
    'C:/Program Files/app/main.rs:42:7',
    'src/main.rs#L42',
    'file:///tmp/a%20b.rs',
    'file:///D:/Program%20Files/app/main.rs:42:7',
    'file://localhost/home/me/файл.rs#L42',
    'FILE:///tmp/a.rs',
    'папка/файл.rs',
  ])('recognizes an entire inline path: %s', (path) => {
    expect(isInlineFilePath(path)).toBe(true);
  });

  it.each([
    'hello',
    'cargo test',
    './script --flag',
    '/bin/sh -c test',
    'cat src/main.rs',
    'git diff -- src/main.rs',
    'https://example.com',
    'javascript:alert(1)',
    'javascript:123',
    'mailto:123',
    'std::fs',
    'a/b + c',
    '*.rs',
    'foo(bar.rs)',
    'x = src/main.rs',
    'path\nnext',
    '[file](main.rs)',
    'main.rs;',
    ' path.rs',
    'path.rs ',
    'cite:notebooks/key',
  ])('leaves code and commands unchanged: %s', (text) => {
    expect(isInlineFilePath(text)).toBe(false);
  });

  it.each([
    'file:',
    'file:/',
    'file://',
    'file:///',
    'file:////',
    'file://localhost',
    'file://localhost/',
    'file://#L42',
    'file:///:42:7',
    'FILE://',
    `file:${'\\'.repeat(1)}`,
    `file:${'\\'.repeat(2)}`,
    `file:${'\\'.repeat(3)}`,
  ])('leaves a file URL prefix as inline code: %s', (value) => {
    expect(isInlineFilePath(value)).toBe(false);
    expect(parseMarkdown(`\`${value}\``)).toEqual({
      kind: 'markdown',
      nodes: [{ kind: 'paragraph', children: [{ kind: 'inline_code', value }] }],
    });
  });

  it('routes explicit paths separately from URLs and citations', () => {
    expect(isFileLink('src')).toBe(true);
    expect(isFileLink('my folder/a.rs')).toBe(true);
    expect(isFileLink('main.rs:42')).toBe(true);
    for (const value of [
      'https://example.com',
      'mailto:owner@example.com',
      'javascript:run()',
      'cite:kind/key',
      '#heading',
    ]) {
      expect(isFileLink(value), value).toBe(false);
    }
  });

  it('links inline paths once and never links prose or fenced code', () => {
    const result = parseMarkdown('[\x60src/main.rs\x60](src/main.rs)');
    expect(result).toEqual({
      kind: 'markdown',
      nodes: [
        {
          kind: 'paragraph',
          children: [
            {
              kind: 'link',
              url: 'src/main.rs',
              children: [{ kind: 'inline_code', value: 'src/main.rs' }],
            },
          ],
        },
      ],
    });
    expect(parseMarkdown('src/main.rs')).toEqual({
      kind: 'markdown',
      nodes: [
        {
          kind: 'paragraph',
          children: [
            {
              kind: 'text',
              value: 'src/main.rs',
            },
          ],
        },
      ],
    });
    expect(parseMarkdown('\x60\x60\x60\nmain.rs\n\x60\x60\x60')).toEqual({
      kind: 'markdown',
      nodes: [
        {
          kind: 'code',
          language: null,
          value: 'main.rs',
        },
      ],
    });
    expect(parseMarkdown('\x60src/main.rs\x60')).toEqual({
      kind: 'markdown',
      nodes: [
        {
          kind: 'paragraph',
          children: [
            {
              kind: 'link',
              url: 'src/main.rs',
              children: [{ kind: 'inline_code', value: 'src/main.rs' }],
            },
          ],
        },
      ],
    });
  });
});
