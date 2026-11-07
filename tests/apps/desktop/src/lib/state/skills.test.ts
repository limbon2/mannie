// The skills as the settings section uses them (docs/features/skills.md): a
// read keeps what the core found, a refused read keeps the skills shown
// before, and a switch sends its place and name and keeps the core's answer
// or its refusal for that skill.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { SkillError, SkillsOverview } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { skillKey, SkillsState } from '../../../../../../apps/desktop/src/lib/state/skills.svelte.js';

const core = vi.hoisted(() => ({
  listSkills: vi.fn(),
  setSkillOn: vi.fn(),
}));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));

const PROJECT = '0199e0f2-0000-7000-8000-0000000000aa';

function overview(isOn: boolean): SkillsOverview {
  return {
    computer: {
      failure: null,
      folder: '/home/owner/.agents/skills',
      incomplete: false,
      label: null,
      leftOut: 0,
      place: { kind: 'computer' },
      problems: [],
      skills: [
        {
          changed: false,
          description: 'Fills PDF forms.',
          folder: '/home/owner/.agents/skills/pdf',
          isOn,
          listed: true,
          mentionable: true,
          name: 'pdf',
          warnings: [],
        },
      ],
    },
    projects: [],
  };
}

function ok<T>(data: T): { data: T; status: 'ok' } {
  return { data, status: 'ok' };
}

function refused(error: SkillError): { error: SkillError & { description: string }; status: 'error' } {
  return { error: { ...error, description: 'refused' }, status: 'error' };
}

beforeEach(() => {
  for (const mock of Object.values(core)) {
    mock.mockReset();
  }
});

describe('skillKey', () => {
  it('tells a skill of this computer from a project skill of the same name', () => {
    expect(skillKey({ kind: 'computer' }, 'pdf')).toBe('computer:pdf');
    expect(skillKey({ kind: 'project', projectId: PROJECT }, 'pdf')).toBe(`project:${PROJECT}:pdf`);
  });
});

describe('SkillsState', () => {
  it('keeps what a read found and keeps it through a refused read', async () => {
    const skills = new SkillsState();
    core.listSkills.mockResolvedValueOnce(ok(overview(true)));
    core.listSkills.mockResolvedValueOnce(refused({ code: 'storage_failed' }));

    await skills.load();
    await skills.load();

    expect(skills.overview).toEqual(overview(true));
    expect(skills.readFailure).toEqual({ error: { code: 'storage_failed', description: 'refused' }, kind: 'core' });
    expect(skills.reading).toBe(false);
  });

  it('sends a switch with its place and name and keeps the core answer', async () => {
    const skills = new SkillsState();
    core.setSkillOn.mockResolvedValueOnce(ok(overview(false)));

    await skills.setOn({ kind: 'computer' }, 'pdf', false);

    expect(core.setSkillOn).toHaveBeenCalledWith({ name: 'pdf', on: false, place: { kind: 'computer' } });
    expect(skills.overview).toEqual(overview(false));
    expect(skills.saving).toBeNull();
    expect(skills.saveFailure).toBeNull();
  });

  it('keeps a refused switch for its skill and leaves the skills as they were', async () => {
    const skills = new SkillsState();
    core.listSkills.mockResolvedValueOnce(ok(overview(true)));
    core.setSkillOn.mockResolvedValueOnce(refused({ code: 'not_found' }));
    await skills.load();

    await skills.setOn({ kind: 'project', projectId: PROJECT }, 'docx', true);

    expect(skills.saveFailure).toEqual({
      failure: { error: { code: 'not_found', description: 'refused' }, kind: 'core' },
      key: `project:${PROJECT}:docx`,
    });
    expect(skills.overview).toEqual(overview(true));
  });
});
