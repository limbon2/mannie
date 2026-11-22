// Constructible-stylesheet replacement for `style-mod` (ADR-0004, ADR-0076).
//
// CodeMirror 6 mounts its theme and layout CSS through `style-mod`. Upstream
// `style-mod` only uses `adoptedStyleSheets` inside shadow roots and falls back
// to injecting `<style>` tags into `document.head`, which Mannie's strict
// Content Security Policy (`style-src 'self'`) blocks. This module keeps the
// exact `StyleModule` API and mounts rules onto a constructible `CSSStyleSheet`
// in `adoptedStyleSheets` for both documents and shadow roots, matching the
// approach used for xterm.js (ADR-0053, decision 7).

/** Nested CSS specification accepted by `StyleModule`. */
export interface StyleSpec {
  readonly [propOrSelector: string]: number | string | StyleSpec | null | undefined;
}

/** Options for constructing a `StyleModule`. */
export interface StyleModuleOptions {
  readonly finish?: (selector: string) => string;
}

/** Options accepted by `StyleModule.mount`. */
export interface StyleMountOptions {
  readonly nonce?: string;
}

type StyleRoot = Document | ShadowRoot;

function ownerDocumentOf(root: StyleRoot): Document {
  return root.ownerDocument ?? root;
}

type StyleCounterHost = Record<symbol, number | undefined>;

const PREFIX = '\u037c';
const COUNTER_KEY = Symbol.for(PREFIX);
const ROOT_SETS = new WeakMap<StyleRoot, StyleSet>();
const ADOPTED_SETS = new WeakMap<Document, StyleSet>();

function expandSelectorList(selector: string, parents: readonly string[] = ['&']): string[] {
  if (selector.startsWith('@')) {
    return [selector];
  }

  const result: string[] = [];
  const fragments: string[] = [];
  let depth = 0;
  let quote: string | null = null;
  let lastNonSpace = 0;
  let cut = 0;

  function finishSelector(): void {
    fragments.push(selector.slice(cut, lastNonSpace));
    for (const parent of parents) {
      result.push(fragments.join(parent));
    }
    fragments.length = 0;
  }

  for (let index = 0; index < selector.length; index += 1) {
    const ch = selector[index] ?? '';

    if (ch === '\\') {
      index += 1;
      lastNonSpace = Math.min(index + 1, selector.length);
      continue;
    }

    const isQuote = ch === '"' || ch === "'";
    const isUnquoted = quote === null;
    const isOpener = ch === '[' || ch === '(';
    const isCloser = ch === ']' || ch === ')';
    const isTopLevelComma = isUnquoted && depth === 0 && ch === ',';
    const isParentReference = isUnquoted && ch === '&';
    const isAtSelectorStart = index === cut && fragments.length === 0;

    if (isQuote) {
      if (ch === quote) {
        quote = null;
      } else {
        quote ??= ch;
      }
    } else if (isUnquoted && isOpener) {
      depth += 1;
    } else if (isUnquoted && isCloser) {
      depth -= 1;
    } else if (isTopLevelComma) {
      finishSelector();
      cut = index + 1;
    } else if (isParentReference) {
      fragments.push(selector.slice(cut, index));
      cut = index + 1;
    }

    if (/\S/u.test(ch)) {
      lastNonSpace = index + 1;
    } else if (isAtSelectorStart) {
      cut += 1;
    }
  }

  finishSelector();
  return result;
}

