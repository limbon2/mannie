// The tools settings, the command rules and the grants beyond one
// conversation, as the settings page shows them (docs/features/system-tools/
// tool-activity.md, settings). The core checks every change and returns what
// it stored.
import {
  type CommandRule,
  commands,
  type GrantId,
  type RuleId,
  type SharedGrant,
  type ToolsSettings,
  type UpdateToolsSettings,
} from '#lib/generated/bindings.js';
import type { ToolsFailure } from '#lib/types/tools.js';
import { call } from '#lib/utils/calls.js';

import { revokeSharedGrant } from './tools-actions.svelte.js';

/** The tools settings and the outcome of the latest request. */
export class ToolsSettingsState {
  /** Why the settings could not be read, if they could not. */
  readFailure: ToolsFailure | null = $state(null);
  /** Why the latest change failed, if it failed. */
  saveFailure: ToolsFailure | null = $state(null);
  /** Whether a change is being saved. */
  saving = $state(false);
  /** The settings, once read. */
  settings: ToolsSettings | null = $state(null);
  #loading: Promise<void> | null = null;

  /** Reads the settings from the core. */
  async load(): Promise<void> {
    this.readFailure = null;
    const outcome = await call(() => commands.getToolsSettings());
    if (outcome.ok) {
      this.settings = outcome.data;
    } else {
      this.readFailure = outcome.failure;
    }
  }

  /** Reads the settings unless they were read, or are being read. */
  async ensureLoaded(): Promise<void> {
    if (this.settings !== null) {
      return;
    }
    this.#loading ??= this.load().finally(() => {
      this.#loading = null;
    });
    await this.#loading;
  }

  /**
   * Changes some of the settings and keeps the others, and returns why the
   * core refused the change, if it did.
   */
  async update(change: Partial<UpdateToolsSettings>): Promise<ToolsFailure | null> {
    await this.ensureLoaded();
    const current = this.settings;
    if (current === null) {
      return this.readFailure;
    }
    const update: UpdateToolsSettings = {
      allowChangesInWorkspace: current.allowChangesInWorkspace,
      commandEnvironment: [...current.commandEnvironment],
      ...change,
    };
    this.saving = true;
    this.saveFailure = null;
    const outcome = await call(() => commands.updateToolsSettings(update));
    this.saving = false;
    if (!outcome.ok) {
      this.saveFailure = outcome.failure;
      return outcome.failure;
    }
    this.settings = outcome.data;
    return null;
  }
}

/** The command rules and the outcome of the latest request. */
export class CommandRulesState {
  /** Why the latest deletion failed, if it failed. */
  deleteFailure: ToolsFailure | null = $state(null);
  /** The rules being deleted. */
  deleting: RuleId[] = $state([]);
  /** Why the rules could not be read, if they could not. */
  readFailure: ToolsFailure | null = $state(null);
  /** The rules, the oldest first, once read. */
  rules: CommandRule[] = $state([]);
  status: 'error' | 'loading' | 'ready' = $state('loading');

  /** Reads the rules from the core. */
  async load(): Promise<void> {
    this.readFailure = null;
    if (this.rules.length === 0) {
      this.status = 'loading';
    }
    const outcome = await call(() => commands.listCommandRules());
    if (outcome.ok) {
      this.rules = outcome.data;
      this.status = 'ready';
    } else {
      this.readFailure = outcome.failure;
      this.status = 'error';
    }
  }

  /** Deletes a rule. A rule that no longer exists leaves the list as well. */
  async remove(ruleId: RuleId): Promise<void> {
    if (this.deleting.includes(ruleId)) {
      return;
    }
    this.deleting = [...this.deleting, ruleId];
    this.deleteFailure = null;
    const outcome = await call(() => commands.deleteCommandRule({ ruleId }));
    this.deleting = this.deleting.filter((deleting) => deleting !== ruleId);
    const isGone = !outcome.ok && outcome.failure.kind === 'core' && outcome.failure.error.code === 'rule_not_found';
    if (outcome.ok || isGone) {
      this.rules = this.rules.filter((rule) => rule.id !== ruleId);
      return;
    }
    this.deleteFailure = outcome.failure;
  }
}

/** The grants for a project or for every conversation, and the outcome of the latest request. */
export class SharedGrantsState {
  /** The grants, the oldest first, once read. */
  grants: SharedGrant[] = $state([]);
  /** Why the grants could not be read, if they could not. */
  readFailure: ToolsFailure | null = $state(null);
  /** Why the latest revocation failed, if it failed. */
  revokeFailure: ToolsFailure | null = $state(null);
  /** The grants being revoked. */
  revoking: GrantId[] = $state([]);
  status: 'error' | 'loading' | 'ready' = $state('loading');

  /** Reads the grants from the core. */
  async load(): Promise<void> {
    this.readFailure = null;
    if (this.grants.length === 0) {
      this.status = 'loading';
    }
    const outcome = await call(() => commands.listSharedGrants());
    if (outcome.ok) {
      this.grants = outcome.data;
      this.status = 'ready';
    } else {
      this.readFailure = outcome.failure;
      this.status = 'error';
    }
  }

  /** Revokes a grant. A grant that no longer exists leaves the list as well. */
  async revoke(grantId: GrantId): Promise<void> {
    if (this.revoking.includes(grantId)) {
      return;
    }
    this.revoking = [...this.revoking, grantId];
    this.revokeFailure = null;
    const failure = await revokeSharedGrant(grantId);
    this.revoking = this.revoking.filter((revoking) => revoking !== grantId);
    if (failure === null) {
      this.grants = this.grants.filter((grant) => grant.id !== grantId);
      return;
    }
    this.revokeFailure = failure;
  }
}

/** The window's tools settings. */
export const toolsSettings = new ToolsSettingsState();

/** The window's command rules. */
export const commandRules = new CommandRulesState();

/** The window's grants beyond one conversation. */
export const sharedGrants = new SharedGrantsState();
