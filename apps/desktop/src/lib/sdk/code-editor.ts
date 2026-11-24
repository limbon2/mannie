// The code editor of the TypeScript plugin SDK, `@mannie/plugin-sdk/code-editor`
// (ADR-0076): CodeMirror 6 with JSON, YAML, XML and HTML syntax highlighting,
// dynamic light/dark theme tokens, optional `{{variable}}` completion, live
// URL/variable token highlighting and compact single-line mode.
//
// CodeMirror loads lazily in its own chunk the first time a code editor mounts,
// so the desktop app's startup and first bundle stay unchanged (checked by
// `scripts/check-first-bundle.mjs`). Its styles mount through Mannie's
// constructible-stylesheet `style-mod` adapter, so no `<style>` tags are ever
// created under `style-src 'self'` (ADR-0004).
import type { Completion, CompletionContext, CompletionResult } from '@codemirror/autocomplete';
import type { Extension } from '@codemirror/state';
import type { DecorationSet, EditorView, ViewUpdate } from '@codemirror/view';

/** Syntax highlighting language supported by the SDK code editor. */
export type CodeEditorLanguage = 'html' | 'json' | 'plain' | 'xml' | 'yaml';

/** Extra inline token highlighting overlaid on editor text. */
export type CodeEditorHighlightMode = 'none' | 'url' | 'variables';

/** One completion item surfaced inside `{{...}}` template expressions. */
export interface CodeCompletionItem {
  /** Text inserted in place of the typed token. Defaults to `{{label}}`. */
  readonly apply?: string;
  /** Short secondary metadata shown beside the label (for example, scope). */
  readonly detail?: string;
  /** Longer description or value preview shown in the completion info box. */
  readonly info?: string;
  /** Variable or symbol name matched against what the owner types. */
  readonly label: string;
}

/** Configuration passed when mounting a CodeMirror 6 view. */
export interface CodeEditorOptions {
  /** Accessible name applied to the editor's editable content area. */
  readonly ariaLabel: string;
  /** Optional provider for `{{variable}}` completions. */
  readonly completions?: () => readonly CodeCompletionItem[];
  /** Extra token decoration mode for `{{variable}}` and URL templates. */
  readonly highlightTokens?: CodeEditorHighlightMode;
  /** Optional DOM `id` applied to the editable content area. */
  readonly id?: string;
  /** Initial syntax language. */
  readonly language: CodeEditorLanguage;
  /** Whether to show line numbers in the gutter. */
  readonly lineNumbers: boolean;
  /** Called whenever the owner edits the document. */
  readonly onChange?: (value: string) => void;
  /** Called when the owner presses Enter in `singleLine` mode. */
  readonly onEnter?: () => void;
  /** Called when the owner presses Ctrl+Enter / Cmd+Enter inside the editor. */
  readonly onModEnter?: () => void;
  /** Placeholder shown when the document is empty. */
  readonly placeholder: string;
  /** Whether the editor is read-only. */
  readonly readonly: boolean;
  /** Restricts the editor to one horizontally scrolling line (for URL bars). */
  readonly singleLine?: boolean;
  /** Initial document text. */
  readonly value: string;
  /** Whether long lines wrap to the viewport width. */
  readonly wrapLines: boolean;
}

/** Live handle returned by `createCodeEditor`. */
export interface CodeEditorHandle {
  /** Destroys the CodeMirror instance and disconnects observers. */
  readonly dispose: () => void;
  /** Moves keyboard focus into the editor. */
  readonly focus: () => void;
  /** Switches the active syntax grammar in place. */
  readonly setLanguage: (language: CodeEditorLanguage) => Promise<void>;
  /** Updates the placeholder text shown when empty. */
  readonly setPlaceholder: (placeholder: string) => void;
  /** Toggles read-only mode in place. */
  readonly setReadonly: (readonly: boolean) => void;
  /** Replaces the document text when it differs from the current buffer. */
  readonly setValue: (value: string) => void;
}

interface TokenSpan {
  readonly cls: string;
  readonly from: number;
  readonly to: number;
}

