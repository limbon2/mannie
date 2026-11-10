// Updates of the desktop app (docs/features/updates.md; ADR-0063): the status
// as the core reports it, first and after each change, the switch for
// automatic checks, and the owner's actions. The core checks, downloads,
// verifies and installs. The window names a version or a page and never an
// address, so a compromised window cannot make Mannie install or open
// anything else (ADR-0004).
import {
  commands,
  type ReleasePage,
  type SubscriptionId,
  type UpdateOutcome,
  type UpdatesSettings,
  type UpdateStatus,
  type UpdateStatusUpdate,
} from '#lib/generated/bindings.js';
import type { RestartQuestion, UpdateActionFailure, UpdatesFailure } from '#lib/types/updates.js';
import { batchChannel, call } from '#lib/utils/calls.js';

/** An action that waits for the core's answer. */
export type PendingUpdateAction = 'check' | 'restart';

/** The window's view of Mannie's updates and the owner's actions on them. */
export class Updates {
  /** The action of the owner that failed last, until the next action starts. */
  actionFailure: UpdateActionFailure | null = $state(null);
  /** Whether Mannie checks for updates on its own, once the core reported the setting. */
  checkAutomatically: boolean | null = $state(null);
  /** Why the status could not be followed, if it could not. */
  failure: UpdatesFailure | null = $state(null);
  /** The action that waits for the core, which keeps the buttons off until it answers. */
  pending: PendingUpdateAction | null = $state(null);
  phase: 'error' | 'loading' | 'ready' = $state('loading');
  /** What the restart would end, while the owner decides whether to restart. */
  question: RestartQuestion | null = $state.raw(null);
  /** Whether Mannie is quitting to install the version that the owner chose. */
  restarting = $state(false);
  /** Why the latest change of the setting was not saved, if it was not. */
  saveFailure: UpdatesFailure | null = $state(null);
  saving = $state(false);
  /** Why the setting could not be read, if it could not. */
  settingsFailure: UpdatesFailure | null = $state(null);
  /** The status, once the core reported it. */
  status: UpdateStatus | null = $state.raw(null);
  /** The version whose notice the owner closed. Another version shows its notice again. */
  #closedVersion: string | null = $state(null);
  /** Counts subscriptions, so that the updates of an ended one change nothing. */
  #generation = 0;
  /** Whether the owner closed the notice about the last install. */
  #outcomeClosed = $state(false);
  #readyVersion: string | null = $state(null);
  #started = false;
  #subscription: SubscriptionId | null = null;

  /** Whether a check can start: this build updates itself, and no check or download runs. */
  get canCheck(): boolean {
    const kind = this.status?.state.kind;
    const stateAllowsCheck = kind === 'idle' || kind === 'ready' || kind === 'failed';
    return stateAllowsCheck && this.#canAct;
  }

  /** Whether "Restart and update" can start: a version is ready, and no action waits for the core. */
  get canRestart(): boolean {
    const hasReadyVersion = this.#readyVersion !== null;
    return hasReadyVersion && this.#canAct;
  }

  /** The version whose notice shows: the ready one, unless the owner closed its notice. */
  get noticeVersion(): string | null {
    return this.#readyVersion === this.#closedVersion ? null : this.#readyVersion;
  }

  /** The result of the last install, until the owner closes its notice. */
  get outcome(): UpdateOutcome | null {
    if (this.#outcomeClosed) {
      return null;
    }
    return this.status?.lastOutcome ?? null;
  }

  /**
   * The version whose installer waits for "Restart and update". A check keeps
   * the installer until it finds another version or none, so the version
   * stays here while a check runs.
   */
  get readyVersion(): string | null {
    return this.#readyVersion;
  }

