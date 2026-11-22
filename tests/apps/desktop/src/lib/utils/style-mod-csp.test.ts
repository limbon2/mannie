// Tests the CSP-safe constructible stylesheet replacement for `style-mod`
// (ADR-0004, ADR-0076): rules render cleanly and mount onto adoptedStyleSheets
// without injecting `<style>` tags.
import { describe, expect, it, vi } from 'vitest';

import { StyleModule } from '../../../../../../apps/desktop/src/lib/utils/style-mod-csp.js';

class FakeCSSStyleSheet {
  text = '';

  replaceSync(css: string): void {
    this.text = css;
  }
}

interface FakeDocument {
  adoptedStyleSheets: FakeCSSStyleSheet[];
  createElement: ReturnType<typeof vi.fn>;
  defaultView: { CSSStyleSheet: typeof FakeCSSStyleSheet };
  ownerDocument: null;
}

function fakeDocument(): FakeDocument {
  return {
    adoptedStyleSheets: [],
    createElement: vi.fn(),
    defaultView: { CSSStyleSheet: FakeCSSStyleSheet },
    ownerDocument: null,
  };
}

describe('StyleModule (CSP-safe constructible stylesheets)', () => {
  it('renders nested selectors, camelCase properties, custom properties and at-rules', () => {
    const mod = new StyleModule({
      '.cm-editor': {
        '--accent': 'blue',
        backgroundColor: 'transparent',
        '&.cm-focused': { outlineStyle: 'none' },
      },
      '@media (prefers-reduced-motion: reduce)': {
        '.cm-cursor': { animationDuration: '0s' },
      },
    });

    const rules = mod.getRules();
    expect(rules).toContain('.cm-editor {--accent: blue; background-color: transparent;}');
    expect(rules).toContain('.cm-editor.cm-focused {outline-style: none;}');
    expect(rules).toContain('@media (prefers-reduced-motion: reduce) {.cm-cursor {animation-duration: 0s;}}');
  });

  it('mounts modules onto adoptedStyleSheets without creating <style> tags and preserves ordering', () => {
    const doc = fakeDocument();
    const base = new StyleModule({ '.a': { color: 'red' } });
    const override = new StyleModule({ '.a': { color: 'green' } });

    StyleModule.mount(doc as unknown as Document, [base, override]);

    expect(doc.createElement).not.toHaveBeenCalled();
    expect(doc.adoptedStyleSheets).toHaveLength(1);
    const sheet = doc.adoptedStyleSheets[0];
    expect(sheet).toBeDefined();
    expect(sheet.text).toBe('.a {color: red;}\n.a {color: green;}');

    // Re-mounting in reversed order updates sheet precedence without adding duplicate sheets.
    StyleModule.mount(doc as unknown as Document, [override, base]);
    expect(doc.adoptedStyleSheets).toHaveLength(1);
    expect(sheet.text).toBe('.a {color: green;}\n.a {color: red;}');
  });

  it('generates distinct unique class names', () => {
    const first = StyleModule.newName();
    const second = StyleModule.newName();
    expect(first).not.toBe(second);
  });
});
