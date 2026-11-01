// A terminal's view in its panel
// (plugins/terminal/docs/adr/0002-keep-a-headless-emulator-of-each-terminal-in-rust.md,
// views and floods): xterm.js, fed by the channel `follow_terminal` while the
// panel's tab shows and the window is visible. A view that opens or shows
// again starts from a full snapshot, so a hidden terminal costs the page
// nothing. xterm.js opens once the view first shows: its DOM renderer
// measures each glyph as it opens and draws, and in a hidden tab every
// measurement comes out as 0, which it does not keep, so it measures again
// each time it draws.
//
// Output goes straight into xterm.js, in pieces of at most 32 KiB, and never
// through Svelte state; the state holds the commands and what the terminal
// does, which change on markers only. The view acknowledges what xterm.js
// processed every 128 KiB or 100 ms, and the core stops sending bytes to a
// view that falls behind. A resize waits 100 ms after the last change, since
// each one makes the shell redraw.
import { call } from '@mannie/plugin-sdk/ipc';
import { createTerminalView, type TerminalView } from '@mannie/plugin-sdk/terminal';

import {
  type BlockView,
  channels,
  commands,
  type EndingView,
  type TerminalPhase,
  type TerminalSettings,
  type TerminalUpdate,
} from '../generated/bindings.js';
import type { Reveal, TerminalAddress, TerminalFailure } from '../types/terminal.js';

import { Followed } from './followed.svelte.js';

/** The bytes after which a view acknowledges at once. */
const ACK_BYTES = 128 * 1024;

/** How long a view waits before it acknowledges what it wrote. */
const ACK_DELAY_MS = 100;

/** How long a resize waits for the size to settle. */
const RESIZE_DELAY_MS = 100;

/** The most commands that the list keeps. */
const MAX_BLOCKS = 200;

/**
 * The most UTF-16 code units of one write. Each takes at most 3 bytes in
 * UTF-8, so a write stays within the core's 64 KiB.
 */
const MAX_WRITE_UNITS = 16 * 1024;

/** The keys that go past the terminal to the page: the palette and the terminal's own key. */
const PASS_KEYS = ['Backquote', 'KeyK'];

/** The start of `text` that one write carries, without splitting a character of two code units. */
function firstPiece(text: string): string {
  if (text.length <= MAX_WRITE_UNITS) {
    return text;
  }
  const last = text.charCodeAt(MAX_WRITE_UNITS - 1);
  const endsInHighSurrogate = last >= 0xd8_00 && last <= 0xdb_ff;
  return text.slice(0, endsInHighSurrogate ? MAX_WRITE_UNITS - 1 : MAX_WRITE_UNITS);
}

/** A random ID for a view, which its acknowledgments name. */
function viewId(): string {
  return crypto.randomUUID();
}

/** A terminal's view. */
export class TerminalViewState {
  /** The terminal's commands, the latest last. */
  blocks: BlockView[] = $state([]);
  /** How the terminal ended, once it did. */
  ending: EndingView | null = $state(null);
  /** Why the view could not open, if it could not. */
  failure: TerminalFailure | null = $state(null);
  /** Whether the shell marks its commands. */
  marking = $state(false);
  /** What the terminal does. */
  phase: TerminalPhase = $state('starting');
  /** Whether the view is ready, once xterm.js loaded and the first snapshot came. */
  ready = $state(false);
  /** The text that the owner selected in the terminal. */
  selectedText = $state('');
  /** The user and folder that the terminal reported last. */
  state: string | null = $state(null);
  readonly #address: TerminalAddress;
  /** The settings that xterm.js opens with. */
  readonly #settings: TerminalSettings;
  /** What the owner typed that no write has carried yet, in order. */
  #typed = '';
  /** Whether a write is on its way, which the next one waits for. */
  #writing = false;
  #ackTimer: ReturnType<typeof setTimeout> | null = null;
  #acknowledged = 0;
  /** The absolute row of the screen's top and the buffer's line there, after the last write. */
  #anchor = { baseY: 0, topRow: 0 };
  /**
   * Whether the view waits for the snapshot that starts its subscription. A
   * view that follows again shows what it showed before until then, and the
   * snapshot resets xterm.js, so lines that it marked before would go.
   */
  #awaitingSnapshot = false;
  #disposed = false;
  /** Where xterm.js opens once the view shows. */
  #element: HTMLElement | null = null;
  readonly #followed: Followed<TerminalUpdate>;
  /** Whether the view follows the terminal. */
  #following = false;
  /** Whether xterm.js is loading, which it does once. */
  #opening = false;
  #pendingReveal: Reveal | null = null;
  #resizeTimer: ReturnType<typeof setTimeout> | null = null;
  /** Whether the panel's tab shows the view. */
  #tabShown = false;
  #view: TerminalView | null = null;
  readonly #viewId = viewId();
  /** Whether the window is visible. */
  #windowShown = true;
  #written = 0;

