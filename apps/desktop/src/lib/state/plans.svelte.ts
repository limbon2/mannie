// The owner's plans as the UI knows them (docs/features/plans.md). A plan is
// model output: the core checks an edit, and a plan runs only when the owner
// presses Send on a message that mentions it (ADR-0060, decision 7).
import {
  commands,
  type MentionEntry,
  type PlanError,
  type PlanSummary,
  type PlanView,
} from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import type { Failure, Outcome } from '#lib/types/calls.js';
import { DEFAULT_AGENT } from '#lib/utils/agents-messages.js';
import { call } from '#lib/utils/calls.js';
import { openConversation, startNewConversation } from '#lib/utils/chat-urls.js';

import { agents } from './agents.svelte.js';
import { conversationDraft } from './conversation-draft.svelte.js';
import { messageBoxes, NEW_CONVERSATION_BOX } from './message-boxes.svelte.js';
import type { MentionChoice } from './message-mentions.svelte.js';

/** The ID of the core's provider of plans' mentions. */
export const PLANS_PROVIDER = 'plans';

/** The key of the starter that hands work to child agents. */
const ORCHESTRATOR = 'custom.orchestrator';

/**
 * The plan as an entry of the "@" list. Its token holds the number and never
 * the title, because a token enters the owner's own text, which the policy
 * reads for a project's name (T-84).
 */
export function planChoice(plan: Pick<PlanSummary, 'number' | 'title'>): MentionChoice {
  const entry: MentionEntry = {
    expands: false,
    key: String(plan.number),
    label: plan.title,
    token: `@plan-${String(plan.number)}`,
    variant: 'plan',
    warning: null,
  };
  return { entry, provider: PLANS_PROVIDER };
}

/** The agent that a plan runs with until the owner chooses another. */
function runningAgent(): string | null {
  return agents.find(ORCHESTRATOR) === undefined ? null : ORCHESTRATOR;
}

/** The window's plans. */
export class Plans {
  /** Why the plans could not be read, if they could not. */
  failure: Failure<PlanError> | null = $state(null);
  plans: PlanSummary[] = $state([]);
  status: 'error' | 'loading' | 'ready' = $state('loading');

  /** The plan that a conversation saved, if it did and the plan exists. */
  ofConversation(conversationId: string): PlanSummary | undefined {
    return this.plans.find((plan) => plan.conversationId === conversationId);
  }

  /** Reads the plans from the core. */
  async reload(): Promise<void> {
    const outcome = await call(() => commands.listPlans());
    if (outcome.ok) {
      this.plans = outcome.data;
      this.status = 'ready';
      this.failure = null;
    } else {
      this.failure = outcome.failure;
      this.status = 'error';
    }
  }

  /** One plan with its text. */
  async read(number: number): Promise<Outcome<PlanView, PlanError>> {
    return await call(() => commands.getPlan({ number }));
  }

  /** The owner's edit of a plan, and the list again. */
  async update(number: number, title: string, body: string): Promise<Outcome<PlanView, PlanError>> {
    const outcome = await call(() => commands.updatePlan({ body, number, title }));
    await this.reload();
    return outcome;
  }

  /** Deletes a plan, and reads the list again. */
  async delete(number: number): Promise<Outcome<null, PlanError>> {
    const outcome = await call(() => commands.deletePlan({ number }));
    await this.reload();
    return outcome;
  }

  /**
   * Opens a new conversation in the plan's project, with the plan mentioned in
   * its message box. The owner can change the agent and add notes, and nothing
   * runs before Send.
   */
  async runInNewConversation(plan: PlanSummary): Promise<void> {
    await agents.ensureLoaded();
    // The intent names the plan's place, so a draft of another place that
    // still shows leaves it to the plan's draft.
    conversationDraft.reset([], null, plan.projectId);
    // The draft may have another agent chosen, so the built-in one is named.
    conversationDraft.agentKey = runningAgent() ?? DEFAULT_AGENT;
    await startNewConversation(plan.projectId);
    // Once the view shows the plan's place: the box of another place's draft
    // is gone by then, and the plan's draft takes the text when its box
    // shows, or at once when it shows already.
    messageBoxes.fill(NEW_CONVERSATION_BOX, m.plans_run_message(), planChoice(plan));
  }

  /**
   * Opens the plan's own conversation with the plan mentioned in its message
   * box, and sets its agent at once, because the agent that saved the plan may
   * only read. The owner can set it back, and nothing runs before Send.
   */
  async runInItsConversation(plan: PlanSummary): Promise<Failure<unknown> | null> {
    const conversationId = plan.conversationId;
    if (conversationId === null) {
      return null;
    }
    await agents.ensureLoaded();
    const agentKey = runningAgent() ?? DEFAULT_AGENT;
    const outcome = await call(() => commands.setConversationAgent({ agentKey, conversationId }));
    if (!outcome.ok) {
      return outcome.failure;
    }
    messageBoxes.fill(conversationId, m.plans_run_message(), planChoice(plan));
    await openConversation(conversationId);
    return null;
  }
}

/** The window's plans. */
export const plans = new Plans();
