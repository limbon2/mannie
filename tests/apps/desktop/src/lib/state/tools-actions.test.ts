// The owner's actions on tool calls (docs/features/system-tools/
// tool-activity.md): what each sends to the core, and what the rows and
// dialogs show when the core refuses it.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  ApprovalRequest,
  ArchiveRef,
  SharedGrant,
  ToolsError,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  OutputRequest,
  PermissionsRequest,
  ToolsActions,
} from '../../../../../../apps/desktop/src/lib/state/tools-actions.svelte.js';

const core = vi.hoisted(() => ({
  decideApproval: vi.fn(),
  listConversationGrants: vi.fn(),
  listSharedGrants: vi.fn(),
  readToolOutput: vi.fn(),
  revokeConversationGrant: vi.fn(),
  revokeSharedGrant: vi.fn(),
}));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));

const CONVERSATION = '0199e0f2-0000-7000-8000-000000000001';
const PART = '0199e0f2-0000-7000-8000-000000000002';

const PROJECT = '0199e0f2-0000-7000-8000-000000000003';

const REQUEST: ApprovalRequest = {
  digest: 'sha256:aa',
  grants: [],
  project: null,
  reasons: ['command'],
  rule: { folder: 'D:\\Projects\\app', words: ['cargo', 'test', '--release'] },
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
  core.listSharedGrants.mockResolvedValue(ok([]));
});

describe('approvals', () => {
  it('sends the decision with the digest of the request the owner saw', async () => {
    const actions = new ToolsActions();
    core.decideApproval.mockResolvedValue(ok(null));

    const accepted = await actions.decide(CONVERSATION, PART, REQUEST.digest, { choice: 'allow_once' });

    expect(accepted).toBe(true);
    expect(core.decideApproval).toHaveBeenCalledWith({
      choice: { choice: 'allow_once' },
      conversationId: CONVERSATION,
      digest: 'sha256:aa',
      partId: PART,
    });
    expect(actions.decisionFailures[PART]).toBeUndefined();
  });

  it('keeps a refusal for the call’s card and clears it with the next decision', async () => {
    const actions = new ToolsActions();
    core.decideApproval.mockResolvedValueOnce(refused({ code: 'approval_changed' }));
    core.decideApproval.mockResolvedValueOnce(ok(null));

    expect(await actions.decide(CONVERSATION, PART, 'sha256:old', { choice: 'deny' })).toBe(false);
    expect(actions.decisionFailures[PART]).toEqual({
      error: { code: 'approval_changed', description: 'refused' },
      kind: 'core',
    });

    expect(await actions.decide(CONVERSATION, PART, 'sha256:new', { choice: 'deny' })).toBe(true);
    expect(actions.decisionFailures[PART]).toBeUndefined();
  });

  it('sends one decision at a time for a call', async () => {
    const actions = new ToolsActions();
    let answer: (value: unknown) => void = () => undefined;
    core.decideApproval.mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );

    const first = actions.decide(CONVERSATION, PART, REQUEST.digest, { choice: 'allow_once' });
    const second = await actions.decide(CONVERSATION, PART, REQUEST.digest, { choice: 'deny' });
    answer(ok(null));

    expect(second).toBe(false);
    expect(await first).toBe(true);
    expect(core.decideApproval).toHaveBeenCalledTimes(1);
  });
});

describe('the "Save rule" dialog', () => {
  it('starts with the whole command, limited to its folder', () => {
    const actions = new ToolsActions();

    actions.startSaveRule(CONVERSATION, PART, REQUEST);

    expect(actions.ruleOpen).toBe(true);
    expect(actions.rule?.prefixWords).toBe(3);
    expect(actions.rule?.limitToFolder).toBe(true);
  });

  it('opens only for a request that offers a rule', () => {
    const actions = new ToolsActions();

    actions.startSaveRule(CONVERSATION, PART, { ...REQUEST, rule: null });

    expect(actions.ruleOpen).toBe(false);
  });

  it('sends the owner’s choice and closes when the core accepted it', async () => {
    const actions = new ToolsActions();
    core.decideApproval.mockResolvedValue(ok(null));
    actions.startSaveRule(CONVERSATION, PART, REQUEST);
    const request = actions.rule;
    if (request === null) {
      throw new Error('the dialog did not open');
    }
    request.prefixWords = 2;
    request.limitToFolder = false;

    await actions.saveRule(request);

    expect(core.decideApproval).toHaveBeenCalledWith({
      choice: { choice: 'save_rule', limitToFolder: false, prefixWords: 2 },
      conversationId: CONVERSATION,
      digest: 'sha256:aa',
      partId: PART,
    });
    expect(actions.ruleOpen).toBe(false);
  });

  it('closes when the request is gone, and stays open for another refusal', async () => {
    const actions = new ToolsActions();
    actions.startSaveRule(CONVERSATION, PART, REQUEST);
    const request = actions.rule;
    if (request === null) {
      throw new Error('the dialog did not open');
    }

    core.decideApproval.mockResolvedValueOnce(refused({ code: 'rule_prefix_invalid' }));
    await actions.saveRule(request);
    expect(actions.ruleOpen).toBe(true);

    core.decideApproval.mockResolvedValueOnce(refused({ code: 'approval_not_pending' }));
    await actions.saveRule(request);
    expect(actions.ruleOpen).toBe(false);
    expect(actions.decisionFailures[PART]?.kind).toBe('core');
  });
});

