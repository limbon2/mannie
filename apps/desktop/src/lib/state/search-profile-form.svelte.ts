// The search profile dialog's form for one opening (ADR-0047). It starts
// from the profile as stored and saves through the core, which checks every
// field. The credential field never holds a stored credential: left empty,
// it keeps the stored one, and a value typed into it replaces it. A rerank
// profile takes no prefixes, so changing the kind to rerank sends none.
import type { CredentialChange, SearchKind, SearchProfile, SearchProfileDraft } from '#lib/generated/bindings.js';
import { searchProfiles } from '#lib/state/search-profiles.svelte.js';
import type { ProfileFailure } from '#lib/types/chat.js';

/**
 * The text before each query of a new embedding profile, which the owner
 * may clear (ADR-0049, context). Instruction-aware models, such as
 * Qwen3-Embedding, find better passages with an instruction, and the earlier
 * harness filled this one in. It is the model's input, in the language of
 * its training, and never shown as the interface's text.
 */
export const DEFAULT_QUERY_PREFIX =
  'Instruct: Given a question, retrieve documentation passages that answer it\nQuery: ';

/** The fields of a search profile while the owner adds or edits it. */
export class SearchProfileForm {
  baseUrl = $state('');
  /** A new credential, sent once when the profile is saved. */
  credential = $state('');
  documentPrefix = $state('');
  /** Why the core refused the latest save. */
  failure: ProfileFailure | null = $state(null);
  kind: SearchKind = $state('embeddings');
  model = $state('');
  name = $state('');
  /** The profile being edited, or `null` while one is added. */
  readonly profile: SearchProfile | null;
  queryPrefix = $state('');
  /** Whether saving removes the stored credential. */
  removesCredential = $state(false);
  saving = $state(false);

  constructor(profile: SearchProfile | null) {
    this.profile = profile;
    if (profile === null) {
      this.queryPrefix = DEFAULT_QUERY_PREFIX;
      return;
    }
    this.baseUrl = profile.baseUrl;
    this.documentPrefix = profile.documentPrefix;
    this.kind = profile.kind;
    this.model = profile.model;
    this.name = profile.name;
    this.queryPrefix = profile.queryPrefix;
  }

  /** The code of the latest save's failure, which marks the fields it concerns. */
  get failureCode(): string | null {
    return this.failure?.kind === 'core' ? this.failure.error.code : null;
  }

  /** Whether the profile being edited has a stored credential. */
  get hasStoredCredential(): boolean {
    return this.profile?.hasCredential ?? false;
  }

  /** Whether the kind takes prefixes: only an embedding model reads them. */
  get takesPrefixes(): boolean {
    return this.kind === 'embeddings';
  }

  /**
   * What saving does with the credential: an empty field keeps the stored
   * one, or leaves a profile without a credential as it is.
   */
  credentialChange(): CredentialChange {
    if (this.removesCredential) {
      return { action: 'remove' };
    }
    if (this.credential === '') {
      return { action: 'keep' };
    }
    return { action: 'replace', credential: this.credential };
  }

  /** The fields as the core takes them. */
  draft(): SearchProfileDraft {
    const prefixes = this.takesPrefixes;
    return {
      baseUrl: this.baseUrl,
      documentPrefix: prefixes ? this.documentPrefix : '',
      kind: this.kind,
      model: this.model,
      name: this.name,
      queryPrefix: prefixes ? this.queryPrefix : '',
    };
  }

  /** Keeps the stored credential after all, which undoes `removeStoredCredential`. */
  keepStoredCredential(): void {
    this.removesCredential = false;
  }

  /** Marks the stored credential for removal when the profile is saved. */
  removeStoredCredential(): void {
    if (!this.hasStoredCredential) {
      return;
    }
    this.credential = '';
    this.removesCredential = true;
  }

  /** Saves the profile, and returns whether the core accepted it. */
  async save(): Promise<boolean> {
    const draft = this.draft();
    this.saving = true;
    const outcome =
      this.profile === null
        ? await searchProfiles.create({ credential: this.credential === '' ? null : this.credential, draft })
        : await searchProfiles.update({ credential: this.credentialChange(), draft, id: this.profile.id });
    this.saving = false;
    if (!outcome.ok) {
      this.failure = outcome.failure;
      return false;
    }
    this.credential = '';
    this.failure = null;
    return true;
  }
}
