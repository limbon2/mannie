// The terminal of the TypeScript plugin SDK (ADR-0053, decision 7): the
// theme from Mannie's tokens in both themes, with text kept at 4.5:1 against
// its background; the renderer's automatic choice, which leaves software
// WebGL to the DOM renderer; the keys that go past the terminal; writes in
// pieces that let xterm.js yield; and a view that follows the theme and frees
// everything when it goes. xterm.js and its addons are replaced by fakes,
// since the tests run without a page. The packages belong to the desktop
// app, so the mocks name them by their place in its node_modules.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  createTerminalView,
  terminalTheme,
  type TerminalViewOptions,
} from '../../../../../../apps/desktop/src/lib/sdk/terminal.js';

/** What the fakes of xterm.js record. */
const xterm = vi.hoisted(() => ({
  addons: [] as string[],
  keyHandler: undefined as ((event: KeyboardEvent) => boolean) | undefined,
  options: undefined as Record<string, unknown> | undefined,
  terminals: [] as { disposed: boolean; options: Record<string, unknown> }[],
  writes: [] as { data: string; hasCallback: boolean }[],
}));

vi.mock('../../../../../../apps/desktop/node_modules/@xterm/xterm', () => ({
  Terminal: class {
    readonly buffer = { active: { baseY: 0, length: 0 } };
    disposed = false;
    readonly options: Record<string, unknown>;
    readonly rows = 30;

    constructor(options: Record<string, unknown>) {
      this.options = { ...options };
      xterm.options = options;
      xterm.terminals.push(this);
    }

    attachCustomKeyEventHandler(handler: (event: KeyboardEvent) => boolean): void {
      xterm.keyHandler = handler;
    }

    dispose(): void {
      this.disposed = true;
    }

    loadAddon(addon: { name: string }): void {
      xterm.addons.push(addon.name);
    }

    onData(): { dispose: () => void } {
      return { dispose: vi.fn() };
    }

    onResize(): { dispose: () => void } {
      return { dispose: vi.fn() };
    }

    onSelectionChange(): { dispose: () => void } {
      return { dispose: vi.fn() };
    }

    readonly open = vi.fn();

    write(data: string, written?: () => void): void {
      xterm.writes.push({ data, hasCallback: written !== undefined });
    }
  },
}));
vi.mock('../../../../../../apps/desktop/node_modules/@xterm/addon-fit', () => ({
  FitAddon: class {
    readonly fit = vi.fn();
    readonly name = 'fit';
  },
}));
vi.mock('../../../../../../apps/desktop/node_modules/@xterm/addon-search', () => ({
  SearchAddon: class {
    readonly name = 'search';
  },
}));
vi.mock('../../../../../../apps/desktop/node_modules/@xterm/addon-web-links', () => ({
  WebLinksAddon: class {
    readonly name = 'web-links';
  },
}));
vi.mock('../../../../../../apps/desktop/node_modules/@xterm/addon-webgl', () => ({
  WebglAddon: class {
    readonly name = 'webgl';
    onContextLoss(): { dispose: () => void } {
      return { dispose: vi.fn() };
    }
  },
}));
vi.mock('../../../../../../apps/desktop/node_modules/@xterm/xterm/css/xterm.css', () => ({}));
vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: {} }));

/** The colors that the fake canvas paints, by the CSS text of each token. */
const PAINTED: Record<string, [number, number, number]> = {
  'oklch(0.145 0 0)': [10, 10, 10],
  'oklch(0.269 0 0)': [38, 38, 38],
  'oklch(0.97 0 0)': [245, 245, 245],
  'oklch(0.985 0 0)': [250, 250, 250],
  'oklch(1 0 0)': [255, 255, 255],
};

/** The page's tokens in the light and the dark theme, as shadcn-svelte sets them. */
const TOKENS = {
  dark: { '--accent': 'oklch(0.269 0 0)', '--background': 'oklch(0.145 0 0)', '--foreground': 'oklch(0.985 0 0)' },
  light: { '--accent': 'oklch(0.97 0 0)', '--background': 'oklch(1 0 0)', '--foreground': 'oklch(0.145 0 0)' },
};

