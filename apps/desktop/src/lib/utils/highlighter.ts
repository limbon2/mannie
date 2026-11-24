// Lazy-loaded Shiki syntax highlighter mapped to semantic CSS classes
// (ADR-0079). Emits zero inline `style` attributes so every highlighted block
// stays compliant with the webview's `style-src: 'self'` Content Security
// Policy (ADR-0004) and the Markdown renderer checks (ADR-0021).
import type { GrammarState, HighlighterCore, LanguageRegistration, ThemeRegistration } from 'shiki/core';

/** Maximum total characters highlighted in a single block (ADR-0079). */
export const MAX_HIGHLIGHT_CHARS = 32_768;

/** Maximum length of a single line before falling back to plain text. */
export const MAX_HIGHLIGHT_LINE_CHARS = 2_000;

/** A single highlighted token compatible with `CodeBlock.Line` (CSS classes only, never `style`). */
export interface HighlightedToken {
  readonly class?: string;
  readonly content: string;
  readonly style?: string;
}

/** A highlighted line: plain text or CSS-class-only token spans. */
export type HighlightedLine = HighlightedToken[] | string;

/** Result of highlighting one chunk of lines with its trailing grammar state. */
export interface HighlightResult {
  readonly lines: HighlightedLine[];
  readonly state: GrammarState | undefined;
}

/** A loaded language tokenizer backed by the shared Shiki core instance. */
export interface LanguageHighlighter {
  readonly highlight: (code: string, state?: GrammarState) => HighlightResult;
}

interface LanguageModule {
  default: LanguageRegistration[];
}
type LanguageLoader = () => Promise<LanguageModule>;

// Sentinel hex colors in the in-memory TextMate theme; never rendered into the
// DOM, only mapped back to `.syntax-*` classes after TextMate scope resolution.
const SENTINEL = {
  comment: '#010101',
  deleted: '#090909',
  function: '#040404',
  inserted: '#0a0a0a',
  keyword: '#020202',
  number: '#060606',
  operator: '#080808',
  string: '#030303',
  type: '#070707',
  variable: '#050505',
} as const;

const TOKEN_CLASSES: Readonly<Record<string, string>> = {
  [SENTINEL.comment]: 'syntax-comment',
  [SENTINEL.deleted]: 'syntax-deleted',
  [SENTINEL.function]: 'syntax-function',
  [SENTINEL.inserted]: 'syntax-inserted',
  [SENTINEL.keyword]: 'syntax-keyword',
  [SENTINEL.number]: 'syntax-number',
  [SENTINEL.operator]: 'syntax-operator',
  [SENTINEL.string]: 'syntax-string',
  [SENTINEL.type]: 'syntax-type',
  [SENTINEL.variable]: 'syntax-variable',
};

const MANNIE_THEME: ThemeRegistration = {
  colors: {},
  name: 'mannie',
  tokenColors: [
    {
      scope: ['comment', 'punctuation.definition.comment', 'string.comment'],
      settings: { foreground: SENTINEL.comment },
    },
    {
      scope: [
        'string',
        'string.quoted',
        'string.template',
        'string.regexp',
        'meta.embedded.assembly',
        'punctuation.definition.string',
      ],
      settings: { foreground: SENTINEL.string },
    },
    {
      scope: [
        'keyword',
        'storage',
        'storage.type',
        'storage.modifier',
        'constant.language.boolean',
        'constant.language.null',
      ],
      settings: { foreground: SENTINEL.keyword },
    },
    {
      scope: [
        'entity.name.function',
        'support.function',
        'meta.function-call',
        'support.function.builtin',
        'entity.name.command',
        'entity.name.tag',
        'support.type.property-name.json',
      ],
      settings: { foreground: SENTINEL.function },
    },
    {
      scope: [
        'variable',
        'variable.other',
        'variable.parameter',
        'variable.language',
        'punctuation.definition.variable',
        'entity.other.attribute-name',
        'support.variable',
      ],
      settings: { foreground: SENTINEL.variable },
    },
    {
      scope: ['constant.numeric', 'constant.language', 'constant.character', 'constant.other'],
      settings: { foreground: SENTINEL.number },
    },
    {
      scope: [
        'entity.name.type',
        'entity.name.class',
        'entity.name.struct',
        'entity.name.enum',
        'entity.name.interface',
        'support.type',
        'support.class',
      ],
      settings: { foreground: SENTINEL.type },
    },
    {
      scope: [
        'keyword.operator',
        'punctuation.separator',
        'punctuation.terminator',
        'punctuation.accessor',
        'variable.parameter.option',
      ],
      settings: { foreground: SENTINEL.operator },
    },
    {
      scope: ['markup.deleted', 'meta.diff.header.from-file'],
      settings: { foreground: SENTINEL.deleted },
    },
    {
      scope: ['markup.inserted', 'meta.diff.header.to-file'],
      settings: { foreground: SENTINEL.inserted },
    },
  ],
  type: 'dark',
};

