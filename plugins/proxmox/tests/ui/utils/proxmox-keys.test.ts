// Unit tests for Proxmox VNC keyboard mapping, pointer masks, coordinate
// scaling, and side-panel argument encoding.
import { describe, expect, it } from 'vitest';

import { domButtonsToRfbMask, keyboardEventToKeysym, scaleCanvasCoords } from '../../../ui/utils/proxmox-keys.js';
import { formatVncPanelArgument, parseVncPanelArgument } from '../../../ui/utils/proxmox-panel.js';

describe('proxmox VNC key and pointer helpers', () => {
  it('maps printable ASCII, Unicode, special keys, and right-side modifiers to X11 KeySyms', () => {
    expect(keyboardEventToKeysym('a')).toBe(0x0061);
    expect(keyboardEventToKeysym('A')).toBe(0x0041);
    expect(keyboardEventToKeysym('Enter')).toBe(0xff0d);
    expect(keyboardEventToKeysym('Escape')).toBe(0xff1b);
    expect(keyboardEventToKeysym('F12')).toBe(0xffc9);
    expect(keyboardEventToKeysym('CapsLock')).toBe(0xffe5);
    expect(keyboardEventToKeysym('Control', 1)).toBe(0xffe3);
    expect(keyboardEventToKeysym('Control', 2)).toBe(0xffe4);
    expect(keyboardEventToKeysym('Shift', 2)).toBe(0xffe2);
    expect(keyboardEventToKeysym('€')).toBe(0x0100_20ac);
    expect(keyboardEventToKeysym('Unidentified')).toBeNull();
  });

  it('converts DOM pointer button masks to RFB button masks', () => {
    expect(domButtonsToRfbMask(0)).toBe(0);
    expect(domButtonsToRfbMask(1)).toBe(1);
    expect(domButtonsToRfbMask(4)).toBe(2);
    expect(domButtonsToRfbMask(2)).toBe(4);
    expect(domButtonsToRfbMask(7)).toBe(7);
  });

  it('scales canvas client coordinates into integer framebuffer pixels and clamps edges', () => {
    const rect = { height: 200, left: 10, top: 20, width: 400 };
    expect(scaleCanvasCoords(10, 20, rect, 800, 600)).toEqual({ x: 0, y: 0 });
    expect(scaleCanvasCoords(210, 120, rect, 800, 600)).toEqual({ x: 400, y: 300 });
    expect(scaleCanvasCoords(999, 999, rect, 800, 600)).toEqual({ x: 799, y: 599 });
    expect(scaleCanvasCoords(0, 0, { height: 0, left: 0, top: 0, width: 400 }, 800, 600)).toBeNull();
  });

  it('round-trips VNC panel arguments and rejects malformed inputs', () => {
    const target = { conversationId: 'conv-1', serverId: 'srv-1', vmid: 105 };
    const encoded = formatVncPanelArgument(target);
    expect(parseVncPanelArgument(encoded)).toEqual(target);
    expect(parseVncPanelArgument('bad')).toBeNull();
    expect(parseVncPanelArgument('conv-1::srv-1::0')).toBeNull();
    expect(parseVncPanelArgument('::srv-1::100')).toBeNull();
  });
});