describe('archived outputs', () => {
  const ARCHIVE: ArchiveRef = { bytes: 10_000, id: '0199e0f2-0000-7000-a000-000000000001', lines: 700, name: 'stdout' };

  it('reads page after page until the whole output is shown', async () => {
    const request = new OutputRequest(CONVERSATION, ARCHIVE);
    core.readToolOutput.mockResolvedValueOnce(ok({ firstLine: 1, lines: ['a', 'b'], totalLines: 3 }));
    core.readToolOutput.mockResolvedValueOnce(ok({ firstLine: 3, lines: ['c'], totalLines: 3 }));

    await request.loadMore();
    expect(request.hasMore).toBe(true);
    await request.loadMore();

    expect(request.lines).toEqual(['a', 'b', 'c']);
    expect(request.hasMore).toBe(false);
    expect(core.readToolOutput).toHaveBeenLastCalledWith({
      archiveId: ARCHIVE.id,
      conversationId: CONVERSATION,
      firstLine: 3,
      limit: 500,
    });
  });

  it('keeps what it read when a later page fails', async () => {
    const request = new OutputRequest(CONVERSATION, ARCHIVE);
    core.readToolOutput.mockResolvedValueOnce(ok({ firstLine: 1, lines: ['a'], totalLines: 2 }));
    core.readToolOutput.mockResolvedValueOnce(refused({ code: 'output_not_found' }));

    await request.loadMore();
    await request.loadMore();

    expect(request.lines).toEqual(['a']);
    expect(request.failure?.kind).toBe('core');
  });
});

describe('permissions of a conversation', () => {
  const GRANT = {
    createdAt: 0,
    id: '0199e0f2-0000-7000-8000-0000000000aa',
    scope: { folder: 'D:\\Projects\\app', kind: 'path' as const, mode: 'write' as const },
  };

  it('revokes a grant, and drops one that no longer exists', async () => {
    const request = new PermissionsRequest(CONVERSATION, null);
    core.listConversationGrants.mockResolvedValue(ok([GRANT, { ...GRANT, id: 'other' }]));
    await request.load();

    core.revokeConversationGrant.mockResolvedValueOnce(ok(null));
    await request.revoke(GRANT.id);
    core.revokeConversationGrant.mockResolvedValueOnce(refused({ code: 'grant_not_found' }));
    await request.revoke('other');

    expect(request.grants).toEqual([]);
    expect(request.revokeFailure).toBeNull();
  });

  it('keeps a grant whose revocation failed, with the cause', async () => {
    const request = new PermissionsRequest(CONVERSATION, null);
    core.listConversationGrants.mockResolvedValue(ok([GRANT]));
    await request.load();

    core.revokeConversationGrant.mockResolvedValueOnce(refused({ code: 'storage_failed' }));
    await request.revoke(GRANT.id);

    expect(request.grants).toEqual([GRANT]);
    expect(request.revokeFailure?.kind).toBe('core');
  });

  it('reports a list that could not be read', async () => {
    const request = new PermissionsRequest(CONVERSATION, null);
    core.listConversationGrants.mockResolvedValue(refused({ code: 'conversation_not_found' }));

    await request.load();

    expect(request.status).toBe('error');
    expect(request.readFailure?.kind).toBe('core');
  });

  it('groups the grants of the conversation, its project and every conversation', async () => {
    const request = new PermissionsRequest(CONVERSATION, PROJECT);
    const shared: SharedGrant[] = [
      { ...GRANT, id: 'project', projectId: PROJECT },
      { ...GRANT, id: 'always', projectId: null },
      { ...GRANT, id: 'elsewhere', projectId: '0199e0f2-0000-7000-8000-000000000009' },
    ];
    core.listConversationGrants.mockResolvedValue(ok([GRANT]));
    core.listSharedGrants.mockResolvedValue(ok(shared));

    await request.load();

    expect(request.grants.map((grant) => grant.id)).toEqual([GRANT.id]);
    expect(request.projectGrants.map((grant) => grant.id)).toEqual(['project']);
    expect(request.alwaysGrants.map((grant) => grant.id)).toEqual(['always']);
  });

  it('revokes a grant of the project or for every conversation', async () => {
    const request = new PermissionsRequest(CONVERSATION, PROJECT);
    core.listConversationGrants.mockResolvedValue(ok([]));
    core.listSharedGrants.mockResolvedValue(ok([{ ...GRANT, id: 'always', projectId: null }]));
    await request.load();

    core.revokeSharedGrant.mockResolvedValueOnce(ok(null));
    await request.revokeShared('always');

    expect(core.revokeSharedGrant).toHaveBeenCalledWith({ grantId: 'always' });
    expect(request.alwaysGrants).toEqual([]);
  });

  it('reports the shared grants that could not be read', async () => {
    const request = new PermissionsRequest(CONVERSATION, null);
    core.listConversationGrants.mockResolvedValue(ok([]));
    core.listSharedGrants.mockResolvedValue(refused({ code: 'storage_failed' }));

    await request.load();

    expect(request.status).toBe('error');
    expect(request.readFailure?.kind).toBe('core');
  });
});
