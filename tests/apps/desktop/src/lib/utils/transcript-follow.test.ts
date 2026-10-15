// When the transcript follows a reply (docs/features/chat/conversation-view.md,
// layout): only the owner scrolling up ends the following, however much the
// content grows at once and however the layout moves the transcript.
import { describe, expect, it } from 'vitest';

import {
  followsAfterScroll,
  type ScrollPosition,
} from '../../../../../../apps/desktop/src/lib/utils/transcript-follow.js';

const FOLLOW_DISTANCE = 48;

/** A transcript `clientHeight` pixels high whose content is `scrollHeight` high and scrolled to `scrollTop`. */
function position(scrollTop: number, scrollHeight: number, clientHeight = 600): ScrollPosition {
  return { clientHeight, scrollHeight, scrollTop };
}

describe('followsAfterScroll', () => {
  it('keeps following when a scroll event arrives after the content grew again', () => {
    // The transcript followed to 1,400, the bottom of 2,000 pixels; before
    // that scroll's event arrived, an approval request added 300 pixels.
    const late = position(1_400, 2_300);

    expect(followsAfterScroll(true, 1_000, late, FOLLOW_DISTANCE, false)).toBe(true);
  });

  it('keeps following when the layout moves the transcript up while the content grows', () => {
    // A sent message leaves the message box, which shrinks: the transcript
    // grows 120 pixels taller and moves up to its bottom, and the new turn
    // has added 300 pixels before the scroll's event arrives.
    const afterSending = position(1_280, 2_300, 720);

    expect(followsAfterScroll(true, 1_400, afterSending, FOLLOW_DISTANCE, false)).toBe(true);
  });

  it('stops following when the owner scrolls up', () => {
    const scrolledUp = position(1_200, 2_000);

    expect(followsAfterScroll(true, 1_400, scrolledUp, FOLLOW_DISTANCE, true)).toBe(false);
  });

  it('keeps following when the owner moves up less than the follow distance', () => {
    const nudged = position(1_370, 2_000);

    expect(followsAfterScroll(true, 1_400, nudged, FOLLOW_DISTANCE, true)).toBe(true);
  });

  it('follows again once the owner reaches the bottom', () => {
    const atBottom = position(1_400, 2_000);

    expect(followsAfterScroll(false, 900, atBottom, FOLLOW_DISTANCE, true)).toBe(true);
  });

  it('stays where the owner left it while the owner scrolls down short of the bottom', () => {
    const reading = position(1_000, 2_000);

    expect(followsAfterScroll(false, 900, reading, FOLLOW_DISTANCE, true)).toBe(false);
  });

  it('keeps following when the content shrinks under the owner', () => {
    // A request that was decided leaves; the browser moves the view up to
    // the new bottom.
    const shrunk = position(1_100, 1_700);

    expect(followsAfterScroll(true, 1_400, shrunk, FOLLOW_DISTANCE, false)).toBe(true);
  });
});
