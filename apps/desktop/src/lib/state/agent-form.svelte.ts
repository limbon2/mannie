// One opening of the agent editor (docs/features/custom-agents.md): the fields
// of one agent while the owner edits them. Tools are chosen by groups, and the
// list that the core stores holds their names. The core checks the draft again
// when it is saved, because the webview is untrusted (ADR-0004).
import type {
  AgentDraft,
  AgentError,
  AgentField,
  AgentKind,
  AgentStep,
  AgentToolGroup,
  AgentView,
} from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';

import { agents } from './agents.svelte.js';

/** A blank step. */
function blankStep(): AgentStep {
  return { agent: null, askFirst: false, instruction: '', several: false, title: '' };
}

/** What a number field holds: an empty one binds `null`, which the core takes as 0 and checks. */
function wholeNumber(value: number | null): number {
  return Math.max(0, Math.trunc(value ?? 0));
}

/** A copy of a list of names that an agent's field holds, or `null` for "every one". */
function namesOf(names: readonly string[] | null): string[] | null {
  return names === null ? null : [...names];
}

/** Whether an agent can name this child agent: it exists, and one that only reads names only those that only read. */
function canName(child: string, readOnly: boolean): boolean {
  const agent = agents.find(child);
  if (agent?.draft.kind !== 'child') {
    return false;
  }
  return agent.draft.readOnly || !readOnly;
}

/** The fields of one agent, while the owner edits them. */
export class AgentForm {
  /** The agent's key, or `null` for one that is not saved yet. */
  readonly key: string | null;
  /** Whether the agent is built in or a plugin's, so nothing changes. */
  readonly builtin: boolean;
  readonly kind: AgentKind;

  children: string[] | null = $state(null);
  description = $state('');
  /** Why the latest save failed, if it did. */
  failure: Failure<AgentError> | null = $state(null);
  instructions = $state('');
  maxTasks = $state(0);
  maxToolCalls = $state(0);
  maxToolRounds = $state(0);
  name = $state('');
  plugins: string[] | null = $state(null);
  profileId: string | null = $state(null);
  readOnly = $state(false);
  saving = $state(false);
  steps: AgentStep[] = $state([]);
  tools: string[] = $state([]);

  /** What the form started with, so it knows whether it holds changes. */
  #saved: string;

  /** The field that the latest save was refused for, if one was. */
  get refusedField(): AgentField | null {
    if (this.failure?.kind !== 'core') {
      return null;
    }
    const { error } = this.failure;
    if (error.code === 'name_taken') {
      return 'name';
    }
    return error.code === 'field_not_valid' ? error.field : null;
  }

  /** Whether the form holds changes that are not saved. */
  get dirty(): boolean {
    if (this.builtin) {
      return false;
    }
    return JSON.stringify(this.draft()) !== this.#saved;
  }

  constructor(view: Pick<AgentView, 'builtin' | 'draft' | 'key'> | { draft: AgentDraft; key: null }) {
    const { draft } = view;
    this.key = view.key;
    this.builtin = 'builtin' in view && view.builtin;
    this.kind = draft.kind;
    // A child agent that was deleted or stopped only reading stays named in the
    // stored document. The checklist does not show it, so it goes here, and a
    // save is not refused for something that the owner cannot see or change.
    this.children = namesOf(draft.children)?.filter((child) => canName(child, draft.readOnly)) ?? null;
    this.description = draft.description;
    this.instructions = draft.instructions;
    this.maxTasks = draft.maxTasks;
    this.maxToolCalls = draft.maxToolCalls;
    this.maxToolRounds = draft.maxToolRounds;
    this.name = draft.name;
    this.plugins = namesOf(draft.plugins);
    this.profileId = draft.profileId;
    this.readOnly = draft.readOnly;
    this.steps = (draft.steps ?? []).map((step) => {
      const namesNoOne = step.agent === null || canName(step.agent, draft.readOnly);
      return namesNoOne ? { ...step } : { ...step, agent: null, several: false };
    });
    this.tools = [...draft.tools];
    this.#saved = JSON.stringify(this.draft());
  }

