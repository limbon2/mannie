// The provider profile form (docs/features/chat/conversation-view.md,
// settings sections): what it sends to the core for each choice the owner
// makes about the credential and the limits, and for a profile on the
// ChatGPT plan, which picks its model from the plan's catalog and has no
// interface, base URL or credential of its own (docs/features/
// chatgpt-plan.md, plan profiles).
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  ChatGptModel,
  ModelDefaults,
  ProviderProfile,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { ProviderProfileForm } from '../../../../../../apps/desktop/src/lib/state/provider-profile-form.svelte.js';
import type { Outcome } from '../../../../../../apps/desktop/src/lib/types/calls.js';

const core = vi.hoisted(() => ({
  create: vi.fn(),
  getModelDefaults: vi.fn(),
  models: vi.fn(),
  update: vi.fn(),
}));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({
  commands: { getModelDefaults: core.getModelDefaults },
}));
vi.mock('../../../../../../apps/desktop/src/lib/state/provider-profiles.svelte.js', () => ({
  providerProfiles: { create: core.create, update: core.update },
}));
vi.mock('../../../../../../apps/desktop/src/lib/state/chatgpt-account.svelte.js', () => ({
  chatgptAccount: { models: core.models },
}));

const PLAN_MODELS: ChatGptModel[] = [
  { displayName: 'GPT-5', slug: 'gpt-5' },
  { displayName: 'GPT-5 mini', slug: 'gpt-5-mini' },
];

function storedProfile(hasCredential: boolean): ProviderProfile {
  return {
    access: 'endpoint',
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
    useTools: true,
  };
}

/** A profile on the ChatGPT plan as the core stores it: the plan's base URL and interface, and no credential. */
function storedPlanProfile(): ProviderProfile {
  return { ...storedProfile(false), access: 'chatgpt_plan', model: 'gpt-5-mini', name: 'ChatGPT', server: null };
}

const KNOWN_MODEL: ModelDefaults = { limits: { contextWindow: 128_000, outputLimit: 16_384 }, reasons: false };

beforeEach(() => {
  core.create.mockReset();
  core.getModelDefaults.mockReset();
  core.models.mockReset();
  core.update.mockReset();
  core.create.mockResolvedValue({ data: storedProfile(true), ok: true });
  core.update.mockResolvedValue({ data: storedProfile(true), ok: true });
  core.models.mockResolvedValue({ data: PLAN_MODELS, ok: true });
  // A model that Mannie does not know has no defaults.
  core.getModelDefaults.mockResolvedValue(null);
});

