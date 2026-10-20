// The Markdown parser of model output (ADR-0021): only Mannie's node kinds come
// out, raw HTML stays text, URLs stay data, and hostile or pathological text
// neither escapes the tree nor freezes the window.
import { threadCpuUsage } from 'node:process';

import { describe, expect, it } from 'vitest';

import {
  finishedBlocksEnd,
  isCheapToParse,
  MAX_MARKDOWN_BYTES,
  parseMarkdown,
  type RenderedMarkdown,
  type RenderNode,
  StreamingMarkdown,
} from '../../../../../../apps/desktop/src/lib/utils/markdown.js';
import { hostileHtml, hostileMarkdown, hostileNavigations } from '../../../../../e2e/support/hostile-html.js';

const ORIGIN = 'http://127.0.0.1:9';

const ALLOWED_KINDS = new Set([
  'blockquote',
  'break',
  'code',
  'delete',
  'emphasis',
  'heading',
  'image',
  'inline_code',
  'link',
  'list',
  'paragraph',
  'strong',
  'table',
  'text',
  'thematic_break',
]);

/** Every node of a tree, depth first. */
function allNodes(nodes: readonly RenderNode[]): RenderNode[] {
  return nodes.flatMap((node) => {
    switch (node.kind) {
      case 'blockquote':
      case 'delete':
      case 'emphasis':
      case 'heading':
      case 'link':
      case 'paragraph':
      case 'strong': {
        return [node, ...allNodes(node.children)];
      }
      case 'list': {
        return [node, ...node.items.flatMap((item) => allNodes(item.children))];
      }
      case 'table': {
        return [node, ...[node.head, ...node.body].flat().flatMap((cell) => allNodes(cell))];
      }
      case 'break':
      case 'code':
      case 'image':
      case 'inline_code':
      case 'text':
      case 'thematic_break': {
        return [node];
      }
    }
  });
}

/** The lists of a tree, ordered or not. */
function lists(nodes: readonly RenderNode[], ordered: boolean): RenderNode[] {
  return nodes.filter((node) => node.kind === 'list').filter((node) => node.ordered === ordered);
}

/** The addresses of a tree's links, in order. */
function linkUrls(nodes: readonly RenderNode[]): string[] {
  return nodes.flatMap((node) => (node.kind === 'link' ? [node.url] : []));
}

function nodesOf(rendered: RenderedMarkdown): RenderNode[] {
  if (rendered.kind === 'plain') {
    throw new Error('expected Markdown, got plain text');
  }
  return allNodes(rendered.nodes);
}

describe('parseMarkdown', () => {
  it('turns Markdown into the render tree', () => {
    const rendered = parseMarkdown(
      [
        '# Title',
        '',
        'Some *emphasis*, **strength**, ~~strike~~ and `code`.  ',
        'A second line.',
        '',
        '> quoted',
        '',
        '1. first',
        '2. second',
        '',
        '- [x] done',
        '- [ ] open',
        '',
        '| left | right |',
        '| :--- | ----: |',
        '| a | b |',
        '',
        '```rust',
        'fn main() {}',
        '```',
        '',
        '---',
        '',
        '[a link](https://example.com/page) and ![a picture](https://example.com/p.png)',
      ].join('\n'),
    );

    expect(rendered.kind).toBe('markdown');
    const nodes = nodesOf(rendered);
    expect(nodes.find((node) => node.kind === 'heading')).toMatchObject({ level: 1 });
    expect(nodes.filter((node) => node.kind === 'break')).toHaveLength(1);
    expect(lists(nodes, true)[0]).toMatchObject({ start: 1 });
    expect(lists(nodes, false)[0]).toMatchObject({
      items: [{ checked: true }, { checked: false }],
    });
    expect(nodes.find((node) => node.kind === 'table')).toMatchObject({ align: ['left', 'right'] });
    expect(nodes.find((node) => node.kind === 'code')).toEqual({
      kind: 'code',
      language: 'rust',
      value: 'fn main() {}',
    });
    expect(nodes.find((node) => node.kind === 'link')).toMatchObject({ url: 'https://example.com/page' });
    expect(nodes.find((node) => node.kind === 'image')).toEqual({
      description: 'a picture',
      kind: 'image',
      url: 'https://example.com/p.png',
    });
    expect(nodes.some((node) => node.kind === 'thematic_break')).toBe(true);
  });

  it('keeps every hostile payload inside the node kinds Mannie renders', () => {
    const payloads = [...hostileHtml(ORIGIN), ...hostileNavigations(ORIGIN), ...hostileMarkdown(ORIGIN)];

    for (const payload of payloads) {
      const nodes = nodesOf(parseMarkdown(payload));
      const kinds = new Set(nodes.map((node) => node.kind));

      expect(
        [...kinds].filter((kind) => !ALLOWED_KINDS.has(kind)),
        payload,
      ).toEqual([]);
    }
  });

  it('shows raw HTML as the text it is', () => {
    const nodes = nodesOf(parseMarkdown('<script>window.compromised = true;</script>\n\nText <b>bold</b>'));
    const texts = nodes.flatMap((node) => (node.kind === 'text' ? [node.value] : []));

    expect(texts).toContain('<script>window.compromised = true;</script>');
    expect(texts).toContain('<b>');
    expect(nodes.every((node) => node.kind !== 'strong')).toBe(true);
  });

  it('decodes character references into text and never back into markup', () => {
    const nodes = nodesOf(parseMarkdown('&lt;script&gt; &amp;amp; &#x3C;img&#x3E;'));
    const text = nodes.flatMap((node) => (node.kind === 'text' ? [node.value] : [])).join('');

    expect(text).toBe('<script> &amp; <img>');
  });

  it('resolves reference links and shows an image inside a link as its description', () => {
    const nodes = nodesOf(
      parseMarkdown(
        '[ref][target] [![inner](https://x.example/i.png)](https://x.example/)\n\n[target]: https://example.com/ref',
      ),
    );

    expect(linkUrls(nodes)).toEqual(['https://example.com/ref', 'https://x.example/']);
    expect(nodes.some((node) => node.kind === 'image')).toBe(false);
  });

  it('shows deep nesting and very long text as plain text', () => {
    const deep = `${'>'.repeat(40)} deep`;
    const long = 'a'.repeat(MAX_MARKDOWN_BYTES + 1);
    const longInBytes = 'é'.repeat(MAX_MARKDOWN_BYTES / 2 + 1);

    expect(parseMarkdown(deep)).toEqual({ kind: 'plain', text: deep });
    expect(parseMarkdown(long).kind).toBe('plain');
    expect(parseMarkdown(longInBytes).kind).toBe('plain');
    expect(parseMarkdown(`${'>'.repeat(10)} fine`).kind).toBe('markdown');
  });

  it('shows nesting beyond the limit as plain text', () => {
    const nested = `${'- '.repeat(100)}a`;

    expect(parseMarkdown(nested)).toEqual({ kind: 'plain', text: nested });
  });
});

