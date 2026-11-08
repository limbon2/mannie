// The provider profile dialog's form for one opening (docs/features/chat/
// conversation-view.md, settings sections). It starts from the profile as
// stored, fills in the limits of a model that Mannie knows unless the owner
// set them, and saves through the core, which checks every field. The
// credential field never holds a stored credential: left empty, it keeps the
// stored one, and a value typed into it replaces it. A profile on the ChatGPT
// plan has no interface, base URL or credential of its own: the core sets the
// first two, and its model comes from the plan's catalog (docs/features/
// chatgpt-plan.md; ADR-0062).
import {
  type ApiInterface,
  type ChatGptModel,
  commands,
  type CredentialChange,
  type ModelDefaults,
  type ProfileAccess,
  type ProfileDraft,
  type ProviderProfile,
  type ReasoningEffort,
} from '#lib/generated/bindings.js';
import { chatgptAccount } from '#lib/state/chatgpt-account.svelte.js';
import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
import type { ChatGptFailure, ProfileFailure } from '#lib/types/chat.js';

/** The models that the ChatGPT plan offers the signed-in account, as the form reads them. */
export type PlanCatalog =
  | { readonly failure: ChatGptFailure; readonly status: 'error' }
  | { readonly models: readonly ChatGptModel[]; readonly status: 'ready' }
  | { readonly status: 'idle' }
  | { readonly status: 'loading' };

/** The fields of a provider profile while the owner adds or edits it. */
export class ProviderProfileForm {
  /** How the profile reaches its model. A new profile uses an API endpoint. */
  access: ProfileAccess = $state('endpoint');
  api: ApiInterface = $state('responses');
  baseUrl = $state('');
  /** The ChatGPT plan's models, which a profile on the plan chooses from. */
  catalog: PlanCatalog = $state({ status: 'idle' });
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
  /** Whether the latest save stopped because no model of the ChatGPT plan was chosen. */
  modelMissing = $state(false);
  name = $state('');
  outputLimit: number | null = $state(null);
  /** The profile being edited, or `null` while one is added. */
  readonly profile: ProviderProfile | null;
  /** Whether saving removes the stored credential. */
  removesCredential = $state(false);
  saving = $state(false);
  /** Whether the model gets tools. A new profile uses them. */
  useTools = $state(true);
  /** Counts the reads of the catalog, so that the answer to an older one changes nothing. */
  #catalogReads = 0;
  /** The model of each way to reach it, kept while the owner switches between them. */
  #modelsByAccess: Record<ProfileAccess, string> = { chatgpt_plan: '', endpoint: '' };

  constructor(profile: ProviderProfile | null) {
    this.profile = profile;
    if (profile === null) {
      return;
    }
    this.access = profile.access;
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

  /**
   * Whether saving forgets the stored credential, because the profile moves
   * to the ChatGPT plan, which uses the account's tokens instead.
   */
  get forgetsStoredCredential(): boolean {
    return this.usesPlan && this.hasStoredCredential;
  }

  /** Whether the profile being edited has a stored credential. */
  get hasStoredCredential(): boolean {
    return this.profile?.hasCredential ?? false;
  }

  /** Whether the profile reaches its model through the ChatGPT plan. */
  get usesPlan(): boolean {
    return this.access === 'chatgpt_plan';
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
    // The owner may have set the limits, or chosen another model of the
    // ChatGPT plan, while the core answered.
    if (this.#ownerSetLimits()) {
      return;
    }
    if (this.model.trim() !== model) {
      return;
    }
    this.contextWindow = defaults.limits.contextWindow;
    this.outputLimit = defaults.limits.outputLimit;
    this.defaultsApplied = true;
  }

  /** Chooses a model of the ChatGPT plan's catalog, and fills in its limits as for a typed model. */
  async chooseModel(slug: string): Promise<void> {
    this.model = slug;
    this.modelMissing = false;
    await this.applyModelDefaults();
  }

  /**
   * What saving does with the credential: an empty field keeps the stored
   * one, or leaves a profile without a credential as it is. A profile on the
   * ChatGPT plan sends none, and the core forgets a stored one.
   */
  credentialChange(): CredentialChange {
    if (this.usesPlan) {
      return { action: 'keep' };
    }
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

  /** Reads the models that the ChatGPT plan offers the signed-in account. */
  async loadModels(): Promise<void> {
    this.#catalogReads += 1;
    const read = this.#catalogReads;
    this.catalog = { status: 'loading' };
    const outcome = await chatgptAccount.models();
    if (read !== this.#catalogReads) {
      return;
    }
    this.catalog = outcome.ok
      ? { models: outcome.data, status: 'ready' }
      : { failure: outcome.failure, status: 'error' };
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
    this.modelMissing = this.usesPlan && this.model === '';
    if (contextWindow === null) {
      return false;
    }
    if (outputLimit === null) {
      return false;
    }
    if (this.modelMissing) {
      return false;
    }
    const draft: ProfileDraft = {
      access: this.access,
      // The core sets the interface and the base URL of a profile on the
      // ChatGPT plan, and never takes them from the window (ADR-0062).
      baseUrl: this.usesPlan ? '' : this.baseUrl,
      contextWindow,
      interface: this.usesPlan ? 'responses' : this.api,
      model: this.model,
      name: this.name,
      outputLimit,
      reasoningEffort: this.effort,
      useTools: this.useTools,
    };
    this.saving = true;
    const outcome =
      this.profile === null
        ? await providerProfiles.create({ credential: this.#newCredential(), draft })
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

  /**
   * Switches how the profile reaches its model. Each way keeps the model it
   * had, so that switching back brings it back: a typed model name on an
   * endpoint, and a model of the catalog on the ChatGPT plan.
   */
  setAccess(access: ProfileAccess): void {
    if (access === this.access) {
      return;
    }
    this.#modelsByAccess[this.access] = this.model;
    this.access = access;
    this.model = this.#modelsByAccess[access];
    this.defaultsApplied = false;
    this.modelMissing = false;
  }

  /** The credential of a new profile: the one typed into the field, never one for the ChatGPT plan. */
  #newCredential(): string | null {
    if (this.usesPlan) {
      return null;
    }
    return this.credential === '' ? null : this.credential;
  }

  /** Read through a call, because the owner can change the limits while a request waits. */
  #ownerSetLimits(): boolean {
    return this.limitsTouched;
  }
}
