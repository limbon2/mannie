// The Markdown renderer's output (ADR-0021, decision 3): rendered on the
// server side, every hostile payload produces only markup that Mannie's
// components chose. No URL lands in an attribute, no raw element or event
// handler from the text survives, and the payload stays visible as text.
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import Markdown from '../../../../../../../../apps/desktop/src/lib/components/app/markdown/Markdown.svelte';
import { hostileHtml, hostileMarkdown, hostileNavigations } from '../../../../../../../e2e/support/hostile-html.js';

const ORIGIN = 'http://127.0.0.1:9';

/** Elements that the renderer never creates. */
const FORBIDDEN_ELEMENTS = new Set([
  'a',
  'audio',
  'base',
  'details',
  'embed',
  'form',
  'iframe',
  'img',
  'input',
  'link',
  'math',
  'meta',
  'noscript',
  'object',
  'script',
  'style',
  'svg',
  'template',
  'video',
]);

/** Attributes that could load, run or navigate. */
const FORBIDDEN_ATTRIBUTE =
  /\s(?:href|src|srcset|style|action|formaction|xlink:href|data|poster|background|on[a-z]+)\s*=/iu;

const LUCIDE_ICON = /<svg\b[^>]*class="[^"]*lucide[^"]*"[^>]*>.*?<\/svg>/gsu;

function renderMarkdown(text: string): string {
  return render(Markdown, { props: { text } }).body;
}

/**
 * The start tags of the HTML, without lucide's icons, which Mannie's
 * components add. Text escapes every `<`, so each literal `<` starts a tag.
 */
function startTags(html: string): string[] {
  return html.replaceAll(LUCIDE_ICON, '').match(/<[a-z][^>]*>/giu) ?? [];
}

function tagName(tag: string): string {
  return /^<([a-z][\w-]*)/iu.exec(tag)?.[1]?.toLowerCase() ?? '';
}

describe('Markdown', () => {
  it('renders every hostile payload as text, without forbidden elements or attributes', () => {
    const payloads = [...hostileHtml(ORIGIN), ...hostileNavigations(ORIGIN), ...hostileMarkdown(ORIGIN)];

    for (const payload of payloads) {
      const tags = startTags(renderMarkdown(payload));

      expect(
        tags.filter((tag) => FORBIDDEN_ELEMENTS.has(tagName(tag))),
        payload,
      ).toEqual([]);
      expect(
        tags.filter((tag) => FORBIDDEN_ATTRIBUTE.test(tag)),
        payload,
      ).toEqual([]);
    }
  });

  it('never puts a link or image address into the markup outside the text', () => {
    const html = renderMarkdown(`[remote](${ORIGIN}/link) ![tracker](${ORIGIN}/tracker.png) <${ORIGIN}/auto>`);

    expect(startTags(html).filter((tag) => tag.includes(ORIGIN))).toEqual([]);
    expect(html).toContain('<button');
  });

  it('shows raw HTML as escaped text', () => {
    const html = renderMarkdown('<script>window.compromised = true;</script>');

    expect(html).toContain('&lt;script>window.compromised = true;&lt;/script>');
  });
});
