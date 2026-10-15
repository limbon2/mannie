// When the transcript follows a reply (docs/features/chat/conversation-view.md,
// layout): it follows while the owner is at its bottom, and only the owner
// scrolling up ends the following.

/** Where a scrolling element stands. */
export interface ScrollPosition {
  readonly clientHeight: number;
  readonly scrollHeight: number;
  readonly scrollTop: number;
}

/**
 * Whether the transcript follows the reply after a scroll event. Reaching the
 * bottom starts the following again. Away from the bottom, only a move up
 * that the owner made ends it: the transcript also moves up when the message
 * box shrinks or the window grows taller, and a scroll event can arrive after
 * the content grew again, far above the new bottom, so the position alone
 * cannot tell the owner's move from the layout's.
 */
export function followsAfterScroll(
  following: boolean,
  previousTop: number,
  position: ScrollPosition,
  followDistance: number,
  ownerScrolls: boolean,
): boolean {
  const distance = position.scrollHeight - position.scrollTop - position.clientHeight;
  if (distance <= followDistance) {
    return true;
  }
  const movedUp = position.scrollTop < previousTop;
  const ownerMovedUp = movedUp && ownerScrolls;
  return ownerMovedUp ? false : following;
}
