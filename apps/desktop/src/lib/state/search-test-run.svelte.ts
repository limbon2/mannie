// One run of a search profile's test (ADR-0047). Each opening of the test
// dialog is a new run.
import type { SearchProfile, SearchTestReport } from '#lib/generated/bindings.js';
import { searchProfiles } from '#lib/state/search-profiles.svelte.js';
import type { ProfileFailure } from '#lib/types/chat.js';

/** The test of a search profile while it runs and after. */
export class SearchTestRun {
  /** Why the test could not run. */
  failure: ProfileFailure | null = $state(null);
  readonly profile: SearchProfile;
  /** The test's checks, once it ran. */
  report: SearchTestReport | null = $state(null);

  constructor(profile: SearchProfile) {
    this.profile = profile;
  }

  /** Runs the test. */
  async run(): Promise<void> {
    const outcome = await searchProfiles.test(this.profile.id);
    if (outcome.ok) {
      this.report = outcome.data;
    } else {
      this.failure = outcome.failure;
    }
  }
}
