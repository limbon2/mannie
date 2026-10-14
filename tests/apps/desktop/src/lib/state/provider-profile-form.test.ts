// The provider profile form (docs/features/chat/conversation-view.md,
// settings sections): what it sends to the core for each choice the owner
// makes about the credential and the limits.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ModelDefaults, ProviderProfile } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { ProviderProfileForm } from '../../../../../../apps/desktop/src/lib/state/provider-profile-form.svelte.js';

const core = vi.hoisted(() => ({
  create: vi.fn(),
  getModelDefaults: vi.fn(),
  update: vi.fn(),
}));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({
  commands: { getModelDefaults: core.getModelDefaults },
}));
vi.mock('../../../../../../apps/desktop/src/lib/state/provider-profiles.svelte.js', () => ({
  providerProfiles: { create: core.create, update: core.update },
}));

function storedProfile(hasCredential: boolean): ProviderProfile {
  return {
    baseUrl: 'https://api.openai.com/v1',
    contextWindow: 400_000,
    createdAt: 0,
    hasCredential,
    id: '0199e0f2-0000-7000-8000-0000000000ff',
    interface: 'responses',
    isDefault: true,
    model: 'gpt-5',
    name: 'OpenAI',
    outputLimit: 128_000,
    reasoningEffort: 'medium',
    server: 'open_ai',
    updatedAt: 0,
  };
}

const KNOWN_MODEL: ModelDefaults = { limits: { contextWindow: 128_000, outputLimit: 16_384 }, reasons: false };

beforeEach(() => {
  core.create.mockReset();
  core.getModelDefaults.mockReset();
  core.update.mockReset();
  core.create.mockResolvedValue({ data: storedProfile(true), ok: true });
  core.update.mockResolvedValue({ data: storedProfile(true), ok: true });
});

describe('a new profile', () => {
  it('asks for both limits before it reaches the core', async () => {
    const form = new ProviderProfileForm(null);
    form.contextWindow = 8_192;

    expect(await form.save()).toBe(false);

    expect(form.limitsMissing).toBe(true);
    expect(core.create).not.toHaveBeenCalled();
  });

  it('sends a credential only when the owner entered one', async () => {
    const form = new ProviderProfileForm(null);
    form.contextWindow = 8_192;
    form.outputLimit = 1_024;
    await form.save();
    form.credential = 'planted-credential';
    await form.save();

    expect(core.create).toHaveBeenNthCalledWith(1, expect.objectContaining({ credential: null }));
    expect(core.create).toHaveBeenNthCalledWith(2, expect.objectContaining({ credential: 'planted-credential' }));
    expect(form.credential).toBe('');
  });

  it('fills in the limits of a known model unless the owner set them', async () => {
    core.getModelDefaults.mockResolvedValue(KNOWN_MODEL);
    const form = new ProviderProfileForm(null);
    form.model = ' gpt-4o ';

    await form.applyModelDefaults();

    expect(core.getModelDefaults).toHaveBeenCalledWith({ model: 'gpt-4o' });
    expect(form.contextWindow).toBe(128_000);
    expect(form.outputLimit).toBe(16_384);
    expect(form.defaultsApplied).toBe(true);
  });

  it('keeps the limits the owner set while the core answered', async () => {
    let answer: (defaults: ModelDefaults) => void = () => undefined;
    core.getModelDefaults.mockReturnValue(
      new Promise<ModelDefaults>((resolve) => {
        answer = resolve;
      }),
    );
    const form = new ProviderProfileForm(null);
    form.model = 'gpt-4o';

    const applying = form.applyModelDefaults();
    form.contextWindow = 4_096;
    form.limitsTouched = true;
    answer(KNOWN_MODEL);
    await applying;

    expect(form.contextWindow).toBe(4_096);
    expect(form.defaultsApplied).toBe(false);
  });
});

describe('an edited profile', () => {
  it('starts from the profile as stored, with its limits as the owner set them', async () => {
    const form = new ProviderProfileForm(storedProfile(true));

    await form.applyModelDefaults();

    expect(form.name).toBe('OpenAI');
    expect(form.hasStoredCredential).toBe(true);
    expect(form.credential).toBe('');
    expect(core.getModelDefaults).not.toHaveBeenCalled();
  });

  it('keeps a stored credential while the field is empty, and replaces it with what the owner types', () => {
    const form = new ProviderProfileForm(storedProfile(true));
    const changes = [form.credentialChange()];
    form.credential = 'new-credential';
    changes.push(form.credentialChange());
    form.credential = '';
    changes.push(form.credentialChange());

    expect(changes).toEqual([
      { action: 'keep' },
      { action: 'replace', credential: 'new-credential' },
      { action: 'keep' },
    ]);
  });

  it('removes a stored credential when the owner asks, until the owner keeps it after all', () => {
    const form = new ProviderProfileForm(storedProfile(true));
    form.credential = 'typed-before-removing';

    form.removeStoredCredential();
    const removing = form.credentialChange();
    const typed = form.credential;
    form.keepStoredCredential();

    expect(removing).toEqual({ action: 'remove' });
    expect(typed).toBe('');
    expect(form.credentialChange()).toEqual({ action: 'keep' });
  });

  it('sends the removal with the profile', async () => {
    const form = new ProviderProfileForm(storedProfile(true));
    form.removeStoredCredential();

    expect(await form.save()).toBe(true);

    expect(core.update).toHaveBeenCalledWith(expect.objectContaining({ credential: { action: 'remove' } }));
  });

  it('changes nothing when a profile without a credential gets none, and has nothing to remove', () => {
    const form = new ProviderProfileForm(storedProfile(false));

    form.removeStoredCredential();

    expect(form.removesCredential).toBe(false);
    expect(form.credentialChange()).toEqual({ action: 'keep' });
  });

  it('shows why the core refused the profile', async () => {
    core.update.mockResolvedValue({ failure: { error: { code: 'name_taken' }, kind: 'core' }, ok: false });
    const form = new ProviderProfileForm(storedProfile(true));

    expect(await form.save()).toBe(false);

    expect(form.failureCode).toBe('name_taken');
    expect(form.saving).toBe(false);
  });
});