describe('isCheapToParse', () => {
  const size = 150_000;
  const dense = [
    '*a'.repeat(size / 2),
    '~~a~~ '.repeat(size / 6),
    '['.repeat(size),
    '[a]('.repeat(size / 4),
    `|a|b|\n|-|-|\n${'|x|y|\n'.repeat(1_000)}`,
    `${'> '.repeat(40)}a`,
    `${'- '.repeat(40)}a`,
    `${' '.repeat(200)}- deeply indented`,
    'a'.repeat(40_000),
  ];

  it('sends dense or long text to the worker', () => {
    for (const text of dense) {
      expect(isCheapToParse(text), `${text.slice(0, 12)}…`).toBe(false);
    }
  });

  it('keeps ordinary replies on the main thread', () => {
    const reply = [
      '## Plan',
      '',
      'Some **bold** and _emphasis_ with `code` and a [link](https://example.com).',
      '',
      '1. First',
      '   - nested',
      '2. Second',
      '',
      '| a | b |',
      '|---|---|',
      '| 1 | 2 |',
      '',
      '```ts',
      'const total = price * count;',
      '```',
    ].join('\n');

    expect(isCheapToParse(reply)).toBe(true);
    expect(isCheapToParse(reply.repeat(20))).toBe(true);
  });

  it('bounds the time of every text it accepts', () => {
    const worstWithinBounds = [
      '*a'.repeat(500),
      '**a** '.repeat(250),
      '~~a~~ '.repeat(250),
      '[a](b) '.repeat(1_000),
      '[^a]'.repeat(1_000),
      `|a|b|c|d|e|f|\n|-|-|-|-|-|-|\n${'|x|y|z|w|v|u|\n'.repeat(148)}`,
      Array.from({ length: 64 }, (_, level) => `${' '.repeat(2 * level)}- a`).join('\n'),
      `${'> '.repeat(32)}a`,
      'Some **bold** and _em_ text with `code` and [link](http://x.y). '.repeat(160),
    ];

    for (const text of worstWithinBounds) {
      expect(isCheapToParse(text), `${text.slice(0, 12)}…`).toBe(true);
      // Count this thread's work, without time when another CI process has
      // the processor. The complexity limit keeps the same 250 ms budget.
      const started = threadCpuUsage();
      parseMarkdown(text);
      const elapsed = threadCpuUsage(started);
      const took = (elapsed.user + elapsed.system) / 1_000;
      expect(took, `${text.slice(0, 12)}…`).toBeLessThan(250);
    }
  });
});

describe('finishedBlocksEnd', () => {
  it('ends after the last blank line', () => {
    expect(finishedBlocksEnd('one\n\ntwo\n\nthr')).toBe('one\n\ntwo\n\n'.length);
    expect(finishedBlocksEnd('one paragraph')).toBe(0);
  });

  it('never ends inside a fenced code block', () => {
    const text = 'intro\n\n```\ncode\n\nmore code\n';

    expect(finishedBlocksEnd(text)).toBe('intro\n\n'.length);
    expect(finishedBlocksEnd(`${text}\`\`\`\n\nafter`)).toBe(`${text}\`\`\`\n\n`.length);
  });
});

describe('StreamingMarkdown', () => {
  it('ends with the tree of the whole text, however the text arrived', () => {
    const text = [
      '# Heading',
      '',
      'A paragraph with **bold** text.',
      '',
      '```js',
      'const a = 1;',
      '',
      'const b = 2;',
      '```',
      '',
      '- one',
      '- two',
      '',
      'The end.',
    ].join('\n');
    const streaming = new StreamingMarkdown();

    for (let end = 1; end <= text.length; end += 7) {
      streaming.update(text.slice(0, end));
    }

    expect(streaming.update(text)).toEqual(parseMarkdown(text));
  });

  it('starts over when the text is replaced', () => {
    const streaming = new StreamingMarkdown();
    streaming.update('first\n\nsecond');

    expect(streaming.update('other')).toEqual(parseMarkdown('other'));
  });
});
