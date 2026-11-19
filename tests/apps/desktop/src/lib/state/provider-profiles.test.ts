import { describe, expect, it, vi } from 'vitest';

import type { ProviderProfile } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { ProviderProfiles } from '../../../../../../apps/desktop/src/lib/state/provider-profiles.svelte.js';

const core = vi.hoisted(() => ({ listProviderProfiles: vi.fn(), refreshChatgptCatalog: vi.fn() }));
vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));

function model(id: string, fields: Partial<ProviderProfile> = {}): ProviderProfile {
  return {
    access: 'chatgpt_plan',
    baseUrl: 'https://api.openai.com/v1',
    catalog: { available: true },
    contextWindow: 32768,
    createdAt: 0,
    hasCredential: false,
    id,
    interface: 'responses',
    isDefault: false,
    model: id,
    name: id,
    outputLimit: 4096,
    reasoningEffort: 'server_default',
    server: null,
    updatedAt: 0,
    useTools: true,
    ...fields,
  };
}

describe('models available in chat', () => {
  it('keeps the catalog in order and gives the saved default precedence', () => {
    const choices = new ProviderProfiles();
    const first = model('first');
    const second = model('second');
    const removed = model('removed', { catalog: { available: false } });
    const custom = model('custom', { catalog: undefined, isDefault: true, model: first.model });
    choices.profiles = [custom, removed, first, second];

    expect(choices.configured).toEqual([custom]);
    expect(choices.planModels).toEqual([first, second]);
    expect(choices.defaultProfile).toEqual(custom);
    expect(choices.find(removed.id)).toEqual(removed);

    choices.profiles = [removed, first, second];
    expect(choices.defaultProfile).toEqual(first);
  });

  it('retains the last successful catalog on a transient failure', async () => {
    const choices = new ProviderProfiles();
    const cached = model('cached');
    core.refreshChatgptCatalog.mockResolvedValueOnce({ error: { code: 'network' }, status: 'error' });
    core.listProviderProfiles.mockResolvedValueOnce({ data: [cached], status: 'ok' });

    await choices.refreshCatalog();

    expect(choices.catalogStatus).toBe('error');
    expect(choices.defaultProfile).toEqual(cached);
  });

  it('discards a refresh from an account that signed out while it was running', async () => {
    const choices = new ProviderProfiles();
    const previouslyAvailable = model('previous');
    choices.profiles = [previouslyAvailable];
    const refresh = Promise.withResolvers<unknown>();
    core.refreshChatgptCatalog.mockReturnValueOnce(refresh.promise);
    core.listProviderProfiles.mockResolvedValueOnce({
      data: [model('previous', { catalog: { available: false } })],
      status: 'ok',
    });

    const pending = choices.refreshCatalog();
    choices.accountChanged({
      email: null,
      known: true,
      noticeSeen: true,
      signIn: { step: 'idle' },
      state: 'signed_out',
    });
    expect(choices.defaultProfile).toBeUndefined();
    refresh.resolve({ data: null, status: 'ok' });
    await pending;

    expect(choices.catalogStatus).toBe('signed_out');
    expect(choices.find('previous')?.catalog?.available).toBe(false);
  });
});
