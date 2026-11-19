// The provider profiles as the UI knows them (docs/features/chat/
// providers.md). The core checks every field and keeps the credentials in the
// credential store; the window sends a credential once and never gets it back.
import {
  type ChatGptAccountView,
  commands,
  type CreateProviderProfile,
  type ProfileId,
  type ProviderProfile,
  type ProviderProfileError,
  type ProviderTestReport,
  type UpdateProviderProfile,
} from '#lib/generated/bindings.js';
import type { Failure, Outcome } from '#lib/types/calls.js';
import type { ChatGptFailure } from '#lib/types/chat.js';
import { call } from '#lib/utils/calls.js';

/** The window's provider profiles. */
export class ProviderProfiles {
  catalogFailure: ChatGptFailure | null = $state(null);
  catalogStatus: 'error' | 'idle' | 'loading' | 'ready' | 'signed_out' = $state('idle');
  /** Why the profiles could not be read, if they could not. */
  failure: Failure<ProviderProfileError> | null = $state(null);
  profiles: ProviderProfile[] = $state([]);
  status: 'error' | 'loading' | 'ready' = $state('loading');
  #loaded = false;
  #refresh = 0;
  #read = 0;

  /** The profiles the owner can edit, including custom profiles for plan models. */
  get configured(): ProviderProfile[] {
    return this.profiles.filter((profile) => !profile.catalog);
  }

  /** The signed-in account's listed models, in catalog order. */
  get planModels(): ProviderProfile[] {
    return this.profiles.filter((profile) => profile.catalog?.available);
  }

  /** The owner's default, or the first available plan model. */
  get defaultProfile(): ProviderProfile | undefined {
    return this.profiles.find((profile) => profile.isDefault) ?? this.planModels[0];
  }

  /** A new session invalidates any in-flight read of the previous account. */
  accountChanged(account: ChatGptAccountView): void {
    this.profiles = this.profiles.map((profile) =>
      profile.catalog ? { ...profile, catalog: { ...profile.catalog, available: false } } : profile,
    );
    this.catalogFailure = null;
    // The core invalidates models on a session change. Read its current rows while
    // refreshing so cached models from the same session can be used immediately.
    void this.reload();
    if (account.state === 'signed_in') {
      void this.refreshCatalog();
      return;
    }

    this.#refresh += 1;
    this.catalogStatus = 'signed_out';
  }

  /** Reconciles the signed-in account's models and reloads even after a transient failure
   * so the core can still offer its last successful catalog for the same account. */
  async refreshCatalog(): Promise<void> {
    this.#refresh += 1;
    const refresh = this.#refresh;
    this.catalogStatus = 'loading';
    this.catalogFailure = null;
    const result = await call(() => commands.refreshChatgptCatalog());
    if (refresh !== this.#refresh) {
      return;
    }

    await this.reload();
    if (refresh === this.#refresh) {
      this.catalogStatus = result.ok ? 'ready' : 'error';
      this.catalogFailure = result.ok ? null : result.failure;
    }
  }

  /** Adds a profile and reads the list again. */
  async create(input: CreateProviderProfile): Promise<Outcome<ProviderProfile, ProviderProfileError>> {
    const outcome = await call(() => commands.createProviderProfile(input));
    await this.reload();
    return outcome;
  }

  /** Deletes a profile and reads the list again. */
  async delete(id: ProfileId): Promise<Outcome<null, ProviderProfileError>> {
    const outcome = await call(() => commands.deleteProviderProfile({ id }));
    await this.reload();
    return outcome;
  }

  /** Reads the profiles the first time something needs them. */
  async ensureLoaded(): Promise<void> {
    if (this.#loaded) {
      return;
    }
    this.#loaded = true;
    await this.reload();
  }

  /** The profile with this ID, if it exists. */
  find(id: ProfileId | null): ProviderProfile | undefined {
    return id === null ? undefined : this.profiles.find((profile) => profile.id === id);
  }

  /** Reads the profiles from the core. */
  async reload(): Promise<void> {
    this.#read += 1;
    const read = this.#read;
    if (this.profiles.length === 0) {
      this.status = 'loading';
    }
    const outcome = await call(() => commands.listProviderProfiles());
    if (read !== this.#read) {
      return;
    }
    if (outcome.ok) {
      this.profiles = outcome.data;
      this.status = 'ready';
      this.failure = null;
    } else {
      this.failure = outcome.failure;
      this.status = 'error';
    }
  }

  /** Makes a profile the default and reads the list again. */
  async setDefault(id: ProfileId): Promise<Outcome<null, ProviderProfileError>> {
    const outcome = await call(() => commands.setDefaultProviderProfile({ id }));
    await this.reload();
    return outcome;
  }

  /** Runs the provider test of a profile, which may also record its server. */
  async test(id: ProfileId): Promise<Outcome<ProviderTestReport, ProviderProfileError>> {
    const outcome = await call(() => commands.testProviderProfile({ id }));
    await this.reload();
    return outcome;
  }

  /** Changes a profile and reads the list again. */
  async update(input: UpdateProviderProfile): Promise<Outcome<ProviderProfile, ProviderProfileError>> {
    const outcome = await call(() => commands.updateProviderProfile(input));
    await this.reload();
    return outcome;
  }
}

/** The window's provider profiles. */
export const providerProfiles = new ProviderProfiles();