// Every grammar loads lazily on its first use so none enters the first bundle.
const LANGUAGES: Readonly<Partial<Record<string, LanguageLoader>>> = {
  bash: () => import('shiki/langs/shellscript.mjs'),
  bat: () => import('shiki/langs/bat.mjs'),
  c: () => import('shiki/langs/c.mjs'),
  cpp: () => import('shiki/langs/cpp.mjs'),
  csharp: () => import('shiki/langs/csharp.mjs'),
  css: () => import('shiki/langs/css.mjs'),
  diff: () => import('shiki/langs/diff.mjs'),
  docker: () => import('shiki/langs/docker.mjs'),
  fish: () => import('shiki/langs/fish.mjs'),
  go: () => import('shiki/langs/go.mjs'),
  graphql: () => import('shiki/langs/graphql.mjs'),
  html: () => import('shiki/langs/html.mjs'),
  ini: () => import('shiki/langs/ini.mjs'),
  java: () => import('shiki/langs/java.mjs'),
  javascript: () => import('shiki/langs/javascript.mjs'),
  json: () => import('shiki/langs/json.mjs'),
  jsonc: () => import('shiki/langs/jsonc.mjs'),
  jsx: () => import('shiki/langs/jsx.mjs'),
  lua: () => import('shiki/langs/lua.mjs'),
  markdown: () => import('shiki/langs/markdown.mjs'),
  nix: () => import('shiki/langs/nix.mjs'),
  php: () => import('shiki/langs/php.mjs'),
  powershell: () => import('shiki/langs/powershell.mjs'),
  python: () => import('shiki/langs/python.mjs'),
  ruby: () => import('shiki/langs/ruby.mjs'),
  rust: () => import('shiki/langs/rust.mjs'),
  sql: () => import('shiki/langs/sql.mjs'),
  svelte: () => import('shiki/langs/svelte.mjs'),
  toml: () => import('shiki/langs/toml.mjs'),
  tsx: () => import('shiki/langs/tsx.mjs'),
  typescript: () => import('shiki/langs/typescript.mjs'),
  xml: () => import('shiki/langs/xml.mjs'),
  yaml: () => import('shiki/langs/yaml.mjs'),
};

const ALIASES: Readonly<Record<string, string>> = {
  batch: 'bat',
  'c++': 'cpp',
  'c#': 'csharp',
  cmd: 'bat',
  console: 'bash',
  cs: 'csharp',
  dockerfile: 'docker',
  dos: 'bat',
  gql: 'graphql',
  js: 'javascript',
  json5: 'jsonc',
  md: 'markdown',
  patch: 'diff',
  ps: 'powershell',
  ps1: 'powershell',
  pwsh: 'powershell',
  py: 'python',
  rb: 'ruby',
  rs: 'rust',
  sh: 'bash',
  shell: 'bash',
  shellscript: 'bash',
  sqlite: 'sql',
  ts: 'typescript',
  yml: 'yaml',
  zsh: 'bash',
};

// Shiki's registered grammar ID when the language module's internal ID differs
// from our canonical key.
const GRAMMAR_IDS: Readonly<Record<string, string>> = {
  bash: 'shellscript',
};

let corePromise: Promise<HighlighterCore> | undefined;
const loadingLanguages = new Map<string, Promise<void>>();

async function getCore(): Promise<HighlighterCore> {
  corePromise ??= createCore();
  return corePromise;
}

async function createCore(): Promise<HighlighterCore> {
  const [{ createHighlighterCore }, { createJavaScriptRegexEngine }] = await Promise.all([
    import('shiki/core'),
    import('shiki/engine/javascript'),
  ]);
  return createHighlighterCore({
    engine: createJavaScriptRegexEngine({ forgiving: true }),
    langs: [],
    themes: [MANNIE_THEME],
  });
}

