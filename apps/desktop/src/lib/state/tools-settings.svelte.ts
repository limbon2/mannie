// The tools settings, the command rules and the grants beyond one
// conversation, as the settings page shows them (docs/features/system-tools/
// tool-activity.md, settings). The core checks every change and returns what
// it stored.
import {
  type CommandRule,
  commands,
  type GrantId,
  type ProgramRule,
  type RemoteCommandRule,
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
      fullAccessForNewConversations: current.fullAccessForNewConversations ?? false,
      wakeOnCommandEnd: current.wakeOnCommandEnd ?? true,
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

/**
 * The rules of commands on this computer and on remote hosts, and of runs of
 * installed programs, and the outcome of the latest request.
 */
export class CommandRulesState {
  /** Why the latest deletion failed, if it failed. */
  deleteFailure: ToolsFailure | null = $state(null);
  /** The rules being deleted. */
  deleting: RuleId[] = $state([]);
  /** The rules for runs of installed programs, each limited to its plugin's program, the oldest first, once read (ADR-0038). */
  programRules: ProgramRule[] = $state([]);
  /** Why the rules could not be read, if they could not. */
  readFailure: ToolsFailure | null = $state(null);
  /** The rules for commands on remote hosts, each limited to its host, the oldest first, once read (ADR-0036). */
  remoteRules: RemoteCommandRule[] = $state([]);
  /** The rules for commands on this computer, the oldest first, once read. */
  rules: CommandRule[] = $state([]);
  status: 'error' | 'loading' | 'ready' = $state('loading');

  /** Whether no rule of any kind is listed. */
  get isEmpty(): boolean {
    const hasNoLocalRules = this.rules.length === 0;
    const hasNoRemoteRules = this.remoteRules.length === 0;
    const hasNoProgramRules = this.programRules.length === 0;
    return hasNoLocalRules && hasNoRemoteRules && hasNoProgramRules;
  }

  /** Reads every kind of rule from the core, and keeps the last lists when one cannot be read. */
  async load(): Promise<void> {
    this.readFailure = null;
    if (this.isEmpty) {
      this.status = 'loading';
    }
    const [local, remote, program] = await Promise.all([
      call(() => commands.listCommandRules()),
      call(() => commands.listRemoteCommandRules()),
      call(() => commands.listProgramRules()),
    ]);
    if (!local.ok) {
      this.readFailure = local.failure;
      this.status = 'error';
      return;
    }
    if (!remote.ok) {
      this.readFailure = remote.failure;
      this.status = 'error';
      return;
    }
    if (!program.ok) {
      this.readFailure = program.failure;
      this.status = 'error';
      return;
    }
    this.rules = local.data;
    this.remoteRules = remote.data;
    this.programRules = program.data;
    this.status = 'ready';
  }

  /** Deletes a rule of an installed program. A rule that no longer exists leaves the list as well. */
  async removeProgram(ruleId: RuleId): Promise<void> {
    const deleted = await this.#delete(ruleId, () => commands.deleteProgramRule({ ruleId }));
    if (deleted) {
      this.programRules = this.programRules.filter((rule) => rule.id !== ruleId);
    }
  }

  /** Deletes a rule of this computer. A rule that no longer exists leaves the list as well. */
  async remove(ruleId: RuleId): Promise<void> {
    const deleted = await this.#delete(ruleId, () => commands.deleteCommandRule({ ruleId }));
    if (deleted) {
      this.rules = this.rules.filter((rule) => rule.id !== ruleId);
    }
  }

  /** Deletes a rule of a remote host. A rule that no longer exists leaves the list as well. */
  async removeRemote(ruleId: RuleId): Promise<void> {
    const deleted = await this.#delete(ruleId, () => commands.deleteRemoteCommandRule({ ruleId }));
    if (deleted) {
      this.remoteRules = this.remoteRules.filter((rule) => rule.id !== ruleId);
    }
  }

  /** Runs a deletion once at a time for each rule, and says whether the rule is gone. */
  async #delete(ruleId: RuleId, request: () => ReturnType<typeof commands.deleteCommandRule>): Promise<boolean> {
    if (this.deleting.includes(ruleId)) {
      return false;
    }
    this.deleting = [...this.deleting, ruleId];
    this.deleteFailure = null;
    const outcome = await call(request);
    this.deleting = this.deleting.filter((deleting) => deleting !== ruleId);
    const isGone = !outcome.ok && outcome.failure.kind === 'core' && outcome.failure.error.code === 'rule_not_found';
    if (outcome.ok || isGone) {
      return true;
    }
    this.deleteFailure = outcome.failure;
    return false;
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
