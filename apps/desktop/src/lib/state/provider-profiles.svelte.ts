// The provider profiles as the UI knows them (docs/features/chat/
// providers.md). The core checks every field and keeps the credentials in the
// credential store; the window sends a credential once and never gets it back.
import {
  commands,
  type CreateProviderProfile,
  type ProfileId,
  type ProviderProfile,
  type ProviderProfileError,
  type ProviderTestReport,
  type UpdateProviderProfile,
} from '#lib/generated/bindings.js';
import type { Failure, Outcome } from '#lib/types/calls.js';
import { call } from '#lib/utils/calls.js';

/** The window's provider profiles. */
export class ProviderProfiles {
  /** Why the profiles could not be read, if they could not. */
  failure: Failure<ProviderProfileError> | null = $state(null);
  profiles: ProviderProfile[] = $state([]);
  status: 'error' | 'loading' | 'ready' = $state('loading');
  #loaded = false;

  /** The default profile, if a profile exists. */
  get defaultProfile(): ProviderProfile | undefined {
    return this.profiles.find((profile) => profile.isDefault);
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
    if (this.profiles.length === 0) {
      this.status = 'loading';
    }
    const outcome = await call(() => commands.listProviderProfiles());
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