function renderSpec(
  selectors: readonly string[],
  spec: StyleSpec | null | undefined,
  target: string[],
  finish: ((selector: string) => string) | undefined,
  isKeyframes: boolean,
): void {
  const first = selectors[0] ?? '';
  const atMatch = /^@(\w+)\b/u.exec(first);
  const isAt = atMatch !== null;
  const keyframes = isAt && atMatch[1] === 'keyframes';

  const isMissingSpec = spec === null || spec === undefined;

  if (isMissingSpec) {
    if (isAt) {
      target.push(`${first};`);
    }
    return;
  }

  const local: string[] = [];

  for (const [rawProp, value] of Object.entries(spec)) {
    const isDefined = value !== null && value !== undefined;
    const isObjectValue = isDefined && typeof value === 'object';

    if (rawProp.includes('&')) {
      if (isObjectValue) {
        renderSpec(expandSelectorList(rawProp, selectors), value, target, finish, false);
      }
    } else if (isObjectValue) {
      if (!isAt) {
        throw new RangeError(`The value of a property (${rawProp}) should be a primitive value.`);
      }
      renderSpec(expandSelectorList(rawProp), value, local, finish, keyframes);
    } else if (isDefined) {
      const cssProp = rawProp.startsWith('--')
        ? rawProp
        : rawProp.replace(/_.*/u, '').replaceAll(/[A-Z]/gu, (letter) => `-${letter.toLowerCase()}`);
      local.push(`${cssProp}: ${String(value)};`);
    }
  }

  const hasRules = local.length > 0;
  const shouldEmit = hasRules || keyframes;

  if (shouldEmit) {
    const hasFinisher = finish !== undefined;
    const shouldFinish = hasFinisher && !isAt && !isKeyframes;
    const renderedSelectors = shouldFinish ? selectors.map((selector) => finish(selector)) : selectors;
    target.push(`${renderedSelectors.join(', ')} {${local.join(' ')}}`);
  }
}

class StyleSet {
  readonly modules: StyleModule[] = [];
  private readonly sheet: CSSStyleSheet | null;

  constructor(root: StyleRoot) {
    const doc = ownerDocumentOf(root);
    const win = doc.defaultView;
    const canAdopt =
      win !== null &&
      typeof win.CSSStyleSheet === 'function' &&
      'adoptedStyleSheets' in root &&
      typeof root.adoptedStyleSheets.includes === 'function';

    this.sheet = canAdopt ? new win.CSSStyleSheet() : null;
  }

  mount(modules: readonly StyleModule[], root: StyleRoot): void {
    let changed = false;
    let insertAt = 0;

    for (const mod of modules) {
      let existing = this.modules.indexOf(mod);

      const isOutOfOrder = existing > -1 && existing < insertAt;

      if (isOutOfOrder) {
        this.modules.splice(existing, 1);
        changed = true;
        insertAt -= 1;
        existing = -1;
      }

      if (existing === -1) {
        this.modules.splice(insertAt, 0, mod);
        insertAt += 1;
        changed = true;
      } else {
        insertAt = existing + 1;
      }
    }

    if (this.sheet === null) {
      return;
    }

    if (changed) {
      const css = this.modules.map((mod) => mod.getRules()).join('\n');
      this.sheet.replaceSync(css);
    }

    if (!root.adoptedStyleSheets.includes(this.sheet)) {
      root.adoptedStyleSheets = [this.sheet, ...root.adoptedStyleSheets];
    }
  }
}

/** Encapsulates a set of CSS rules mounted into a document or shadow root. */
export class StyleModule {
  readonly rules: string[] = [];

  constructor(spec: StyleSpec, options?: StyleModuleOptions) {
    const finish = options?.finish;

    for (const [selector, nested] of Object.entries(spec)) {
      const isRuleBlock = nested === null || nested === undefined || typeof nested === 'object';

      if (isRuleBlock) {
        renderSpec(expandSelectorList(selector), nested, this.rules, finish, false);
      }
    }
  }

  /** Returns the module's CSS rules joined by newlines. */
  getRules(): string {
    return this.rules.join('\n');
  }

  /** Generates a unique CSS class name. */
  static newName(): string {
    const host = globalThis as unknown as StyleCounterHost;
    const next = host[COUNTER_KEY] ?? 1;
    host[COUNTER_KEY] = next + 1;
    return `${PREFIX}${next.toString(36)}`;
  }

  /** Mounts one or more style modules into `root` using constructible stylesheets. */
  static mount(root: StyleRoot, modules: readonly StyleModule[] | StyleModule, _options?: StyleMountOptions): void {
    let set = ROOT_SETS.get(root);

    if (set === undefined) {
      const doc = ownerDocumentOf(root);
      const isDocument = root.ownerDocument === null;
      const existingAdopted = isDocument ? undefined : ADOPTED_SETS.get(doc);

      if (existingAdopted !== undefined) {
        set = existingAdopted;
      } else {
        set = new StyleSet(root);
        if (!isDocument) {
          ADOPTED_SETS.set(doc, set);
        }
      }

      ROOT_SETS.set(root, set);
    }

    const list = Array.isArray(modules) ? modules : [modules];
    set.mount(list, root);
  }
}