const DOUBLE_BRACE_VAR = /\{\{[^{}\s]+\}\}/gu;
const URL_BASE_TOKEN = /\{\{[^{}\s]+\}\}|(?<!\{)\{[^{}/\s?&#]+\}(?!\})/gu;

function pushVariableSpans(
  text: string,
  offset: number,
  end: number,
  fallbackCls: string | null,
  out: TokenSpan[],
): void {
  const slice = text.slice(offset, end);
  let cursor = 0;

  for (const match of slice.matchAll(DOUBLE_BRACE_VAR)) {
    const start = match.index;
    const matchEnd = start + match[0].length;
    const hasFallback = fallbackCls !== null;
    const hasGap = start > cursor;

    if (hasFallback && hasGap) {
      out.push({ cls: fallbackCls, from: offset + cursor, to: offset + start });
    }
    out.push({ cls: 'cm-tok-var', from: offset + start, to: offset + matchEnd });
    cursor = matchEnd;
  }

  const hasFallbackTail = fallbackCls !== null;
  const hasRemaining = cursor < slice.length;
  if (hasFallbackTail && hasRemaining) {
    out.push({ cls: fallbackCls, from: offset + cursor, to: end });
  }
}

function collectUrlTokenSpans(text: string): TokenSpan[] {
  const spans: TokenSpan[] = [];
  const hashIndex = text.indexOf('#');
  const beforeHashEnd = hashIndex === -1 ? text.length : hashIndex;
  const queryIndex = text.indexOf('?');
  const hasQuestionMark = queryIndex !== -1;
  const isQueryBeforeHash = queryIndex < beforeHashEnd;
  const hasQuery = hasQuestionMark && isQueryBeforeHash;
  const baseEnd = hasQuery ? queryIndex : beforeHashEnd;

  const baseSlice = text.slice(0, baseEnd);
  for (const match of baseSlice.matchAll(URL_BASE_TOKEN)) {
    const start = match.index;
    const matchEnd = start + match[0].length;
    const cls = match[0].startsWith('{{') ? 'cm-tok-var' : 'cm-tok-path';
    spans.push({ cls, from: start, to: matchEnd });
  }

  if (hasQuery) {
    spans.push({ cls: 'cm-tok-query-punct', from: queryIndex, to: queryIndex + 1 });
    let cursor = queryIndex + 1;

    while (cursor < beforeHashEnd) {
      const nextAmp = text.indexOf('&', cursor);
      const hasAmp = nextAmp !== -1;
      const isAmpInsideQuery = nextAmp <= beforeHashEnd;
      const partEnd = hasAmp && isAmpInsideQuery ? nextAmp : beforeHashEnd;

      if (partEnd > cursor) {
        const eqIndex = text.indexOf('=', cursor);
        const hasEquals = eqIndex !== -1;
        const isEqualsInsidePart = eqIndex < partEnd;
        const hasEq = hasEquals && isEqualsInsidePart;

        if (hasEq) {
          if (eqIndex > cursor) {
            pushVariableSpans(text, cursor, eqIndex, 'cm-tok-query-key', spans);
          }
          spans.push({ cls: 'cm-tok-query-punct', from: eqIndex, to: eqIndex + 1 });
          if (partEnd > eqIndex + 1) {
            pushVariableSpans(text, eqIndex + 1, partEnd, 'cm-tok-query-val', spans);
          }
        } else {
          pushVariableSpans(text, cursor, partEnd, 'cm-tok-query-key', spans);
        }
      }

      if (partEnd < beforeHashEnd) {
        spans.push({ cls: 'cm-tok-query-punct', from: partEnd, to: partEnd + 1 });
        cursor = partEnd + 1;
      } else {
        cursor = beforeHashEnd;
      }
    }
  }

  if (hashIndex !== -1) {
    pushVariableSpans(text, hashIndex, text.length, null, spans);
  }

  return spans;
}

function collectTokenSpans(text: string, mode: CodeEditorHighlightMode): TokenSpan[] {
  if (mode === 'none') {
    return [];
  }
  if (mode === 'url') {
    return collectUrlTokenSpans(text);
  }
  const spans: TokenSpan[] = [];
  pushVariableSpans(text, 0, text.length, null, spans);
  return spans;
}

function isDarkTheme(): boolean {
  const hasDocument = typeof document !== 'undefined';

  return hasDocument && document.documentElement.classList.contains('dark');
}

async function loadLanguageExtension(language: CodeEditorLanguage): Promise<Extension> {
  switch (language) {
    case 'json': {
      const { json } = await import('@codemirror/lang-json');
      return json();
    }
    case 'yaml': {
      const { yaml } = await import('@codemirror/lang-yaml');
      return yaml();
    }
    case 'xml': {
      const { xml } = await import('@codemirror/lang-xml');
      return xml();
    }
    case 'html': {
      const { html } = await import('@codemirror/lang-html');
      return html();
    }
    case 'plain': {
      return [];
    }
  }
}

function buildVariableCompletionSource(
  provider: () => readonly CodeCompletionItem[],
): (context: CompletionContext) => CompletionResult | null {
  return (context: CompletionContext): CompletionResult | null => {
    const match = context.matchBefore(/\{\{[\w.-]*/u);

    if (match === null) {
      return null;
    }

    const isEmptyMatch = match.from === match.to;

    if (isEmptyMatch && !context.explicit) {
      return null;
    }

    const items = provider();

    if (items.length === 0) {
      return null;
    }

    const afterCursor = context.state.sliceDoc(context.pos, context.pos + 2);
    const replaceTo = afterCursor === '}}' ? context.pos + 2 : context.pos;
    const options: Completion[] = items.map((item) => ({
      apply: item.apply ?? `{{${item.label}}}`,
      detail: item.detail,
      info: item.info,
      label: item.label,
      type: 'variable',
    }));

    return {
      from: match.from,
      options,
      to: replaceTo,
      validFor: /^\{\{[\w.-]*$/u,
    };
  };
}

/**
 * Mounts a lazily loaded CodeMirror 6 editor inside `element`.
 * All CodeMirror packages stay in a separate async chunk outside the first bundle.
 */
export async function createCodeEditor(element: HTMLElement, options: CodeEditorOptions): Promise<CodeEditorHandle> {
  const [
    { Compartment, EditorState, RangeSetBuilder },
    {
      Decoration,
      EditorView: EditorViewClass,
      ViewPlugin,
      drawSelection,
      highlightActiveLine,
      keymap,
      lineNumbers,
      placeholder,
    },
    { HighlightStyle, bracketMatching, defaultHighlightStyle, foldGutter, indentOnInput, syntaxHighlighting },
    { defaultKeymap, history, historyKeymap },
    { acceptCompletion, autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap },
    { tags },
  ] = await Promise.all([
    import('@codemirror/state'),
    import('@codemirror/view'),
    import('@codemirror/language'),
    import('@codemirror/commands'),
    import('@codemirror/autocomplete'),
    import('@lezer/highlight'),
  ]);

  const singleLine = options.singleLine === true;
  const highlightMode = options.highlightTokens ?? 'none';

  const buildDecorations = (view: EditorView): DecorationSet => {
    const builder = new RangeSetBuilder<ReturnType<typeof Decoration.mark>>();
    const docText = view.state.doc.toString();

    if (highlightMode === 'url') {
      for (const span of collectTokenSpans(docText, 'url')) {
        builder.add(span.from, span.to, Decoration.mark({ class: span.cls }));
      }
      return builder.finish();
    }

    for (const { from, to } of view.visibleRanges) {
      const slice = view.state.doc.sliceString(from, to);
      for (const span of collectTokenSpans(slice, highlightMode)) {
        builder.add(from + span.from, from + span.to, Decoration.mark({ class: span.cls }));
      }
    }

    return builder.finish();
  };

  const tokenHighlightPlugin =
    highlightMode === 'none'
      ? []
      : ViewPlugin.fromClass(
          class {
            decorations: DecorationSet;

            constructor(view: EditorView) {
              this.decorations = buildDecorations(view);
            }

            update(update: ViewUpdate): void {
              const needsRebuild = update.docChanged || update.viewportChanged;
              if (needsRebuild) {
                this.decorations = buildDecorations(update.view);
              }
            }
          },
          { decorations: (plugin) => plugin.decorations },
        );

  const buildTheme = (dark: boolean): Extension => {
    // High-contrast syntax colors meeting WCAG 2.1 AA against Mannie's light and dark surfaces.
    const highlight = HighlightStyle.define(
      dark
        ? [
            { color: '#93c5fd', tag: [tags.propertyName, tags.attributeName] },
            { color: '#86efac', tag: [tags.string, tags.special(tags.string)] },
            { color: '#fcd34d', tag: [tags.number, tags.integer, tags.float] },
            { color: '#f0abfc', tag: [tags.bool, tags.null, tags.atom, tags.keyword] },
            { color: '#67e8f9', tag: [tags.tagName, tags.typeName] },
            { color: '#a1a1aa', tag: [tags.comment, tags.meta] },
            { color: '#d4d4d8', tag: [tags.punctuation, tags.brace, tags.squareBracket] },
          ]
        : [
            { color: '#1d4ed8', tag: [tags.propertyName, tags.attributeName] },
            { color: '#15803d', tag: [tags.string, tags.special(tags.string)] },
            { color: '#b45309', tag: [tags.number, tags.integer, tags.float] },
            { color: '#7e22ce', tag: [tags.bool, tags.null, tags.atom, tags.keyword] },
            { color: '#0e7490', tag: [tags.tagName, tags.typeName] },
            { color: '#52525b', tag: [tags.comment, tags.meta] },
            { color: '#3f3f46', tag: [tags.punctuation, tags.brace, tags.squareBracket] },
          ],
    );

    const tokenStyles = {
      '.cm-tok-path': {
        backgroundColor: 'var(--token-path)',
        borderRadius: '3px',
        boxShadow: 'inset 0 0 0 1px color-mix(in oklab, var(--token-path-border) 55%, transparent)',
        color: 'var(--token-path-foreground)',
        fontWeight: '500',
      },
      '.cm-tok-query-key': {
        backgroundColor: 'var(--token-query)',
        borderRadius: '3px',
        boxShadow: 'inset 0 0 0 1px color-mix(in oklab, var(--token-query-border) 55%, transparent)',
        color: 'var(--token-query-foreground)',
        fontWeight: '500',
      },
      '.cm-tok-query-punct': {
        color: 'var(--token-query-foreground)',
        fontWeight: '600',
      },
      '.cm-tok-query-val': {
        color: 'var(--token-query-foreground)',
      },
      '.cm-tok-var': {
        backgroundColor: 'var(--token-variable)',
        borderRadius: '3px',
        boxShadow: 'inset 0 0 0 1px color-mix(in oklab, var(--token-variable-border) 55%, transparent)',
        color: 'var(--token-variable-foreground)',
        fontWeight: '500',
      },
    };

    const baseTheme = EditorViewClass.theme(
      {
        '&': {
          backgroundColor: 'var(--background)',
          color: 'var(--foreground)',
          fontFamily: 'var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace)',
          fontSize: '0.75rem',
          height: '100%',
        },
        '&.cm-focused': {
          outline: 'none',
        },
        '.cm-activeLine': {
          backgroundColor: 'color-mix(in oklab, var(--accent) 55%, transparent)',
        },
        '.cm-activeLineGutter': {
          backgroundColor: 'color-mix(in oklab, var(--accent) 65%, transparent)',
          color: 'var(--foreground)',
        },
        '.cm-completionDetail': {
          color: 'var(--muted-foreground)',
          fontStyle: 'normal',
          marginLeft: '0.5rem',
        },
        '.cm-content': {
          caretColor: 'var(--foreground)',
          padding: singleLine ? '0.4375rem 0' : '0.5rem 0',
        },
        '.cm-cursor, .cm-dropCursor': {
          borderLeftColor: 'var(--foreground)',
        },
        '.cm-gutters': {
          backgroundColor: 'var(--muted)',
          borderRight: '1px solid var(--border)',
          color: 'var(--muted-foreground)',
          userSelect: 'none',
        },
        '.cm-line': {
          padding: singleLine ? '0 0.75rem' : '0 0.625rem',
        },
        '.cm-placeholder': {
          color: 'var(--muted-foreground)',
        },
        '.cm-scroller': {
          ...(singleLine ? { alignItems: 'center', scrollbarWidth: 'none' } : {}),
          fontFamily: 'inherit',
          lineHeight: '1.5',
          overflow: 'auto',
          overflowY: singleLine ? 'hidden' : 'auto',
        },
        '.cm-selectionBackground, &.cm-focused .cm-selectionBackground, ::selection': {
          backgroundColor: 'color-mix(in oklab, var(--primary) 22%, transparent)',
        },
        '.cm-tooltip': {
          backgroundColor: 'var(--popover)',
          border: '1px solid var(--border)',
          borderRadius: 'var(--radius-md)',
          boxShadow: '0 4px 12px rgb(0 0 0 / 0.14)',
          color: 'var(--popover-foreground)',
          zIndex: '60',
        },
        '.cm-tooltip.cm-tooltip-autocomplete > ul': {
          fontFamily: 'var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace)',
          fontSize: '0.75rem',
          maxHeight: '14rem',
        },
        '.cm-tooltip.cm-tooltip-autocomplete > ul > li': {
          padding: '0.25rem 0.5rem',
        },
        '.cm-tooltip.cm-tooltip-autocomplete > ul > li[aria-selected]': {
          backgroundColor: 'color-mix(in oklab, var(--primary) 12%, var(--popover))',
          color: 'var(--primary)',
        },
        ...tokenStyles,
      },
      { dark },
    );

    return [baseTheme, syntaxHighlighting(highlight), syntaxHighlighting(defaultHighlightStyle, { fallback: true })];
  };

  const languageCompartment = new Compartment();
  const readonlyCompartment = new Compartment();
  const placeholderCompartment = new Compartment();
  const themeCompartment = new Compartment();

  const initialLanguage = await loadLanguageExtension(options.language);
  let syncingExternal = false;
  let disposed = false;

  const customKeys = [
    ...(options.onModEnter === undefined
      ? []
      : [
          {
            key: 'Mod-Enter',
            run: (): boolean => {
              options.onModEnter?.();
              return true;
            },
          },
        ]),
    ...(singleLine
      ? [
          {
            key: 'Enter',
            run: (target: EditorView): boolean => {
              if (acceptCompletion(target)) {
                return true;
              }
              options.onEnter?.();
              return true;
            },
          },
        ]
      : []),
    ...(options.completions === undefined
      ? []
      : [
          {
            key: 'Tab',
            run: acceptCompletion,
          },
        ]),
  ];

  const singleLineFilter = singleLine
    ? EditorState.transactionFilter.of((tr) => {
        if (!tr.docChanged) {
          return tr;
        }
        const nextDoc = tr.newDoc.toString();
        const hasLf = nextDoc.includes('\n');
        const hasCr = nextDoc.includes('\r');
        const hasNewline = hasLf || hasCr;
        if (!hasNewline) {
          return tr;
        }
        const cleaned = nextDoc.replaceAll(/[\r\n]+/gu, '');
        return {
          changes: { from: 0, insert: cleaned, to: tr.startState.doc.length },
          selection: { anchor: Math.min(cleaned.length, tr.newSelection.main.head) },
        };
      })
    : [];

  const contentAttrs: Record<string, string> = {
    'aria-label': options.ariaLabel,
    'data-content': '',
    role: 'textbox',
  };
  if (!singleLine) {
    contentAttrs['aria-multiline'] = 'true';
  }
  if (options.id !== undefined) {
    contentAttrs.id = options.id;
  }

  const activeLineExt = !options.readonly && !singleLine ? [highlightActiveLine()] : [];

  const extensions: Extension[] = [
    history(),
    drawSelection(),
    indentOnInput(),
    bracketMatching(),
    closeBrackets(),
    singleLineFilter,
    tokenHighlightPlugin,
    keymap.of([...customKeys, ...closeBracketsKeymap, ...completionKeymap, ...defaultKeymap, ...historyKeymap]),
    EditorViewClass.contentAttributes.of(contentAttrs),
    languageCompartment.of(initialLanguage),
    readonlyCompartment.of([
      EditorState.readOnly.of(options.readonly),
      EditorViewClass.editable.of(!options.readonly),
      ...(options.readonly
        ? [EditorViewClass.contentAttributes.of({ 'aria-readonly': 'true', tabindex: '0' })]
        : activeLineExt),
    ]),
    placeholderCompartment.of(options.placeholder === '' ? [] : placeholder(options.placeholder)),
    themeCompartment.of(buildTheme(isDarkTheme())),
    EditorViewClass.updateListener.of((update) => {
      if (update.docChanged && !syncingExternal) {
        options.onChange?.(update.state.doc.toString());
      }
    }),
  ];

  if (options.lineNumbers && !singleLine) {
    extensions.push(lineNumbers(), foldGutter());
  }

  if (options.wrapLines && !singleLine) {
    extensions.push(EditorViewClass.lineWrapping);
  }

  if (options.completions !== undefined) {
    extensions.push(
      autocompletion({
        override: [buildVariableCompletionSource(options.completions)],
      }),
    );
  }

  const state = EditorState.create({
    doc: singleLine ? options.value.replaceAll(/[\r\n]+/gu, '') : options.value,
    extensions,
  });
  const view = new EditorViewClass({
    parent: element,
    state,
  });

  const themeObserver = new MutationObserver(() => {
    if (disposed) {
      return;
    }
    view.dispatch({
      effects: themeCompartment.reconfigure(buildTheme(isDarkTheme())),
    });
  });
  themeObserver.observe(document.documentElement, { attributeFilter: ['class'], attributes: true });

  let languageRequest = 0;

  return {
    dispose: () => {
      disposed = true;
      themeObserver.disconnect();
      view.destroy();
    },
    focus: () => {
      view.focus();
    },
    setLanguage: async (nextLanguage: CodeEditorLanguage): Promise<void> => {
      languageRequest += 1;
      const ticket = languageRequest;
      const loaded = await loadLanguageExtension(nextLanguage);
      const isStale = ticket !== languageRequest;

      if (disposed || isStale) {
        return;
      }

      view.dispatch({
        effects: languageCompartment.reconfigure(loaded),
      });
    },
    setPlaceholder: (nextPlaceholder: string) => {
      if (disposed) {
        return;
      }
      view.dispatch({
        effects: placeholderCompartment.reconfigure(nextPlaceholder === '' ? [] : placeholder(nextPlaceholder)),
      });
    },
    setReadonly: (nextReadonly: boolean) => {
      if (disposed) {
        return;
      }
      const nextActiveLine = !nextReadonly && !singleLine ? [highlightActiveLine()] : [];
      view.dispatch({
        effects: readonlyCompartment.reconfigure([
          EditorState.readOnly.of(nextReadonly),
          EditorViewClass.editable.of(!nextReadonly),
          ...(nextReadonly
            ? [EditorViewClass.contentAttributes.of({ 'aria-readonly': 'true', tabindex: '0' })]
            : nextActiveLine),
        ]),
      });
    },
    setValue: (nextValue: string) => {
      if (disposed) {
        return;
      }
      const normalized = singleLine ? nextValue.replaceAll(/[\r\n]+/gu, '') : nextValue;
      const current = view.state.doc.toString();
      if (current === normalized) {
        return;
      }
      syncingExternal = true;
      try {
        view.dispatch({
          changes: { from: 0, insert: normalized, to: current.length },
        });
      } finally {
        syncingExternal = false;
      }
    },
  };
}
