// The tools settings, the command rules and the grants beyond one
// conversation as the settings page uses them (docs/features/system-tools/
// tool-activity.md, settings): a change sends the whole section, and a rule
// or grant that is already gone leaves the list.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  CommandRule,
  SharedGrant,
  ToolsError,
  ToolsSettings,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  CommandRulesState,
  SharedGrantsState,
  ToolsSettingsState,
} from '../../../../../../apps/desktop/src/lib/state/tools-settings.svelte.js';

const core = vi.hoisted(() => ({
  deleteCommandRule: vi.fn(),
  getToolsSettings: vi.fn(),
  listCommandRules: vi.fn(),
  listSharedGrants: vi.fn(),
  revokeSharedGrant: vi.fn(),
  updateToolsSettings: vi.fn(),
}));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));

const STORED: ToolsSettings = {
  allowChangesInWorkspace: false,
  commandEnvironment: ['JAVA_HOME'],
};

const RULE: CommandRule = {
  createdAt: 0,
  folder: null,
  id: '0199e0f2-0000-7000-8000-0000000000bb',
  words: ['cargo', 'test'],
};

function ok<T>(data: T): { data: T; status: 'ok' } {
  return { data, status: 'ok' };
}

function refused(error: ToolsError): { error: ToolsError & { description: string }; status: 'error' } {
  return { error: { ...error, description: 'refused' }, status: 'error' };
}

beforeEach(() => {
  for (const mock of Object.values(core)) {
    mock.mockReset();
  }
});

describe('ToolsSettingsState', () => {
  it('sends the whole section with the one change, and keeps what the core stored', async () => {
    const settings = new ToolsSettingsState();
    core.getToolsSettings.mockResolvedValue(ok(STORED));
    core.updateToolsSettings.mockResolvedValue(ok({ ...STORED, allowChangesInWorkspace: true }));

    const failure = await settings.update({ allowChangesInWorkspace: true });

    expect(failure).toBeNull();
    expect(core.updateToolsSettings).toHaveBeenCalledWith({
      allowChangesInWorkspace: true,
      commandEnvironment: ['JAVA_HOME'],
      fullAccessForNewConversations: false,
    });
    expect(settings.settings?.allowChangesInWorkspace).toBe(true);
  });

  it('reads the settings once for callers that wait together', async () => {
    const settings = new ToolsSettingsState();
    core.getToolsSettings.mockResolvedValue(ok(STORED));

    await Promise.all([settings.ensureLoaded(), settings.ensureLoaded()]);
    await settings.ensureLoaded();

    expect(core.getToolsSettings).toHaveBeenCalledTimes(1);
  });

  it('returns and keeps a refused change, and leaves the stored settings', async () => {
    const settings = new ToolsSettingsState();
    core.getToolsSettings.mockResolvedValue(ok(STORED));
    core.updateToolsSettings.mockResolvedValue(refused({ code: 'environment_name_secret', name: 'API_KEY' }));

    const failure = await settings.update({ commandEnvironment: ['API_KEY'] });

    expect(failure).toEqual({
      error: { code: 'environment_name_secret', description: 'refused', name: 'API_KEY' },
      kind: 'core',
    });
    expect(settings.saveFailure).toBe(failure);
    expect(settings.settings).toEqual(STORED);
  });

  it('changes nothing when the settings cannot be read', async () => {
    const settings = new ToolsSettingsState();
    core.getToolsSettings.mockResolvedValue(refused({ code: 'store_unavailable' }));

    const failure = await settings.update({ allowChangesInWorkspace: true });

    expect(failure?.kind).toBe('core');
    expect(core.updateToolsSettings).not.toHaveBeenCalled();
  });
});

describe('CommandRulesState', () => {
  it('lists the rules and deletes one', async () => {
    const rules = new CommandRulesState();
    core.listCommandRules.mockResolvedValue(ok([RULE]));
    core.deleteCommandRule.mockResolvedValue(ok(null));

    await rules.load();
    expect(rules.status).toBe('ready');
    await rules.remove(RULE.id);

    expect(core.deleteCommandRule).toHaveBeenCalledWith({ ruleId: RULE.id });
    expect(rules.rules).toEqual([]);
  });

  it('drops a rule that is already gone, and keeps one whose deletion failed', async () => {
    const rules = new CommandRulesState();
    core.listCommandRules.mockResolvedValue(ok([RULE, { ...RULE, id: 'kept' }]));
    await rules.load();

    core.deleteCommandRule.mockResolvedValueOnce(refused({ code: 'rule_not_found' }));
    await rules.remove(RULE.id);
    core.deleteCommandRule.mockResolvedValueOnce(refused({ code: 'storage_failed' }));
    await rules.remove('kept');

    expect(rules.rules.map((rule) => rule.id)).toEqual(['kept']);
    expect(rules.deleteFailure?.kind).toBe('core');
  });

  it('reports rules that could not be read', async () => {
    const rules = new CommandRulesState();
    core.listCommandRules.mockResolvedValue(refused({ code: 'storage_failed' }));

    await rules.load();

    expect(rules.status).toBe('error');
    expect(rules.readFailure?.kind).toBe('core');
  });
});

describe('SharedGrantsState', () => {
  const GRANT: SharedGrant = {
    createdAt: 0,
    id: '0199e0f2-0000-7000-8000-0000000000cc',
    projectId: null,
    scope: { command: 'npm test', kind: 'command', workingDirectory: 'D:\\Projects\\app' },
  };

  it('revokes a grant, and drops one that no longer exists', async () => {
    const grants = new SharedGrantsState();
    core.listSharedGrants.mockResolvedValue(ok([GRANT, { ...GRANT, id: 'other' }]));
    await grants.load();

    core.revokeSharedGrant.mockResolvedValueOnce(ok(null));
    await grants.revoke(GRANT.id);
    core.revokeSharedGrant.mockResolvedValueOnce(refused({ code: 'grant_not_found' }));
    await grants.revoke('other');

    expect(core.revokeSharedGrant).toHaveBeenCalledWith({ grantId: GRANT.id });
    expect(grants.grants).toEqual([]);
    expect(grants.revokeFailure).toBeNull();
  });

  it('keeps a grant whose revocation failed, and reports a list that could not be read', async () => {
    const grants = new SharedGrantsState();
    core.listSharedGrants.mockResolvedValue(ok([GRANT]));
    await grants.load();
    core.revokeSharedGrant.mockResolvedValueOnce(refused({ code: 'storage_failed' }));
    await grants.revoke(GRANT.id);

    expect(grants.grants).toEqual([GRANT]);
    expect(grants.revokeFailure?.kind).toBe('core');

    core.listSharedGrants.mockResolvedValue(refused({ code: 'store_unavailable' }));
    await grants.load();
    expect(grants.status).toBe('error');
  });
});