/** The page that the module reads: its theme, its tokens, its canvas and its observers. */
interface FakePage {
  canvas2d: boolean;
  dark: boolean;
  observers: (() => void)[];
  webglRenderer: string | null;
}

function stubPage(): FakePage {
  const page: FakePage = { canvas2d: true, dark: false, observers: [], webglRenderer: null };
  const context2d = {
    fillRect: vi.fn(),
    fillStyle: '',
    getImageData: () => ({ data: PAINTED[context2d.fillStyle] ?? hexChannels(context2d.fillStyle) }),
  };
  const webgl = {
    getExtension: (name: string) => (name === 'WEBGL_debug_renderer_info' ? { UNMASKED_RENDERER_WEBGL: 1 } : null),
    getParameter: () => page.webglRenderer,
  };
  vi.stubGlobal('document', {
    createElement: () => ({
      getContext: (kind: string) => {
        if (kind === '2d') {
          return page.canvas2d ? context2d : null;
        }
        return page.webglRenderer === null ? null : webgl;
      },
    }),
    documentElement: {
      classList: {
        contains: (name: string) => {
          const isDarkClass = name === 'dark';
          return isDarkClass && page.dark;
        },
      },
    },
  });
  vi.stubGlobal('getComputedStyle', () => ({
    getPropertyValue: (name: string) => {
      const tokens: Record<string, string> = page.dark ? TOKENS.dark : TOKENS.light;
      return tokens[name] ?? '';
    },
  }));
  vi.stubGlobal(
    'MutationObserver',
    class {
      constructor(private readonly changed: () => void) {}
      disconnect(): void {
        page.observers = page.observers.filter((observer) => observer !== this.changed);
      }
      observe(): void {
        page.observers.push(this.changed);
      }
    },
  );
  vi.stubGlobal('requestAnimationFrame', (callback: () => void) => {
    callback();
    return 1;
  });
  vi.stubGlobal('cancelAnimationFrame', vi.fn());
  return page;
}

/** The channels of a `#rrggbb` color. */
function hexChannels(color: string): [number, number, number] {
  return [1, 3, 5].map((start) => Number.parseInt(color.slice(start, start + 2), 16)) as [number, number, number];
}