describe('the "Use tools" switch', () => {
  it('is on for a new profile and follows a stored one', () => {
    expect(new ProviderProfileForm(null).useTools).toBe(true);
    expect(new ProviderProfileForm({ ...storedProfile(true), useTools: false }).useTools).toBe(false);
  });

  it('reaches the core with the profile', async () => {
    const form = new ProviderProfileForm(storedProfile(true));
    form.useTools = false;

    expect(await form.save()).toBe(true);

    expect(core.update).toHaveBeenCalledWith(
      expect.objectContaining({
        draft: {
          access: 'endpoint',
          baseUrl: 'https://api.openai.com/v1',
          contextWindow: 400_000,
          interface: 'responses',
          model: 'gpt-5',
          name: 'OpenAI',
          outputLimit: 128_000,
          reasoningEffort: 'medium',
          useTools: false,
        },
      }),
    );
  });
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

describe('a profile on the ChatGPT plan', () => {
  it('reaches the core without an interface, base URL or credential of its own', async () => {
    const form = new ProviderProfileForm(null);
    form.name = 'ChatGPT';
    form.baseUrl = 'http://127.0.0.1:9/v1';
    form.api = 'chat_completions';
    form.credential = 'typed-before-switching';
    form.setAccess('chatgpt_plan');
    await form.chooseModel('gpt-5');
    form.contextWindow = 400_000;
    form.outputLimit = 128_000;

    expect(await form.save()).toBe(true);

    expect(core.create).toHaveBeenCalledWith({
      credential: null,
      draft: {
        access: 'chatgpt_plan',
        baseUrl: '',
        contextWindow: 400_000,
        interface: 'responses',
        model: 'gpt-5',
        name: 'ChatGPT',
        outputLimit: 128_000,
        reasoningEffort: 'server_default',
        useTools: true,
      },
    });
  });

  it('asks for one of the plan’s models before it reaches the core', async () => {
    const form = new ProviderProfileForm(null);
    form.setAccess('chatgpt_plan');
    form.contextWindow = 8_192;
    form.outputLimit = 1_024;

    expect(await form.save()).toBe(false);
    const missing = form.modelMissing;
    await form.chooseModel('gpt-5');

    expect(missing).toBe(true);
    expect(form.modelMissing).toBe(false);
    expect(core.create).not.toHaveBeenCalled();
  });

  it('keeps the model of each way to reach it while the owner switches between them', () => {
    const form = new ProviderProfileForm(storedProfile(true));
    form.setAccess('chatgpt_plan');
    const onPlan = form.model;
    form.model = 'gpt-5-mini';
    form.setAccess('endpoint');
    const backOnEndpoint = form.model;
    form.setAccess('chatgpt_plan');

    expect(onPlan).toBe('');
    expect(backOnEndpoint).toBe('gpt-5');
    expect(form.model).toBe('gpt-5-mini');
  });

  it('fills in the limits of a chosen model as for a typed one', async () => {
    core.getModelDefaults.mockResolvedValue(KNOWN_MODEL);
    const form = new ProviderProfileForm(null);
    form.setAccess('chatgpt_plan');

    await form.chooseModel('gpt-5-mini');

    expect(core.getModelDefaults).toHaveBeenCalledWith({ model: 'gpt-5-mini' });
    expect(form.contextWindow).toBe(128_000);
    expect(form.defaultsApplied).toBe(true);
  });

  it('keeps out the limits of a model that the owner left while the core answered', async () => {
    let answer: (defaults: ModelDefaults) => void = () => undefined;
    core.getModelDefaults.mockReturnValueOnce(
      new Promise<ModelDefaults>((resolve) => {
        answer = resolve;
      }),
    );
    core.getModelDefaults.mockResolvedValueOnce(null);
    const form = new ProviderProfileForm(null);
    form.setAccess('chatgpt_plan');

    const first = form.chooseModel('gpt-5');
    await form.chooseModel('gpt-5-mini');
    answer(KNOWN_MODEL);
    await first;

    expect(form.model).toBe('gpt-5-mini');
    expect(form.contextWindow).toBeNull();
    expect(form.defaultsApplied).toBe(false);
  });

  it('reads the plan’s catalog, and an older answer changes nothing', async () => {
    let answerFirst: (outcome: Outcome<ChatGptModel[], never>) => void = () => undefined;
    core.models.mockReturnValueOnce(
      new Promise<Outcome<ChatGptModel[], never>>((resolve) => {
        answerFirst = resolve;
      }),
    );
    const form = new ProviderProfileForm(null);

    const first = form.loadModels();
    const loading = form.catalog.status;
    await form.loadModels();
    answerFirst({ data: [], ok: true });
    await first;

    expect(loading).toBe('loading');
    expect(form.catalog).toEqual({ models: PLAN_MODELS, status: 'ready' });
  });

  it('says why the catalog could not be read', async () => {
    core.models.mockResolvedValue({ failure: { error: { code: 'signed_out' }, kind: 'core' }, ok: false });
    const form = new ProviderProfileForm(null);

    await form.loadModels();

    expect(form.catalog).toEqual({ failure: { error: { code: 'signed_out' }, kind: 'core' }, status: 'error' });
  });

  it('starts an edited profile from its model, and keeps no credential', async () => {
    const form = new ProviderProfileForm(storedPlanProfile());

    expect(await form.save()).toBe(true);

    expect(form.usesPlan).toBe(true);
    expect(form.model).toBe('gpt-5-mini');
    expect(form.forgetsStoredCredential).toBe(false);
    expect(core.update).toHaveBeenCalledWith({
      credential: { action: 'keep' },
      draft: {
        access: 'chatgpt_plan',
        baseUrl: '',
        contextWindow: 400_000,
        interface: 'responses',
        model: 'gpt-5-mini',
        name: 'ChatGPT',
        outputLimit: 128_000,
        reasoningEffort: 'medium',
        useTools: true,
      },
      id: '0199e0f2-0000-7000-8000-0000000000ff',
    });
  });

  it('says that moving a profile with a key to the plan forgets the key, and sends no new one', async () => {
    const form = new ProviderProfileForm(storedProfile(true));
    form.credential = 'typed-before-switching';
    const before = form.forgetsStoredCredential;
    form.setAccess('chatgpt_plan');
    await form.chooseModel('gpt-5');

    expect(await form.save()).toBe(true);

    expect(before).toBe(false);
    expect(form.forgetsStoredCredential).toBe(true);
    expect(core.update).toHaveBeenCalledWith(expect.objectContaining({ credential: { action: 'keep' } }));
  });
});