async function ensureLanguageLoaded(core: HighlighterCore, lang: string, loader: LanguageLoader): Promise<void> {
  const grammarId = GRAMMAR_IDS[lang] ?? lang;
  if (core.getLoadedLanguages().includes(grammarId)) {
    return;
  }

  let pending = loadingLanguages.get(lang);
  if (pending === undefined) {
    pending = loader().then(async (module) => {
      await core.loadLanguage(module.default);
    });
    loadingLanguages.set(lang, pending);
    void pending.finally(() => {
      loadingLanguages.delete(lang);
    });
  }
  await pending;
}

/** Whether `code` is within the size and line-length bounds for syntax highlighting. */
export function isHighlightable(code: string): boolean {
  if (code.length === 0) {
    return false;
  }
  if (code.length > MAX_HIGHLIGHT_CHARS) {
    return false;
  }

  let lineStart = 0;
  for (let index = 0; index < code.length; index += 1) {
    if (code.charCodeAt(index) === 10) {
      if (index - lineStart > MAX_HIGHLIGHT_LINE_CHARS) {
        return false;
      }
      lineStart = index + 1;
    }
  }
  return code.length - lineStart <= MAX_HIGHLIGHT_LINE_CHARS;
}

/** Resolves a language name or alias to a supported canonical key, or `null`. */
export function resolveLanguage(language: string | null | undefined): string | null {
  if (language === null) {
    return null;
  }
  if (language === undefined) {
    return null;
  }

  const normalized = language.trim().toLowerCase();
  if (normalized === '') {
    return null;
  }
  if (Object.hasOwn(LANGUAGES, normalized)) {
    return normalized;
  }
  return Object.hasOwn(ALIASES, normalized) ? (ALIASES[normalized] ?? null) : null;
}

/** Picks the Shiki language for a shell executable path, such as `powershell.exe` or `/bin/bash`. */
export function shellLanguage(shell: string | null | undefined): string {
  if (shell === null) {
    return 'bash';
  }
  if (shell === undefined) {
    return 'bash';
  }

  const fileName =
    shell
      .trim()
      .replace(/[\\/]+$/u, '')
      .split(/[\\/]/u)
      .at(-1) ?? '';
  const base = fileName.replace(/\.exe$/iu, '').toLowerCase();
  const isPowerShell = base === 'powershell' || base === 'pwsh' || base.includes('powershell');
  if (isPowerShell) {
    return 'powershell';
  }
  if (base === 'cmd') {
    return 'bat';
  }
  if (base === 'fish') {
    return 'fish';
  }
  return 'bash';
}

/** Loads a grammar on demand and returns a CSS-class-only tokenizer, or `null` for unsupported languages. */
export async function loadHighlighter(language: string | null | undefined): Promise<LanguageHighlighter | null> {
  const lang = resolveLanguage(language);
  if (lang === null) {
    return null;
  }

  const loader = LANGUAGES[lang];
  if (loader === undefined) {
    return null;
  }

  const core = await getCore();
  await ensureLanguageLoaded(core, lang, loader);
  const grammarId = GRAMMAR_IDS[lang] ?? lang;

  return {
    highlight(code: string, state?: GrammarState): HighlightResult {
      const tokenized = core.codeToTokens(code, {
        grammarState: state,
        lang: grammarId,
        theme: 'mannie',
      });

      const lines: HighlightedLine[] = tokenized.tokens.map((line) =>
        line.map((token) => {
          const tokenClass = token.color === undefined ? undefined : TOKEN_CLASSES[token.color.toLowerCase()];
          return tokenClass === undefined ? { content: token.content } : { class: tokenClass, content: token.content };
        }),
      );

      return {
        lines,
        state: core.getLastGrammarState(tokenized.tokens),
      };
    },
  };
}

/** Highlights `code` in `language` once, or returns `null` when unsupported or above size bounds. */
export async function highlightCode(
  code: string,
  language: string | null | undefined,
): Promise<HighlightedLine[] | null> {
  if (!isHighlightable(code)) {
    return null;
  }

  const highlighter = await loadHighlighter(language);
  if (highlighter === null) {
    return null;
  }

  const trimmed = code.endsWith('\n') ? code.slice(0, -1) : code;
  return highlighter.highlight(trimmed).lines;
}
