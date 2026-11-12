// Types that the state, the helpers and the components of the updates feature
// share (docs/features/updates.md).
import type { ReleasePage, UpdatesError } from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';

/** Why an updates command failed. */
export type UpdatesFailure = Failure<UpdatesError>;

/** An action of the owner on updates that can fail. */
export type UpdateAction = 'check' | 'page' | 'restart';

/** An action of the owner that failed, and why. */
export type UpdateActionFailure =
  | { readonly action: 'check' | 'restart'; readonly failure: UpdatesFailure }
  | {
      readonly action: 'page';
      readonly failure: UpdatesFailure;
      /** The page that did not open, which tells where the owner asked for it. */
      readonly page: ReleasePage;
    };

/** What a restart would end, as the core counts it. */
export interface RestartCounts {
  readonly activeRuns: number;
  readonly backgroundCommands: number;
  readonly terminals: number;
}

/** The question before a restart that would end running work. */
export interface RestartQuestion extends RestartCounts {
  /** The version that the owner chose to install. */
  readonly version: string;
}