  constructor(address: TerminalAddress, settings: TerminalSettings) {
    this.#address = address;
    // A copy, so that opening xterm.js later reads no state of the plugin.
    this.#settings = { ...settings };
    this.#followed = new Followed(
      (channel) =>
        channels.followTerminal(
          {
            conversationId: address.conversationId,
            scrollback: this.#settings.scrollback,
            terminal: address.terminal,
            view: this.#viewId,
          },
          channel,
        ),
      (update) => {
        this.#apply(update);
      },
    );
  }

  /** The terminal that the view shows. */
  get address(): TerminalAddress {
    return this.#address;
  }

  /** Why following the terminal failed, if it did. */
  get followFailure(): TerminalFailure | null {
    return this.#followed.failure;
  }

  /** Gives the view the element where xterm.js opens, once the view shows. */
  attach(element: HTMLElement): void {
    this.#element = element;
    this.#update();
  }

  /** Says whether the panel's tab shows the view. */
  setTabShown(shown: boolean): void {
    this.#tabShown = shown;
    this.#update();
  }

  /** Says whether the window is visible. */
  setWindowShown(shown: boolean): void {
    this.#windowShown = shown;
    this.#update();
  }

  /**
   * Follows the terminal while the view shows, and stops while it hides. A
   * view that shows for the first time opens xterm.js first.
   */
  #update(): void {
    if (this.#disposed) {
      return;
    }
    const isShown = this.#tabShown && this.#windowShown;
    if (!isShown) {
      if (this.#following) {
        this.#following = false;
        this.#followed.stop();
      }
      return;
    }
    if (this.#view === null) {
      void this.#open();
      return;
    }
    if (!this.#following) {
      this.#following = true;
      this.#awaitingSnapshot = true;
      this.#followed.start();
    }
  }

  /** Opens xterm.js in the view's element, and follows the terminal if the view still shows. */
  async #open(): Promise<void> {
    const element = this.#element;
    if (element === null) {
      return;
    }
    if (this.#opening) {
      return;
    }
    this.#opening = true;
    const settings = this.#settings;
    const view = await createTerminalView(element, {
      cursorBlink: settings.cursorBlink,
      fontSize: settings.fontSize,
      onData: (data) => {
        void this.#type(data);
      },
      onResize: (columns, rows) => {
        this.#resizeSoon(columns, rows);
      },
      onSelection: () => {
        this.selectedText = this.#view?.selection() ?? '';
      },
      passKeys: PASS_KEYS,
      renderer: settings.renderer,
      screenReader: settings.screenReader,
      scrollback: settings.scrollback,
    });
    if (this.#disposed) {
      view.dispose();
      return;
    }
    this.#view = view;
    this.#update();
  }

  /** Stops following and frees xterm.js. */
  dispose(): void {
    this.#disposed = true;
    this.#following = false;
    this.#followed.stop();
    if (this.#ackTimer !== null) {
      clearTimeout(this.#ackTimer);
    }
    if (this.#resizeTimer !== null) {
      clearTimeout(this.#resizeTimer);
    }
    this.#view?.dispose();
    this.#view = null;
  }

  /** Fits the view to its element. */
  fit(): void {
    this.#view?.fit();
  }

  /** Moves the focus into the terminal. */
  focus(): void {
    this.#view?.focus();
  }

  /** Finds text in the terminal, and says whether it found any. */
  find(text: string, direction: 'next' | 'previous'): boolean {
    return this.#view?.find(text, direction) ?? false;
  }

  /** Clears the find's marks. */
  clearFind(): void {
    this.#view?.clearFind();
  }

  /** Scrolls to a command and marks it, or its lines, once the view has it. */
  reveal(reveal: Reveal): void {
    this.#pendingReveal = reveal;
    this.#revealNow();
  }

  #revealNow(): void {
    const reveal = this.#pendingReveal;
    const view = this.#view;
    if (reveal === null) {
      return;
    }
    if (view === null) {
      return;
    }
    const showsCurrent = this.ready && !this.#awaitingSnapshot;
    if (!showsCurrent) {
      return;
    }
    const block = this.blocks.find((candidate) => candidate.number === reveal.block);
    if (block === undefined) {
      return;
    }
    this.#pendingReveal = null;
    const firstRow = block.firstRow + (reveal.lines === null ? 0 : reveal.lines.first - 1);
    const lastRow =
      reveal.lines === null ? Math.max(block.endRow - 1, block.firstRow) : block.firstRow + reveal.lines.last - 1;
    view.markLines(this.#lineOf(firstRow), this.#lineOf(lastRow));
  }

  /** The buffer's line of an absolute row, after the last write. */
  #lineOf(row: number): number {
    return this.#anchor.baseY + (row - this.#anchor.topRow);
  }

  #apply(update: TerminalUpdate): void {
    switch (update.kind) {
      case 'snapshot': {
        this.#view?.reset();
        this.#write(update.data, update.seq, update.topRow, () => {
          this.#awaitingSnapshot = false;
          this.ready = true;
          this.#revealNow();
        });
        return;
      }
      case 'screen':
      case 'output': {
        this.#write(update.data, update.seq, update.topRow);
        return;
      }
      case 'blocks': {
        this.blocks = update.blocks.slice(-MAX_BLOCKS);
        this.#revealNow();
        return;
      }
      case 'block': {
        const index = this.blocks.findIndex((block) => block.number === update.block.number);
        this.blocks =
          index === -1 ? [...this.blocks, update.block].slice(-MAX_BLOCKS) : this.blocks.with(index, update.block);
        this.#revealNow();
        return;
      }
      case 'status': {
        this.phase = update.phase;
        this.marking = update.marking;
        this.state = update.state;
        return;
      }
      case 'ended': {
        this.ending = update.ending;
        this.phase = 'ended';
      }
    }
  }

  #write(data: string, seq: number, topRow: number, then?: () => void): void {
    const view = this.#view;
    if (view === null) {
      return;
    }
    view.write(data, () => {
      this.#anchor = { baseY: view.buffer().baseY, topRow };
      this.#written = seq;
      this.#acknowledgeSoon();
      then?.();
    });
  }

  #acknowledgeSoon(): void {
    if (this.#written - this.#acknowledged >= ACK_BYTES) {
      this.#acknowledge();
      return;
    }
    this.#ackTimer ??= setTimeout(() => {
      this.#acknowledge();
    }, ACK_DELAY_MS);
  }

  #acknowledge(): void {
    if (this.#ackTimer !== null) {
      clearTimeout(this.#ackTimer);
      this.#ackTimer = null;
    }
    if (this.#written === this.#acknowledged) {
      return;
    }
    this.#acknowledged = this.#written;
    void call(() =>
      commands.acknowledgeOutput({
        conversationId: this.#address.conversationId,
        seq: this.#written,
        terminal: this.#address.terminal,
        view: this.#viewId,
      }),
    );
  }

  /**
   * Hands what the owner typed to the terminal, one write at a time. The core
   * may handle calls side by side, so a write that did not wait for the one
   * before it could reach the shell first and swap the keys. What the owner
   * types meanwhile goes with the next write, in pieces within the core's
   * limit, so a fast typist or a long paste costs few calls.
   */
  async #type(data: string): Promise<void> {
    this.#typed += data;
    if (this.#writing) {
      return;
    }
    this.#writing = true;
    try {
      while (this.#typed !== '') {
        const piece = firstPiece(this.#typed);
        this.#typed = this.#typed.slice(piece.length);
        const outcome = await call(() =>
          commands.writeTerminal({
            conversationId: this.#address.conversationId,
            data: piece,
            terminal: this.#address.terminal,
          }),
        );
        if (!outcome.ok) {
          // What waited would reach the shell without what failed before it.
          this.#typed = '';
          if (this.ending === null) {
            this.failure = outcome.failure;
          }
          return;
        }
      }
    } finally {
      this.#writing = false;
    }
  }

  #resizeSoon(columns: number, rows: number): void {
    if (this.#resizeTimer !== null) {
      clearTimeout(this.#resizeTimer);
    }
    this.#resizeTimer = setTimeout(() => {
      this.#resizeTimer = null;
      void call(() =>
        commands.resizeTerminal({
          columns,
          conversationId: this.#address.conversationId,
          rows,
          terminal: this.#address.terminal,
        }),
      );
    }, RESIZE_DELAY_MS);
  }
}
