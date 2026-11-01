// The terminal of the TypeScript plugin SDK, `@mannie/plugin-sdk/terminal`
// (ADR-0053, decision 7): xterm.js 6 with its fit, search, web links and
// WebGL addons, which a plugin's UI cannot add itself (ADR-0035, decision 7).
//
// xterm.js loads as a chunk of its own when the first terminal opens, so the
// app's start and first bundle do not change; this module imports only its
// types up front. The renderer is automatic: WebGL where the webview reports
// a hardware WebGL2 context, the DOM renderer otherwise, as under SwiftShader
// or llvmpipe, and again after a lost context. Links open through
// `openLink`, which asks the owner first, and no clipboard addon loads, so
// output never writes to the clipboard. The theme comes from Mannie's tokens.
import type { ITerminalOptions, ITheme, Terminal } from '@xterm/xterm';

import { openLink } from './links.js';

/** How a terminal draws. */
export type TerminalRenderer = 'automatic' | 'compatible' | 'gpu';

/** What a terminal's view starts with. */
export interface TerminalViewOptions {
  /** Whether the cursor blinks. */
  readonly cursorBlink: boolean;
  /** The font size, in pixels. */
  readonly fontSize: number;
  /** Called with what the owner types or pastes, as xterm.js encodes the keys. */
  readonly onData: (data: string) => void;
  /** Called when the view's size in characters changes. */
  readonly onResize: (columns: number, rows: number) => void;
  /** Called when the selection changes. */
  readonly onSelection?: () => void;
  /**
   * Keys that the terminal leaves to the page, such as the shell's own keys
   * and the key that moves the focus out, matched by `KeyboardEvent.code`
   * with Ctrl or Cmd.
   */
  readonly passKeys?: readonly string[];
  /** How the view draws. */
  readonly renderer: TerminalRenderer;
  /** Whether the view serves screen readers, which costs some speed. */
  readonly screenReader: boolean;
  /** The lines of scrollback that the view keeps. */
  readonly scrollback: number;
}

/** Where a view's buffer stands: the line at the top of the screen, and how many lines it holds. */
export interface TerminalBuffer {
  readonly baseY: number;
  readonly length: number;
  readonly rows: number;
}

/** A terminal's view, which a plugin's component writes into. */
export interface TerminalView {
  /** Where the buffer stands. */
  readonly buffer: () => TerminalBuffer;
  /** Clears the find's marks. */
  readonly clearFind: () => void;
  /** Frees the view and its renderer. */
  readonly dispose: () => void;
  /** Finds text from the selection on, and says whether it found any. */
  readonly find: (text: string, direction: 'next' | 'previous') => boolean;
  /** Fits the view to its element, on the next frame. */
  readonly fit: () => void;
  /** Moves the focus into the view. */
  readonly focus: () => void;
  /** Selects lines of the buffer, from `first` to `last`, and scrolls them into view. */
  readonly markLines: (first: number, last: number) => void;
  /** Clears the view and its scrollback, as before a full snapshot. */
  readonly reset: () => void;
  /** The selected text. */
  readonly selection: () => string;
  /** Writes text, and calls `written` once xterm.js processed all of it. */
  readonly write: (data: string, written: () => void) => void;
}

/** The most characters that one write hands xterm.js, so that it yields between pieces. */
const WRITE_PIECE = 32 * 1024;

/** The words of a renderer string that mean a software rasterizer. */
const SOFTWARE_RENDERERS = ['swiftshader', 'llvmpipe', 'softpipe', 'software', 'basic render'];

/** Whether the webview reports a WebGL2 context with hardware acceleration. */
function hasHardwareWebgl(): boolean {
  const canvas = document.createElement('canvas');
  const context = canvas.getContext('webgl2');
  if (context === null) {
    return false;
  }
  const info = context.getExtension('WEBGL_debug_renderer_info');
  const renderer = info === null ? '' : String(context.getParameter(info.UNMASKED_RENDERER_WEBGL));
  context.getExtension('WEBGL_lose_context')?.loseContext();
  const name = renderer.toLowerCase();
  return !SOFTWARE_RENDERERS.some((word) => name.includes(word));
}

