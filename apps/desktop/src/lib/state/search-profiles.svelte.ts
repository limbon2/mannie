// The search profiles as the UI knows them (ADR-0047): embedding and rerank
// models, each on a server that the owner names. The core checks every field
// and keeps the credentials in the credential store; the window sends a
// credential once and never gets it back.
import {
  commands,
  type CreateSearchProfile,
  type ProfileId,
  type ProviderProfileError,
  type SearchProfile,
  type SearchTestReport,
  type UpdateSearchProfile,
} from '#lib/generated/bindings.js';
import type { Failure, Outcome } from '#lib/types/calls.js';
import { call } from '#lib/utils/calls.js';

/** The window's search profiles. */
export class SearchProfiles {
  /** Why the profiles could not be read, if they could not. */
  failure: Failure<ProviderProfileError> | null = $state(null);
  profiles: SearchProfile[] = $state([]);
  status: 'error' | 'loading' | 'ready' = $state('loading');
  #loaded = false;

  /** Adds a profile and reads the list again. */
  async create(input: CreateSearchProfile): Promise<Outcome<SearchProfile, ProviderProfileError>> {
    const outcome = await call(() => commands.createSearchProfile(input));
    await this.reload();
    return outcome;
  }

  /** Deletes a profile and reads the list again. */
  async delete(id: ProfileId): Promise<Outcome<null, ProviderProfileError>> {
    const outcome = await call(() => commands.deleteSearchProfile({ id }));
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

  /** Reads the profiles from the core. */
  async reload(): Promise<void> {
    if (this.profiles.length === 0) {
      this.status = 'loading';
    }
    const outcome = await call(() => commands.listSearchProfiles());
    if (outcome.ok) {
      this.profiles = outcome.data;
      this.status = 'ready';
      this.failure = null;
    } else {
      this.failure = outcome.failure;
      this.status = 'error';
    }
  }

  /** Runs the test of a profile, which may also record its server. */
  async test(id: ProfileId): Promise<Outcome<SearchTestReport, ProviderProfileError>> {
    const outcome = await call(() => commands.testSearchProfile({ id }));
    await this.reload();
    return outcome;
  }

  /** Changes a profile and reads the list again. */
  async update(input: UpdateSearchProfile): Promise<Outcome<SearchProfile, ProviderProfileError>> {
    const outcome = await call(() => commands.updateSearchProfile(input));
    await this.reload();
    return outcome;
  }
}

/** The window's search profiles. */
export const searchProfiles = new SearchProfiles();
