// Browser KeyboardEvent -> X11 KeySym conversion and canvas pointer coordinate
// scaling for the live VNC panel (ADR-0080).

const SPECIAL_KEYSYMS: Readonly<Partial<Record<string, number>>> = {
  Alt: 0xffe9,
  AltGraph: 0xffea,
  ArrowDown: 0xff54,
  ArrowLeft: 0xff51,
  ArrowRight: 0xff53,
  ArrowUp: 0xff52,
  Backspace: 0xff08,
  CapsLock: 0xffe5,
  ContextMenu: 0xff67,
  Control: 0xffe3,
  Delete: 0xffff,
  End: 0xff57,
  Enter: 0xff0d,
  Escape: 0xff1b,
  F1: 0xffbe,
  F10: 0xffc7,
  F11: 0xffc8,
  F12: 0xffc9,
  F2: 0xffbf,
  F3: 0xffc0,
  F4: 0xffc1,
  F5: 0xffc2,
  F6: 0xffc3,
  F7: 0xffc4,
  F8: 0xffc5,
  F9: 0xffc6,
  Home: 0xff50,
  Insert: 0xff63,
  Meta: 0xffeb,
  NumLock: 0xff7f,
  PageDown: 0xff56,
  PageUp: 0xff55,
  Pause: 0xff13,
  PrintScreen: 0xff61,
  ScrollLock: 0xff14,
  Shift: 0xffe1,
  Tab: 0xff09,
};

const RIGHT_MODIFIER_KEYSYMS: Readonly<Partial<Record<string, number>>> = {
  Alt: 0xffea,
  Control: 0xffe4,
  Meta: 0xffec,
  Shift: 0xffe2,
};

function isLatin1Keysym(code: number): boolean {
  const isPrintableAscii = code >= 0x20 && code <= 0x7e;
  const isUpperLatin1 = code >= 0xa0 && code <= 0xff;
  return isPrintableAscii || isUpperLatin1;
}

/** Maps a DOM KeyboardEvent's `key` and `location` to an X11 KeySym, or `null` when unsupported. */
export function keyboardEventToKeysym(key: string, location = 0): number | null {
  if (location === 2) {
    const rightModifier = RIGHT_MODIFIER_KEYSYMS[key];
    if (rightModifier !== undefined) {
      return rightModifier;
    }
  }

  const special = SPECIAL_KEYSYMS[key];
  if (special !== undefined) {
    return special;
  }

  const code = key.codePointAt(0);
  if (code === undefined) {
    return null;
  }

  if (String.fromCodePoint(code) !== key) {
    return null;
  }

  if (isLatin1Keysym(code)) {
    return code;
  }

  if (code > 0xff) {
    return 0x0100_0000 | code;
  }

  return null;
}

/** Converts browser pointer `buttons` bitmask into RFB button mask (bits 0=left, 1=middle, 2=right). */
export function domButtonsToRfbMask(buttons: number): number {
  let mask = 0;
  if ((buttons & 1) !== 0) {
    mask |= 1;
  }
  if ((buttons & 4) !== 0) {
    mask |= 2;
  }
  if ((buttons & 2) !== 0) {
    mask |= 4;
  }
  return mask;
}

/** Bounding client rect of the live VNC display canvas. */
export interface CanvasRect {
  readonly height: number;
  readonly left: number;
  readonly top: number;
  readonly width: number;
}

/** Framebuffer pixel coordinates scaled from a canvas pointer event. */
export interface CanvasCoords {
  readonly x: number;
  readonly y: number;
}

function hasPositiveCanvasBounds(rect: CanvasRect, fbWidth: number, fbHeight: number): boolean {
  const hasRect = rect.width > 0 && rect.height > 0;
  const hasFramebuffer = fbWidth > 0 && fbHeight > 0;
  return hasRect && hasFramebuffer;
}

/** Scales client pointer coordinates on an HTML canvas to integer RFB framebuffer pixels. */
export function scaleCanvasCoords(
  clientX: number,
  clientY: number,
  rect: CanvasRect,
  fbWidth: number,
  fbHeight: number,
): CanvasCoords | null {
  if (!hasPositiveCanvasBounds(rect, fbWidth, fbHeight)) {
    return null;
  }

  const normX = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
  const normY = Math.min(1, Math.max(0, (clientY - rect.top) / rect.height));
  return {
    x: Math.min(fbWidth - 1, Math.floor(normX * fbWidth)),
    y: Math.min(fbHeight - 1, Math.floor(normY * fbHeight)),
  };
}