/** Whether a view draws with the GPU: always, never, or where the webview has hardware WebGL2. */
function usesGpu(renderer: TerminalRenderer): boolean {
  switch (renderer) {
    case 'automatic': {
      return hasHardwareWebgl();
    }
    case 'compatible': {
      return false;
    }
    case 'gpu': {
      return true;
    }
  }
}

/** A CSS color of the page as `#rrggbb`, which xterm.js reads; tokens are in oklch. */
function hexOf(color: string, fallback: string): string {
  const canvas = document.createElement('canvas');
  canvas.width = 1;
  canvas.height = 1;
  const context = canvas.getContext('2d', { willReadFrequently: true });
  if (context === null) {
    return fallback;
  }
  if (color.trim() === '') {
    return fallback;
  }
  context.fillStyle = fallback;
  context.fillStyle = color;
  context.fillRect(0, 0, 1, 1);
  const [red = 0, green = 0, blue = 0] = context.getImageData(0, 0, 1, 1).data;
  return `#${[red, green, blue].map((channel) => channel.toString(16).padStart(2, '0')).join('')}`;
}

/** The value of one of Mannie's tokens. */
function token(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name);
}

/** Whether the page shows the dark theme. */
function isDark(): boolean {
  return document.documentElement.classList.contains('dark');
}

/**
 * The terminal's theme from Mannie's tokens. Its ANSI colors keep a contrast
 * of at least 4.5:1 against the background in either theme, apart from black
 * on the dark one, which programs mostly use as a background; the view's
 * `minimumContrastRatio` raises such text as it draws it.
 */
export function terminalTheme(): ITheme {
  const dark = isDark();
  const background = hexOf(token('--background'), dark ? '#0a0a0a' : '#ffffff');
  const foreground = hexOf(token('--foreground'), dark ? '#fafafa' : '#0a0a0a');
  const selection = hexOf(token('--accent'), dark ? '#262626' : '#f5f5f5');
  const ansi = dark
    ? {
        black: '#3f3f46',
        blue: '#60a5fa',
        brightBlack: '#a1a1aa',
        brightBlue: '#93c5fd',
        brightCyan: '#67e8f9',
        brightGreen: '#86efac',
        brightMagenta: '#f0abfc',
        brightRed: '#fca5a5',
        brightWhite: '#ffffff',
        brightYellow: '#fde047',
        cyan: '#22d3ee',
        green: '#4ade80',
        magenta: '#e879f9',
        red: '#f87171',
        white: '#e4e4e7',
        yellow: '#facc15',
      }
    : {
        black: '#18181b',
        blue: '#1d4ed8',
        brightBlack: '#52525b',
        brightBlue: '#1e40af',
        brightCyan: '#0e7490',
        brightGreen: '#15803d',
        brightMagenta: '#a21caf',
        brightRed: '#b91c1c',
        brightWhite: '#27272a',
        brightYellow: '#854d0e',
        cyan: '#0e7490',
        green: '#15803d',
        magenta: '#a21caf',
        red: '#b91c1c',
        white: '#3f3f46',
        yellow: '#a16207',
      };
  return {
    ...ansi,
    background,
    cursor: foreground,
    cursorAccent: background,
    foreground,
    selectionBackground: selection,
    selectionForeground: foreground,
  };
}

/**
 * Opens a terminal's view in `element`. xterm.js and its addons load the
 * first time, as a chunk of their own with their stylesheet.
 */