  /** What the form holds, as the core takes it. */
  draft(): AgentDraft {
    return {
      children: this.children === null ? null : [...this.children],
      description: this.description,
      instructions: this.instructions,
      kind: this.kind,
      maxTasks: wholeNumber(this.maxTasks),
      maxToolCalls: wholeNumber(this.maxToolCalls),
      maxToolRounds: wholeNumber(this.maxToolRounds),
      name: this.name,
      plugins: this.plugins === null ? null : [...this.plugins],
      profileId: this.profileId,
      readOnly: this.readOnly,
      steps: this.steps.map((step) => ({ ...step })),
      tools: [...this.tools],
    };
  }

  /** Whether the agent has every tool of a group. */
  hasGroup(group: AgentToolGroup): boolean {
    return group.tools.every((tool) => this.tools.includes(tool));
  }

  /** Gives the agent a group's tools, or takes them away. */
  setGroup(group: AgentToolGroup, on: boolean): void {
    const others = this.tools.filter((tool) => !group.tools.includes(tool));
    this.tools = on ? [...others, ...group.tools] : others;
    if (on) {
      return;
    }
    if (group.id === 'tasks') {
      // A step's child agent needs the task tools.
      this.steps = this.steps.map((step) => ({ ...step, agent: null, several: false }));
    }
    if (group.id === 'questions') {
      this.steps = this.steps.map((step) => ({ ...step, askFirst: false }));
    }
  }

  /**
   * Switches whether the agent only reads. Switching it on takes away what the
   * agent can no longer have: tools that act, and the child agents and steps
   * that name a child that acts.
   */
  setReadOnly(on: boolean, groups: readonly AgentToolGroup[]): void {
    this.readOnly = on;
    if (!on) {
      return;
    }
    const acting = [...groups.filter((group) => !group.forReadOnly).flatMap((group) => group.tools), 'move_to_project'];
    this.tools = this.tools.filter((tool) => !acting.includes(tool));
    if (this.children !== null) {
      this.children = this.children.filter((child) => canName(child, true));
    }
    this.steps = this.steps.map((step) => {
      const keeps = step.agent !== null && canName(step.agent, true);
      return keeps ? step : { ...step, agent: null, several: false };
    });
  }

  /** Switches between every plugin and a list that starts with every plugin that has tools now. */
  setEveryPlugin(every: boolean, known: readonly string[]): void {
    this.plugins = every ? null : [...known];
  }

  /** Lets the agent use a plugin's tools, or stops it. */
  setPlugin(plugin: string, on: boolean): void {
    const others = (this.plugins ?? []).filter((name) => name !== plugin);
    this.plugins = on ? [...others, plugin] : others;
  }

  /** Switches between every child agent and a list that starts with every one that exists now. */
  setEveryChild(every: boolean, known: readonly string[]): void {
    this.children = every ? null : [...known];
  }

  /** Lets the agent give tasks to a child agent, or stops it, with the steps that name it. */
  setChild(child: string, on: boolean): void {
    const others = (this.children ?? []).filter((name) => name !== child);
    this.children = on ? [...others, child] : others;
    if (!on) {
      this.steps = this.steps.map((step) => (step.agent === child ? { ...step, agent: null, several: false } : step));
    }
  }

  /** Adds a step at the end. */
  addStep(): void {
    this.steps = [...this.steps, blankStep()];
  }

  /** Removes a step. */
  removeStep(index: number): void {
    this.steps = this.steps.filter((_step, position) => position !== index);
  }

  /** Moves a step up (-1) or down (1), when there is room. */
  moveStep(index: number, offset: -1 | 1): void {
    const target = index + offset;
    const isInside = target >= 0 && target < this.steps.length;
    if (!isInside) {
      return;
    }
    const moved = [...this.steps];
    const [step] = moved.splice(index, 1);
    moved.splice(target, 0, step);
    this.steps = moved;
  }

  /** Saves the agent, and returns it when the core took it. */
  async save(): Promise<AgentView | null> {
    this.saving = true;
    const draft = this.draft();
    const outcome = this.key === null ? await agents.create(draft) : await agents.update({ draft, key: this.key });
    this.saving = false;
    if (!outcome.ok) {
      this.failure = outcome.failure;
      return null;
    }
    this.failure = null;
    this.#saved = JSON.stringify(this.draft());
    return outcome.data;
  }
}