  /** Whether an action of the owner can start: none waits for the core, and Mannie is not quitting. */
  get #canAct(): boolean {
    const nothingPending = this.pending === null;
    return nothingPending && !this.restarting;
  }

  /** Keeps everything running: the owner answered the question with "Cancel". */
  cancelRestart(): void {
    this.question = null;
  }

  /** Starts a check. What it finds arrives with the status. */
  async check(): Promise<void> {
    if (!this.#canAct) {
      return;
    }
    this.actionFailure = null;
    this.pending = 'check';
    const outcome = await call(() => commands.checkForUpdates());
    this.pending = null;
    if (!outcome.ok) {
      this.actionFailure = { action: 'check', failure: outcome.failure };
    }
  }

  /** Closes the notice about the last install, here at once and in the core for later windows. */
  async closeOutcome(): Promise<void> {
    this.#outcomeClosed = true;
    try {
      await commands.dismissUpdateOutcome();
    } catch {
      // The notice stays closed in this window. A core that did not hear of
      // it shows the result again after the window loads again.
    }
  }

  /** Closes the notice about the ready version until Mannie starts again or another version is ready. */
  closeReadyNotice(): void {
    this.#closedVersion = this.#readyVersion;
  }

  /** Restarts to install the version of the question: the owner confirmed what the restart ends. */
  async confirmRestart(): Promise<void> {
    const question = this.question;
    if (question === null) {
      return;
    }
    await this.#restart(question.version, true);
  }

  /** Reads the setting for automatic checks. */
  async loadSettings(): Promise<void> {
    const outcome = await call(() => commands.getUpdatesSettings());
    if (outcome.ok) {
      this.#showSettings(outcome.data);
      this.settingsFailure = null;
      return;
    }
    this.settingsFailure = outcome.failure;
  }

  /** Opens a page of Mannie's releases in the system browser. The core builds its address. */
  async openReleasePage(page: ReleasePage): Promise<void> {
    this.actionFailure = null;
    const outcome = await call(() => commands.openReleasePage({ page }));
    if (!outcome.ok) {
      this.actionFailure = { action: 'page', failure: outcome.failure, page };
    }
  }

  /**
   * Restarts Mannie to install the ready version. When the restart would end
   * running work, the core changes nothing and the question opens first.
   */
  async restart(): Promise<void> {
    const version = this.#readyVersion;
    if (version === null) {
      return;
    }
    await this.#restart(version, false);
  }

  /** Follows the status again after a failure. */
  retry(): void {
    void this.#subscribe();
  }

  /** Turns automatic checks on or off. Turning them on makes the core check at once. */
  async setCheckAutomatically(checkAutomatically: boolean): Promise<void> {
    if (this.saving) {
      return;
    }
    this.saving = true;
    this.saveFailure = null;
    const outcome = await call(() => commands.updateUpdatesSettings({ checkAutomatically }));
    this.saving = false;
    if (outcome.ok) {
      this.#showSettings(outcome.data);
      return;
    }
    this.saveFailure = outcome.failure;
  }

  /** Starts following the status. Later calls change nothing. */
  start(): void {
    if (this.#started) {
      return;
    }
    this.#started = true;
    void this.#subscribe();
  }

  #apply(update: UpdateStatusUpdate): void {
    switch (update.type) {
      case 'ended': {
        // A status that fell behind follows again; at shutdown the window is
        // about to close.
        if (update.reason === 'lagged') {
          void this.#subscribe();
        }
        return;
      }
      case 'status': {
        this.#show(update.status);
      }
    }
  }

  async #restart(version: string, confirmed: boolean): Promise<void> {
    if (!this.#canAct) {
      return;
    }
    this.actionFailure = null;
    this.pending = 'restart';
    const outcome = await call(() => commands.restartToUpdate({ confirmed, version }));
    this.pending = null;
    if (!outcome.ok) {
      this.question = null;
      this.actionFailure = { action: 'restart', failure: outcome.failure };
      // The notice is where the owner reads why, also after closing it.
      this.#closedVersion = null;
      return;
    }
    const answer = outcome.data;
    if (answer.kind === 'confirm_first') {
      this.question = {
        activeRuns: answer.activeRuns,
        backgroundCommands: answer.backgroundCommands,
        terminals: answer.terminals,
        version,
      };
      return;
    }
    this.question = null;
    this.restarting = true;
  }

  #show(status: UpdateStatus): void {
    const previous = this.#readyVersion;
    const state = status.state;
    if (state.kind === 'ready') {
      this.#readyVersion = state.version;
    } else if (state.kind !== 'checking') {
      this.#readyVersion = null;
    }
    if (this.#readyVersion !== previous) {
      // A question and a failed restart were about the version before.
      this.question = null;
      if (this.actionFailure?.action === 'restart') {
        this.actionFailure = null;
      }
    }
    this.status = status;
    this.phase = 'ready';
    this.failure = null;
  }

  #showSettings(settings: UpdatesSettings): void {
    // A section that was stored without the field checks automatically (D55).
    this.checkAutomatically = settings.checkAutomatically ?? true;
  }

  async #subscribe(): Promise<void> {
    this.#generation += 1;
    const generation = this.#generation;
    const previous = this.#subscription;
    this.#subscription = null;
    if (previous !== null) {
      void commands.unsubscribe({ subscriptionId: previous });
    }
    if (this.status === null) {
      this.phase = 'loading';
    }
    const channel = batchChannel<UpdateStatusUpdate>(
      (updates) => {
        if (generation !== this.#generation) {
          return;
        }
        for (const update of updates) {
          this.#apply(update);
        }
      },
      () => {
        // Lost batches: subscribing again gets the status as it is now.
        if (generation === this.#generation) {
          void this.#subscribe();
        }
      },
    );
    const outcome = await call(() => commands.subscribeUpdates(channel));
    if (generation !== this.#generation) {
      // A newer subscription replaced this one while it started.
      if (outcome.ok) {
        void commands.unsubscribe({ subscriptionId: outcome.data });
      }
      return;
    }
    if (outcome.ok) {
      this.#subscription = outcome.data;
      return;
    }
    this.failure = outcome.failure;
    this.phase = 'error';
  }
}

/** The window's updates. */
export const updates = new Updates();
