// The provider profile dialog's form for one opening (docs/features/chat/
// conversation-view.md, settings sections). It starts from the profile as
// stored, fills in the limits of a model that Mannie knows unless the owner
// set them, and saves through the core, which checks every field. The
// credential field never holds a stored credential: left empty, it keeps the
// stored one, and a value typed into it replaces it.
import {
  type ApiInterface,
  commands,
  type CredentialChange,
  type ModelDefaults,
  type ProfileDraft,
  type ProviderProfile,
  type ReasoningEffort,
} from '#lib/generated/bindings.js';
import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
import type { ProfileFailure } from '#lib/types/chat.js';

/** The fields of a provider profile while the owner adds or edits it. */
export class ProviderProfileForm {
  api: ApiInterface = $state('responses');
  baseUrl = $state('');
  contextWindow: number | null = $state(null);
  /** A new credential, sent once when the profile is saved. */
  credential = $state('');
  /** Whether the limits came from a model that Mannie knows. */
  defaultsApplied = $state(false);
  effort: ReasoningEffort = $state('server_default');
  /** Why the core refused the latest save. */
  failure: ProfileFailure | null = $state(null);
  /** Whether the owner set the limits, which model defaults then leave alone. */
  limitsTouched = $state(false);
  /** Whether the latest save stopped because a limit was empty. */
  limitsMissing = $state(false);
  model = $state('');
  name = $state('');
  outputLimit: number | null = $state(null);
  /** The profile being edited, or `null` while one is added. */
  readonly profile: ProviderProfile | null;
  /** Whether saving removes the stored credential. */
  removesCredential = $state(false);
  saving = $state(false);
  /** Whether the model gets tools. A new profile uses them. */
  useTools = $state(true);

  constructor(profile: ProviderProfile | null) {
    this.profile = profile;
    if (profile === null) {
      return;
    }
    this.api = profile.interface;
    this.baseUrl = profile.baseUrl;
    this.contextWindow = profile.contextWindow;
    this.effort = profile.reasoningEffort;
    this.limitsTouched = true;
    this.model = profile.model;
    this.name = profile.name;
    this.outputLimit = profile.outputLimit;
    this.useTools = profile.useTools;
  }

  /** The code of the latest save's failure, which marks the fields it concerns. */
  get failureCode(): string | null {
    return this.failure?.kind === 'core' ? this.failure.error.code : null;
  }

  /** Whether the profile being edited has a stored credential. */
  get hasStoredCredential(): boolean {
    return this.profile?.hasCredential ?? false;
  }

  /** Fills in the limits of a model that Mannie knows, unless the owner set them. */
  async applyModelDefaults(): Promise<void> {
    const model = this.model.trim();
    const canApply = !this.limitsTouched && model !== '';
    if (!canApply) {
      return;
    }
    let defaults: ModelDefaults | null;
    try {
      defaults = await commands.getModelDefaults({ model });
    } catch {
      // Without defaults the owner enters the limits.
      return;
    }
    if (defaults === null) {
      return;
    }
    // The owner may have set the limits while the core answered.
    if (this.#ownerSetLimits()) {
      return;
    }
    this.contextWindow = defaults.limits.contextWindow;
    this.outputLimit = defaults.limits.outputLimit;
    this.defaultsApplied = true;
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
    const contextWindow = this.contextWindow;
    const outputLimit = this.outputLimit;
    this.limitsMissing = contextWindow === null || outputLimit === null;
    if (contextWindow === null) {
      return false;
    }
    if (outputLimit === null) {
      return false;
    }
    const draft: ProfileDraft = {
      baseUrl: this.baseUrl,
      contextWindow,
      interface: this.api,
      model: this.model,
      name: this.name,
      outputLimit,
      reasoningEffort: this.effort,
      useTools: this.useTools,
    };
    this.saving = true;
    const outcome =
      this.profile === null
        ? await providerProfiles.create({ credential: this.credential === '' ? null : this.credential, draft })
        : await providerProfiles.update({ credential: this.credentialChange(), draft, id: this.profile.id });
    this.saving = false;
    if (!outcome.ok) {
      this.failure = outcome.failure;
      return false;
    }
    this.credential = '';
    this.failure = null;
    return true;
  }

  /** Read through a call, because the owner can change the limits while a request waits. */
  #ownerSetLimits(): boolean {
    return this.limitsTouched;
  }
}
