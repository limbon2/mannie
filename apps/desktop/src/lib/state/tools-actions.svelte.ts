// What the owner does about tool calls from more than one place: the tool
// call rows, the command palette and the settings (docs/features/
// system-tools/tool-activity.md). The dialogs these actions open are mounted
// once, as the feature's overlay.
import {
  type ApprovalChoice,
  type ApprovalRequest,
  type ArchiveRef,
  commands,
  type ConversationGrant,
  type ConversationId,
  type GrantId,
  type PartId,
  type ProjectId,
  type RuleProposal,
  type SharedGrant,
} from '#lib/generated/bindings.js';
import type { ToolsFailure } from '#lib/types/tools.js';
import { call } from '#lib/utils/calls.js';

/** How many lines of an archived output one page loads. */
const OUTPUT_PAGE_LINES = 500;

/** Whether a failure says that a grant no longer exists, which leaves it revoked. */
function isGrantGone(failure: ToolsFailure): boolean {
  const isGone = failure.kind === 'core' && failure.error.code === 'grant_not_found';
  return isGone;
}

/**
 * Revokes a grant for a project or for every conversation, and returns why
 * the core refused it. A grant that no longer exists counts as revoked.
 */
export async function revokeSharedGrant(grantId: GrantId): Promise<ToolsFailure | null> {
  const outcome = await call(() => commands.revokeSharedGrant({ grantId }));
  if (outcome.ok || isGrantGone(outcome.failure)) {
    return null;
  }
  return outcome.failure;
}

function isRequestGone(failure: ToolsFailure): boolean {
  if (failure.kind !== 'core') {
    return false;
  }
  const code = failure.error.code;
  const isGone = code === 'approval_changed' || code === 'approval_not_pending';
  return isGone;
}

/** One opening of the "Save rule" dialog, for the command of one approval request. */
export class SaveRuleRequest {
  readonly conversationId: ConversationId;
  readonly digest: string;
  /** Whether the rule applies only in the command's working directory and below it. */
  limitToFolder = $state(true);
  readonly partId: PartId;
  /** How many of the command's leading words the rule keeps. All of them at first, the narrowest rule. */
  prefixWords = $state(1);
  readonly proposal: RuleProposal;

  constructor(conversationId: ConversationId, partId: PartId, digest: string, proposal: RuleProposal) {
    this.conversationId = conversationId;
    this.partId = partId;
    this.digest = digest;
    this.proposal = proposal;
    this.prefixWords = proposal.words.length;
  }
}

/** One opening of the dialog that shows an archived output, loaded page by page. */
export class OutputRequest {
  readonly archive: ArchiveRef;
  readonly conversationId: ConversationId;
  /** Why the latest page could not be read, if it could not. */
  failure: ToolsFailure | null = $state(null);
  /** The lines loaded so far. */
  lines: string[] = $state([]);
  /** Whether a page is being read. */
  loading = $state(false);
  /** The lines of the whole output, once the first page arrived. */
  totalLines: number | null = $state(null);

  constructor(conversationId: ConversationId, archive: ArchiveRef) {
    this.conversationId = conversationId;
    this.archive = archive;
  }

  /** Whether lines follow the loaded ones. */
  get hasMore(): boolean {
    const hasMore = this.totalLines !== null && this.lines.length < this.totalLines;
    return hasMore;
  }

  /** Reads the next page, or the first one. */
  async loadMore(): Promise<void> {
    if (this.loading) {
      return;
    }
    this.loading = true;
    this.failure = null;
    const outcome = await call(() =>
      commands.readToolOutput({
        archiveId: this.archive.id,
        conversationId: this.conversationId,
        firstLine: this.lines.length + 1,
        limit: OUTPUT_PAGE_LINES,
      }),
    );
    this.loading = false;
    if (!outcome.ok) {
      this.failure = outcome.failure;
      return;
    }
    this.lines = [...this.lines, ...outcome.data.lines];
    this.totalLines = outcome.data.totalLines;
  }
}

/**
 * One opening of the dialog with the grants that apply to a conversation:
 * its own, those of its project, and those for every conversation.
 */
export class PermissionsRequest {
  readonly conversationId: ConversationId;
  /** The conversation's own grants, the oldest first. */
  grants: ConversationGrant[] = $state([]);
  /** The conversation's project, whose grants apply to it too. */
  readonly projectId: ProjectId | null;
  /** Why the grants could not be read, if they could not. */
  readFailure: ToolsFailure | null = $state(null);
  /** Why the latest revocation failed, if it failed. */
  revokeFailure: ToolsFailure | null = $state(null);
  /** The grants being revoked. */
  revoking: GrantId[] = $state([]);
  /** The grants for a project or for every conversation, the oldest first. */
  shared: SharedGrant[] = $state([]);
  status: 'error' | 'loading' | 'ready' = $state('loading');

  constructor(conversationId: ConversationId, projectId: ProjectId | null) {
    this.conversationId = conversationId;
    this.projectId = projectId;
  }

  /** The grants for every conversation. */
  get alwaysGrants(): SharedGrant[] {
    return this.shared.filter((grant) => grant.projectId === null);
  }

  /** The grants of the conversation's project. */
  get projectGrants(): SharedGrant[] {
    const projectId = this.projectId;
    return projectId === null ? [] : this.shared.filter((grant) => grant.projectId === projectId);
  }

