// One run of a profile's provider test (docs/features/chat/providers.md, the
// provider test), and the owner's choice to use the limits the server
// reports. Each opening of the test dialog is a new run.
import type { ProviderProfile, ProviderTestReport } from '#lib/generated/bindings.js';
import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
import type { ProfileFailure } from '#lib/types/chat.js';

/** A provider test while it runs and after. */
export class ProviderTestRun {
  /** Whether the profile uses the reported limits now. */
  applied = $state(false);
  /** Why the reported limits could not be stored. */
  applyFailure: ProfileFailure | null = $state(null);
  /** Why the test could not run. */
  failure: ProfileFailure | null = $state(null);
  readonly profile: ProviderProfile;
  /** The test's checks, once it ran. */
  report: ProviderTestReport | null = $state(null);

  constructor(profile: ProviderProfile) {
    this.profile = profile;
  }

  /** Stores the limits the server reports in the profile. */
  async applyLimits(): Promise<void> {
    const limits = this.report?.reportedLimits ?? null;
    if (limits === null) {
      return;
    }
    const profile = this.profile;
    const outcome = await providerProfiles.update({
      credential: { action: 'keep' },
      draft: {
        baseUrl: profile.baseUrl,
        contextWindow: limits.contextWindow,
        interface: profile.interface,
        model: profile.model,
        name: profile.name,
        outputLimit: limits.outputLimit ?? profile.outputLimit,
        reasoningEffort: profile.reasoningEffort,
        useTools: profile.useTools,
      },
      id: profile.id,
    });
    this.applied = outcome.ok;
    this.applyFailure = outcome.ok ? null : outcome.failure;
  }

  /** Runs the test. */
  async run(): Promise<void> {
    const outcome = await providerProfiles.test(this.profile.id);
    if (outcome.ok) {
      this.report = outcome.data;
    } else {
      this.failure = outcome.failure;
    }
  }
}