export async function createTerminalView(element: HTMLElement, options: TerminalViewOptions): Promise<TerminalView> {
  const [{ Terminal: XtermTerminal }, { FitAddon }, { SearchAddon }, { WebLinksAddon }] = await Promise.all([
    import('@xterm/xterm'),
    import('@xterm/addon-fit'),
    import('@xterm/addon-search'),
    import('@xterm/addon-web-links'),
    import('@xterm/xterm/css/xterm.css'),
  ]);
  const settings: ITerminalOptions = {
    allowProposedApi: true,
    cursorBlink: options.cursorBlink,
    cursorStyle: 'block',
    fontFamily: token('--font-mono').trim() || 'ui-monospace, monospace',
    fontSize: options.fontSize,
    // xterm.js raises a text color below 4.5:1 against its background, the
    // level of WCAG 2.1 AA, whether it comes from the theme or a program.
    minimumContrastRatio: 4.5,
    scrollback: options.scrollback,
    screenReaderMode: options.screenReader,
    smoothScrollDuration: 0,
    theme: terminalTheme(),
  };
  const terminal: Terminal = new XtermTerminal(settings);
  const fit = new FitAddon();
  const search = new SearchAddon({ highlightLimit: 1000 });
  terminal.loadAddon(fit);
  terminal.loadAddon(search);
  terminal.loadAddon(
    new WebLinksAddon((_event, uri) => {
      void openLink(uri);
    }),
  );
  const passKeys = new Set(options.passKeys ?? []);
  terminal.attachCustomKeyEventHandler((event) => {
    const withCommandKey = event.ctrlKey || event.metaKey;
    // The page's own keys, such as the palette's, go past the terminal.
    return !(withCommandKey && passKeys.has(event.code));
  });
  terminal.open(element);
  if (usesGpu(options.renderer)) {
    try {
      const { WebglAddon } = await import('@xterm/addon-webgl');
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        // The DOM renderer takes over, as after a sleep or a switch of GPU.
        webgl.dispose();
      });
      terminal.loadAddon(webgl);
    } catch {
      // Without WebGL, the DOM renderer draws.
    }
  }
  const data = terminal.onData((typed) => {
    options.onData(typed);
  });
  const resize = terminal.onResize(({ cols, rows }) => {
    options.onResize(cols, rows);
  });
  const selection = terminal.onSelectionChange(() => {
    options.onSelection?.();
  });
  let frame: number | null = null;
  const fitNow = (): void => {
    frame = null;
    try {
      fit.fit();
    } catch {
      // An element without a size, such as a hidden one, keeps the last size.
    }
  };
  fitNow();
  const theme = new MutationObserver(() => {
    terminal.options.theme = terminalTheme();
  });
  theme.observe(document.documentElement, { attributeFilter: ['class'], attributes: true });
  return {
    buffer: () => ({
      baseY: terminal.buffer.active.baseY,
      length: terminal.buffer.active.length,
      rows: terminal.rows,
    }),
    clearFind: () => {
      search.clearDecorations();
    },
    dispose: () => {
      if (frame !== null) {
        cancelAnimationFrame(frame);
      }
      theme.disconnect();
      data.dispose();
      resize.dispose();
      selection.dispose();
      terminal.dispose();
    },
    find: (text, direction) =>
      direction === 'next'
        ? search.findNext(text, { incremental: false })
        : search.findPrevious(text, { incremental: false }),
    fit: () => {
      frame ??= requestAnimationFrame(fitNow);
    },
    focus: () => {
      terminal.focus();
    },
    markLines: (first, last) => {
      const top = Math.max(first, 0);
      terminal.selectLines(top, Math.max(last, top));
      terminal.scrollToLine(top);
    },
    reset: () => {
      terminal.reset();
    },
    selection: () => terminal.getSelection(),
    write: (text, written) => {
      if (text.length <= WRITE_PIECE) {
        terminal.write(text, written);
        return;
      }
      for (let start = 0; start < text.length; start += WRITE_PIECE) {
        const end = start + WRITE_PIECE;
        terminal.write(text.slice(start, end), end >= text.length ? written : undefined);
      }
    },
  };
}