/** A color's relative luminance, as WCAG 2.1 defines it. */
function luminance(color: string): number {
  const [red, green, blue] = hexChannels(color).map((channel) => {
    const value = channel / 255;
    return value <= 0.040_45 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

/** The contrast ratio of two colors, as WCAG 2.1 defines it. */
function contrast(first: string, second: string): number {
  const [lighter, darker] = [luminance(first), luminance(second)].sort((left, right) => right - left);
  return (lighter + 0.05) / (darker + 0.05);
}

const ANSI_NAMES = [
  'black',
  'red',
  'green',
  'yellow',
  'blue',
  'magenta',
  'cyan',
  'white',
  'brightBlack',
  'brightRed',
  'brightGreen',
  'brightYellow',
  'brightBlue',
  'brightMagenta',
  'brightCyan',
  'brightWhite',
] as const;

function options(overrides: Partial<TerminalViewOptions> = {}): TerminalViewOptions {
  return {
    cursorBlink: false,
    fontSize: 13,
    onData: vi.fn(),
    onResize: vi.fn(),
    renderer: 'compatible',
    screenReader: false,
    scrollback: 2000,
    ...overrides,
  };
}

let page: FakePage;

beforeEach(() => {
  page = stubPage();
  xterm.addons = [];
  xterm.keyHandler = undefined;
  xterm.options = undefined;
  xterm.terminals = [];
  xterm.writes = [];
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('terminalTheme', () => {
  it("takes the light theme's colors from the tokens, with every ANSI color at 4.5:1 or more", () => {
    const theme = terminalTheme();

    expect(theme.background).toBe('#ffffff');
    expect(theme.foreground).toBe('#0a0a0a');
    expect(theme.selectionBackground).toBe('#f5f5f5');
    for (const name of ANSI_NAMES) {
      expect(contrast(theme[name] ?? '', '#ffffff'), name).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("takes the dark theme's colors from the tokens, with every ANSI color but black at 4.5:1 or more", () => {
    page.dark = true;

    const theme = terminalTheme();

    expect(theme.background).toBe('#0a0a0a');
    expect(theme.foreground).toBe('#fafafa');
    expect(theme.selectionBackground).toBe('#262626');
    for (const name of ANSI_NAMES.filter((ansi) => ansi !== 'black')) {
      expect(contrast(theme[name] ?? '', '#0a0a0a'), name).toBeGreaterThanOrEqual(4.5);
    }
    expect(contrast(theme.foreground ?? '', theme.background ?? '')).toBeGreaterThanOrEqual(4.5);
  });

  it('falls back to the default colors of each theme without a canvas to read the tokens', () => {
    page.canvas2d = false;
    const light = terminalTheme();
    page.dark = true;
    const dark = terminalTheme();

    expect([light.background, light.foreground]).toEqual(['#ffffff', '#0a0a0a']);
    expect([dark.background, dark.foreground]).toEqual(['#0a0a0a', '#fafafa']);
  });
});

describe('createTerminalView', () => {
  it('opens xterm.js with the options and keeps text at 4.5:1, without a clipboard addon', async () => {
    await createTerminalView({} as HTMLElement, options({ cursorBlink: true, scrollback: 5000, screenReader: true }));

    expect(xterm.options).toMatchObject({
      cursorBlink: true,
      minimumContrastRatio: 4.5,
      screenReaderMode: true,
      scrollback: 5000,
      smoothScrollDuration: 0,
    });
    expect(xterm.addons).toEqual(['fit', 'search', 'web-links']);
  });

  it('draws with WebGL where the webview has it in hardware, and leaves software WebGL to the DOM renderer', async () => {
    page.webglRenderer = 'ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)';
    await createTerminalView({} as HTMLElement, options({ renderer: 'automatic' }));
    const software = [...xterm.addons];
    xterm.addons = [];
    page.webglRenderer = 'ANGLE (Intel, Intel(R) UHD Graphics 620 Direct3D11 vs_5_0 ps_5_0, D3D11)';
    await createTerminalView({} as HTMLElement, options({ renderer: 'automatic' }));
    const hardware = [...xterm.addons];
    xterm.addons = [];
    await createTerminalView({} as HTMLElement, options({ renderer: 'compatible' }));

    expect(software).not.toContain('webgl');
    expect(hardware).toContain('webgl');
    expect(xterm.addons).not.toContain('webgl');
  });

  it("leaves the page's keys to the page", async () => {
    await createTerminalView({} as HTMLElement, options({ passKeys: ['Backquote'] }));
    const handler = xterm.keyHandler;

    expect(handler?.({ code: 'Backquote', ctrlKey: true, metaKey: false } as KeyboardEvent)).toBe(false);
    expect(handler?.({ code: 'Backquote', ctrlKey: false, metaKey: false } as KeyboardEvent)).toBe(true);
    expect(handler?.({ code: 'KeyC', ctrlKey: true, metaKey: false } as KeyboardEvent)).toBe(true);
  });

  it('writes long output in pieces of 32 KiB and says once that all of it was processed', async () => {
    const view = await createTerminalView({} as HTMLElement, options());

    view.write('x'.repeat(70 * 1024), vi.fn());

    expect(xterm.writes.map((write) => write.data.length)).toEqual([32 * 1024, 32 * 1024, 6 * 1024]);
    expect(xterm.writes.map((write) => write.hasCallback)).toEqual([false, false, true]);
  });

  it('follows the theme of the page, and frees the terminal and its observer when it goes', async () => {
    const view = await createTerminalView({} as HTMLElement, options());
    const terminal = xterm.terminals.at(-1);
    page.dark = true;

    for (const changed of page.observers) {
      changed();
    }
    const followed = terminal?.options.theme;
    view.dispose();

    expect(followed).toMatchObject({ background: '#0a0a0a' });
    expect(terminal?.disposed).toBe(true);
    expect(page.observers).toEqual([]);
  });
});