  /** Reads the grants from the core. */
  async load(): Promise<void> {
    this.status = 'loading';
    this.readFailure = null;
    const [own, shared] = await Promise.all([
      call(() => commands.listConversationGrants({ conversationId: this.conversationId })),
      call(() => commands.listSharedGrants()),
    ]);
    if (own.ok && shared.ok) {
      this.grants = own.data;
      this.shared = shared.data;
      this.status = 'ready';
      return;
    }
    if (!own.ok) {
      this.readFailure = own.failure;
    } else if (!shared.ok) {
      this.readFailure = shared.failure;
    }
    this.status = 'error';
  }

  /** Revokes a grant of the conversation. A grant that no longer exists leaves the list as well. */
  async revoke(grantId: GrantId): Promise<void> {
    if (this.revoking.includes(grantId)) {
      return;
    }
    this.revoking = [...this.revoking, grantId];
    this.revokeFailure = null;
    const outcome = await call(() =>
      commands.revokeConversationGrant({ conversationId: this.conversationId, grantId }),
    );
    this.revoking = this.revoking.filter((revoking) => revoking !== grantId);
    if (outcome.ok || isGrantGone(outcome.failure)) {
      this.grants = this.grants.filter((grant) => grant.id !== grantId);
      return;
    }
    this.revokeFailure = outcome.failure;
  }

  /** Revokes a grant of the project or for every conversation. */
  async revokeShared(grantId: GrantId): Promise<void> {
    if (this.revoking.includes(grantId)) {
      return;
    }
    this.revoking = [...this.revoking, grantId];
    this.revokeFailure = null;
    const failure = await revokeSharedGrant(grantId);
    this.revoking = this.revoking.filter((revoking) => revoking !== grantId);
    if (failure === null) {
      this.shared = this.shared.filter((grant) => grant.id !== grantId);
      return;
    }
    this.revokeFailure = failure;
  }
}

/** The owner's actions on tool calls, and the dialogs they open. */
export class ToolsActions {
  /** Why the core refused the latest decision about a tool call, by its part. */
  decisionFailures: Partial<Record<PartId, ToolsFailure>> = $state({});
  /** The tool calls whose decision is being sent. */
  deciding: PartId[] = $state([]);
  /** The latest output dialog, which stays while it closes. */
  output: OutputRequest | null = $state(null);
  outputOpen = $state(false);
  /** The latest permissions dialog, which stays while it closes. */
  permissions: PermissionsRequest | null = $state(null);
  permissionsOpen = $state(false);
  /** The latest "Save rule" dialog, which stays while it closes. */
  rule: SaveRuleRequest | null = $state(null);
  ruleOpen = $state(false);

  /**
   * Sends the owner's answer to an approval request, and returns whether the
   * core accepted it. A refusal is kept for the call's card.
   */
  async decide(
    conversationId: ConversationId,
    partId: PartId,
    digest: string,
    choice: ApprovalChoice,
  ): Promise<boolean> {
    if (this.deciding.includes(partId)) {
      return false;
    }
    this.deciding = [...this.deciding, partId];
    const { [partId]: _previous, ...others } = this.decisionFailures;
    this.decisionFailures = others;
    const outcome = await call(() => commands.decideApproval({ choice, conversationId, digest, partId }));
    this.deciding = this.deciding.filter((deciding) => deciding !== partId);
    if (!outcome.ok) {
      this.decisionFailures = { ...this.decisionFailures, [partId]: outcome.failure };
      return false;
    }
    return true;
  }

  /**
   * Allows the call of a "Save rule" dialog and saves the rule. The dialog
   * closes when the core accepted it, or when the request it answers is gone
   * or changed, which the call's card then explains.
   */
  async saveRule(request: SaveRuleRequest): Promise<void> {
    const choice: ApprovalChoice = {
      choice: 'save_rule',
      limitToFolder: request.limitToFolder,
      prefixWords: request.prefixWords,
    };
    const accepted = await this.decide(request.conversationId, request.partId, request.digest, choice);
    const failure = this.decisionFailures[request.partId];
    const closes = accepted || (failure !== undefined && isRequestGone(failure));
    const isLatest = this.rule === request;
    if (closes && isLatest) {
      this.ruleOpen = false;
    }
  }

  /** Opens the dialog that shows an archived output, and reads its first page. */
  showOutput(conversationId: ConversationId, archive: ArchiveRef): void {
    const request = new OutputRequest(conversationId, archive);
    this.output = request;
    this.outputOpen = true;
    void request.loadMore();
  }

  /** Opens the dialog with the grants that apply to a conversation, and reads them. */
  showPermissions(conversationId: ConversationId, projectId: ProjectId | null): void {
    const request = new PermissionsRequest(conversationId, projectId);
    this.permissions = request;
    this.permissionsOpen = true;
    void request.load();
  }

  /** Opens the "Save rule" dialog for an approval request that offers a rule. */
  startSaveRule(conversationId: ConversationId, partId: PartId, request: ApprovalRequest): void {
    if (request.rule === null) {
      return;
    }
    const { [partId]: _previous, ...others } = this.decisionFailures;
    this.decisionFailures = others;
    this.rule = new SaveRuleRequest(conversationId, partId, request.digest, request.rule);
    this.ruleOpen = true;
  }
}

/** The window's tool actions. */
export const toolsActions = new ToolsActions();
