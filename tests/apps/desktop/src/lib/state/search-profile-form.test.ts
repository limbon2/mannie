// The search profile form (ADR-0047): what it sends to the core for each
// choice of the owner about the kind, the prefixes and the credential.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { SearchProfile } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  DEFAULT_QUERY_PREFIX,
  SearchProfileForm,
} from '../../../../../../apps/desktop/src/lib/state/search-profile-form.svelte.js';

const core = vi.hoisted(() => ({
  create: vi.fn(),
  update: vi.fn(),
}));

vi.mock('../../../../../../apps/desktop/src/lib/state/search-profiles.svelte.js', () => ({
  searchProfiles: { create: core.create, update: core.update },
}));

function storedProfile(hasCredential: boolean): SearchProfile {
  return {
    baseUrl: 'http://gpu-box.local:8080/v1',
    createdAt: 0,
    documentPrefix: '',
    hasCredential,
    id: '0199e0f2-0000-7000-8000-0000000000aa',
    kind: 'embeddings',
    model: 'an-embedding-model',
    name: 'Embeddings',
    queryPrefix: 'Instruct: find passages\nQuery: ',
    server: null,
    updatedAt: 0,
  };
}

beforeEach(() => {
  core.create.mockReset();
  core.update.mockReset();
  core.create.mockResolvedValue({ data: storedProfile(false), ok: true });
  core.update.mockResolvedValue({ data: storedProfile(true), ok: true });
});

describe('a new search profile', () => {
  it('starts with the query instruction of instruction-aware models, which the owner may clear', async () => {
    const form = new SearchProfileForm(null);

    const prefilled = form.queryPrefix;
    form.queryPrefix = '';
    await form.save();

    expect(prefilled).toBe(DEFAULT_QUERY_PREFIX);
    expect(DEFAULT_QUERY_PREFIX).toBe(
      'Instruct: Given a question, retrieve documentation passages that answer it\nQuery: ',
    );
    expect(core.create).toHaveBeenCalledWith(
      expect.objectContaining({ draft: expect.objectContaining({ queryPrefix: '' }) as unknown }),
    );
  });

  it('is an embedding profile that sends its prefixes exactly and no credential when the field is empty', async () => {
    const form = new SearchProfileForm(null);
    form.name = 'Embeddings';
    form.baseUrl = 'http://127.0.0.1:8080/v1';
    form.model = 'an-embedding-model';
    form.queryPrefix = 'query: ';
    form.documentPrefix = 'passage: ';

    expect(form.kind).toBe('embeddings');
    expect(await form.save()).toBe(true);

    expect(core.create).toHaveBeenCalledWith({
      credential: null,
      draft: {
        baseUrl: 'http://127.0.0.1:8080/v1',
        documentPrefix: 'passage: ',
        kind: 'embeddings',
        model: 'an-embedding-model',
        name: 'Embeddings',
        queryPrefix: 'query: ',
      },
    });
  });

  it('sends no prefixes once it is a rerank profile, whatever the fields still hold', async () => {
    const form = new SearchProfileForm(null);
    form.queryPrefix = 'query: ';
    form.kind = 'rerank';

    await form.save();

    expect(form.takesPrefixes).toBe(false);
    expect(core.create).toHaveBeenCalledWith(
      expect.objectContaining({
        draft: expect.objectContaining({ documentPrefix: '', kind: 'rerank', queryPrefix: '' }) as unknown,
      }),
    );
  });
});

describe('an edited search profile', () => {
  it('starts from the stored fields and keeps the stored credential while the field stays empty', async () => {
    const form = new SearchProfileForm(storedProfile(true));

    await form.save();

    expect(form.queryPrefix).toBe('Instruct: find passages\nQuery: ');
    expect(core.update).toHaveBeenCalledWith(
      expect.objectContaining({
        credential: { action: 'keep' },
        id: '0199e0f2-0000-7000-8000-0000000000aa',
      }),
    );
  });

  it('replaces the stored credential with a typed one, and removes it only when asked', async () => {
    const replacing = new SearchProfileForm(storedProfile(true));
    replacing.credential = 'sk-new';
    const removing = new SearchProfileForm(storedProfile(true));
    removing.removeStoredCredential();

    await replacing.save();
    await removing.save();

    expect(core.update).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({ credential: { action: 'replace', credential: 'sk-new' } }),
    );
    expect(core.update).toHaveBeenNthCalledWith(2, expect.objectContaining({ credential: { action: 'remove' } }));
  });

  it('keeps the core’s refusal, and its code marks the fields', async () => {
    core.update.mockResolvedValue({
      failure: { error: { code: 'invalid_prefix', maxCharacters: 1000 }, kind: 'core' },
      ok: false,
    });
    const form = new SearchProfileForm(storedProfile(false));

    expect(await form.save()).toBe(false);

    expect(form.failureCode).toBe('invalid_prefix');
    expect(form.saving).toBe(false);
  });
});
