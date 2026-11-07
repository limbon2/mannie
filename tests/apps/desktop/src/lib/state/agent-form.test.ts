// The agent editor's form (docs/features/custom-agents.md): tools chosen by
// groups, what "Only reads" takes away, the steps' order, what it sends to the
// core, whether it holds changes, and which field the core refused.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  AgentDraft,
  AgentStep,
  AgentToolGroup,
  AgentView,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { AgentForm } from '../../../../../../apps/desktop/src/lib/state/agent-form.svelte.js';

const core = vi.hoisted(() => ({
  create: vi.fn(),
  find: vi.fn(),
  update: vi.fn(),
}));

vi.mock('../../../../../../apps/desktop/src/lib/state/agents.svelte.js', () => ({
  agents: { create: core.create, find: core.find, update: core.update },
}));

function group(id: AgentToolGroup['id'], tools: string[], forReadOnly: boolean): AgentToolGroup {
  return { forChild: true, forConversation: true, forReadOnly, id, tools };
}

const READ = group('read', ['read_file', 'search_text'], true);
const CHANGE = group('change', ['write_file'], false);
const COMMANDS = group('commands', ['run_command'], false);
const QUESTIONS = group('questions', ['ask_owner'], true);
const TASKS = group('tasks', ['start_task', 'follow_up_task', 'wait_for_tasks'], true);
const GROUPS = [READ, CHANGE, COMMANDS, QUESTIONS, TASKS];

function draft(overrides: Partial<AgentDraft> = {}): AgentDraft {
  return {
    children: null,
    description: '',
    instructions: 'Do it.',
    kind: 'conversation',
    maxTasks: 20,
    maxToolCalls: 250,
    maxToolRounds: 100,
    name: 'Mine',
    plugins: null,
    profileId: null,
    readOnly: false,
    steps: [],
    tools: ['read_file', 'search_text', 'write_file', 'run_command', 'ask_owner', ...TASKS.tools],
    ...overrides,
  };
}

function view(overrides: Partial<AgentDraft> = {}, builtin = false): AgentView {
  return { builtin, draft: draft(overrides), key: 'custom.mine', updatedAt: 0 };
}

function step(title: string, agent: string | null = null): AgentStep {
  return { agent, askFirst: true, instruction: `${title} text`, several: agent !== null, title };
}

beforeEach(() => {
  core.create.mockReset();
  core.find.mockReset();
  core.update.mockReset();
  core.find.mockImplementation((key: string) => {
    const isKnown = key === 'explore' || key === 'general';
    return isKnown ? { draft: { kind: 'child', readOnly: key === 'explore' } } : undefined;
  });
});

