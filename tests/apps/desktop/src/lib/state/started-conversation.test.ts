// The conversation that the first message of a new conversation started
// (docs/features/chat/conversation-view.md, the message box): its pane gives
// its message box the focus once, and no other pane does.
import { describe, expect, it } from 'vitest';

import {
  markStartedConversation,
  startedConversationId,
  takeComposerFocus,
} from '../../../../../../apps/desktop/src/lib/state/started-conversation.js';

describe('the started conversation', () => {
  it('is none before a first message', () => {
    expect(startedConversationId()).toBeNull();
    expect(takeComposerFocus('started')).toBe(false);
  });

  it('gives its pane’s message box the focus once', () => {
    markStartedConversation('started');

    expect(takeComposerFocus('started')).toBe(true);
    expect(takeComposerFocus('started')).toBe(false);
    expect(startedConversationId()).toBe('started');
  });

  it('gives the focus to no pane of another conversation', () => {
    markStartedConversation('started');

    expect(takeComposerFocus('other')).toBe(false);
    expect(takeComposerFocus('started')).toBe(true);
  });

  it('is the conversation of the latest first message', () => {
    markStartedConversation('first');
    markStartedConversation('second');

    expect(startedConversationId()).toBe('second');
    expect(takeComposerFocus('first')).toBe(false);
    expect(takeComposerFocus('second')).toBe(true);
  });
});
