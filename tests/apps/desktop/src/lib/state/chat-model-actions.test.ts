import { describe, expect, it, vi } from 'vitest';

import { ChatActions } from '../../../../../../apps/desktop/src/lib/state/chat-actions.svelte.js';

const core = vi.hoisted(() => ({
  retryReply: vi.fn(),
  setConversationProfile: vi.fn(),
  setConversationReasoningEffort: vi.fn(),
}));
vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));
vi.mock('$app/state', () => ({ page: { url: new URL('http://localhost/') } }));

describe('a conversation’s model choices', () => {
  it('applies a model switch and an effort change before an immediate retry', async () => {
    const actions = new ChatActions();
    const saved = Promise.withResolvers<unknown>();
    const accepted = { data: null, status: 'ok' };
    core.setConversationProfile.mockReturnValueOnce(saved.promise);
    core.setConversationReasoningEffort.mockResolvedValueOnce(accepted);
    core.retryReply.mockResolvedValueOnce(accepted);

    const profile = actions.setProfile('conversation', 'model');
    const effort = actions.setEffort('conversation', 'high');
    const retry = actions.retry('conversation');
    expect(core.setConversationReasoningEffort).not.toHaveBeenCalled();
    expect(core.retryReply).not.toHaveBeenCalled();
    expect(actions.pendingModelChange('another-chat')).toBeNull();

    saved.resolve(accepted);
    await Promise.all([profile, effort, retry]);
    expect(core.setConversationReasoningEffort).toHaveBeenCalledWith({
      conversationId: 'conversation',
      reasoningEffort: 'high',
    });
    expect(core.retryReply).toHaveBeenCalledWith({ conversationId: 'conversation' });
    expect(actions.pendingModelChange('conversation')).toBeNull();
  });

  it('returns a refused effort to a waiting retry before starting another reply', async () => {
    const actions = new ChatActions();
    const saved = Promise.withResolvers<unknown>();
    core.setConversationReasoningEffort.mockReturnValueOnce(saved.promise);
    const effort = actions.setEffort('conversation', 'high');
    const retry = actions.retry('conversation');
    const error = { code: 'unsupported_reasoning_effort' };
    saved.resolve({ error, status: 'error' });
    expect(await effort).toEqual({ error, kind: 'core' });
    expect(await retry).toEqual({ error, kind: 'core' });
  });
});