describe('AgentForm', () => {
  it('holds no changes until the owner makes one, and none for a built-in agent', () => {
    const form = new AgentForm(view());
    const builtin = new AgentForm(view({}, true));

    expect(form.dirty).toBe(false);
    form.name = 'Other';
    builtin.name = 'Other';

    expect(form.dirty).toBe(true);
    expect(builtin.dirty).toBe(false);
  });

  it('starts a new agent without changes, so leaving an untouched one does not ask', () => {
    const form = new AgentForm({ draft: draft(), key: null });

    expect(form.dirty).toBe(false);
    form.addStep();
    expect(form.dirty).toBe(true);
  });

  it('turns a group of tools on and off as a whole', () => {
    const form = new AgentForm(view({ tools: ['read_file', 'search_text'] }));

    expect(form.hasGroup(READ)).toBe(true);
    expect(form.hasGroup(CHANGE)).toBe(false);

    form.setGroup(CHANGE, true);
    form.setGroup(READ, false);

    expect(form.tools).toEqual(['write_file']);
  });

  it('takes the steps’ child agents away with the task tools, and asking with the questions', () => {
    const form = new AgentForm(view({ steps: [step('Look', 'explore')] }));

    form.setGroup(TASKS, false);
    expect(form.steps[0]).toEqual(expect.objectContaining({ agent: null, askFirst: true, several: false }));

    form.setGroup(QUESTIONS, false);
    expect(form.steps[0]).toEqual(expect.objectContaining({ askFirst: false }));
  });

  it('takes away what an agent that only reads cannot have', () => {
    const form = new AgentForm(
      view({
        children: ['explore', 'general'],
        steps: [step('Look', 'explore'), step('Change', 'general')],
      }),
    );

    form.setReadOnly(true, GROUPS);

    expect(form.readOnly).toBe(true);
    expect(form.tools).toEqual(['read_file', 'search_text', 'ask_owner', ...TASKS.tools]);
    expect(form.children).toEqual(['explore']);
    expect(form.steps.map((entry) => entry.agent)).toEqual(['explore', null]);
    expect(form.steps[1]?.several).toBe(false);
  });

  it('keeps everything when "Only reads" is turned off', () => {
    const form = new AgentForm(view({ readOnly: true, tools: ['read_file'] }));

    form.setReadOnly(false, GROUPS);

    expect(form.readOnly).toBe(false);
    expect(form.tools).toEqual(['read_file']);
  });

  it('moves between every plugin and a list that starts with those that exist', () => {
    const form = new AgentForm(view());

    form.setEveryPlugin(false, ['git', 'ssh']);
    expect(form.plugins).toEqual(['git', 'ssh']);

    form.setPlugin('git', false);
    expect(form.plugins).toEqual(['ssh']);

    form.setPlugin('mcp', true);
    expect(form.plugins).toEqual(['ssh', 'mcp']);

    form.setEveryPlugin(true, []);
    expect(form.plugins).toBeNull();
  });

  it('drops a child agent that no longer exists, which is no change of the owner’s', () => {
    const form = new AgentForm(
      view({
        children: ['explore', 'custom.gone'],
        steps: [step('Look', 'explore'), step('Old', 'custom.gone')],
      }),
    );

    expect(form.children).toEqual(['explore']);
    expect(form.steps.map((entry) => entry.agent)).toEqual(['explore', null]);
    expect(form.steps[1]?.several).toBe(false);
    expect(form.dirty).toBe(false);
  });

  it('drops a child agent that stopped only reading from an agent that only reads', () => {
    const form = new AgentForm(
      view({
        children: ['explore', 'general'],
        readOnly: true,
        steps: [step('Change', 'general')],
      }),
    );

    expect(form.children).toEqual(['explore']);
    expect(form.steps[0]?.agent).toBeNull();
    expect(form.dirty).toBe(false);
  });

  it('takes a child agent off the steps that name it', () => {
    const form = new AgentForm(view({ children: ['explore', 'general'], steps: [step('Look', 'explore')] }));

    form.setChild('explore', false);

    expect(form.children).toEqual(['general']);
    expect(form.steps[0]?.agent).toBeNull();
  });

  it('adds, moves and removes steps, and stays inside the list', () => {
    const form = new AgentForm(view({ steps: [step('One'), step('Two'), step('Three')] }));

    form.moveStep(0, -1);
    form.moveStep(2, 1);
    expect(form.steps.map((entry) => entry.title)).toEqual(['One', 'Two', 'Three']);

    form.moveStep(0, 1);
    expect(form.steps.map((entry) => entry.title)).toEqual(['Two', 'One', 'Three']);
    form.moveStep(2, -1);
    expect(form.steps.map((entry) => entry.title)).toEqual(['Two', 'Three', 'One']);

    form.removeStep(1);
    form.addStep();
    expect(form.steps.map((entry) => entry.title)).toEqual(['Two', 'One', '']);
  });

  it('creates a new agent, and has no changes after the core took it', async () => {
    const saved = view();
    core.create.mockResolvedValue({ data: saved, ok: true });
    const form = new AgentForm({ draft: draft({ name: 'New' }), key: null });
    form.instructions = 'Changed.';

    const result = await form.save();

    expect(result).toEqual(saved);
    expect(core.create).toHaveBeenCalledWith(expect.objectContaining({ instructions: 'Changed.', name: 'New' }));
    expect(form.dirty).toBe(false);
  });

  it('updates an agent under its key', async () => {
    core.update.mockResolvedValue({ data: view(), ok: true });
    const form = new AgentForm(view());
    form.description = 'Now described.';

    await form.save();

    expect(core.update).toHaveBeenCalledWith({
      draft: expect.objectContaining({ description: 'Now described.' }) as AgentDraft,
      key: 'custom.mine',
    });
  });

  it('keeps its changes and says which field the core refused', async () => {
    const form = new AgentForm(view());
    form.name = 'Taken';
    core.update.mockResolvedValueOnce({
      failure: { error: { code: 'name_taken' }, kind: 'core' },
      ok: false,
    });

    expect(await form.save()).toBeNull();
    expect(form.refusedField).toBe('name');
    expect(form.dirty).toBe(true);

    core.update.mockResolvedValueOnce({
      failure: { error: { code: 'field_not_valid', field: 'steps' }, kind: 'core' },
      ok: false,
    });
    await form.save();
    expect(form.refusedField).toBe('steps');

    core.update.mockResolvedValueOnce({ failure: { kind: 'ipc' }, ok: false });
    await form.save();
    expect(form.refusedField).toBeNull();
  });
});
