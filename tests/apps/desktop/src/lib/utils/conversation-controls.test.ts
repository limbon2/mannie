// The places of conversation controls (ADR-0042): which controls show in the
// toolbar, the conversation's menu and the message box, and what the menu's
// button counts.
import { describe, expect, it } from 'vitest';

import type { ConversationSummary } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import type {
  ConversationControl,
  ConversationControlProps,
} from '../../../../../../apps/desktop/src/lib/types/shell-slots.js';
import { controlsIn, runningInMenu } from '../../../../../../apps/desktop/src/lib/utils/conversation-controls.js';

const component = () => ({});

const CONVERSATION: ConversationSummary = {
  awaitingApproval: false,
  createdAt: 2,
  hasActiveRun: false,
  id: '0199e0f2-0000-7000-8000-000000000001',
  profileId: null,
  projectId: null,
  task: null,
  title: 'A conversation',
  titleSource: 'first_message',
  updatedAt: 2,
  workspace: null,
};

/** The props that the controls receive. */
const PROPS: ConversationControlProps = { conversation: CONVERSATION, turns: [] };

function control(id: string, order: number, changes: Partial<ConversationControl> = {}): ConversationControl {
  return { component, id, order, ...changes };
}

describe('controlsIn', () => {
  it('gives a control that names no place to the toolbar', () => {
    const controls = [control('plugin.branch', 30), control('tools.full-access', 20, { place: 'message-box' })];

    expect(controlsIn(controls, 'toolbar').map((entry) => entry.id)).toEqual(['plugin.branch']);
    expect(controlsIn(controls, 'message-box').map((entry) => entry.id)).toEqual(['tools.full-access']);
    expect(controlsIn(controls, 'menu')).toEqual([]);
  });

  it('lists the controls of a place by their order', () => {
    const controls = [
      control('export', 30, { place: 'menu' }),
      control('child-agents', 10, { place: 'menu' }),
      control('commands', 15, { place: 'menu' }),
    ];

    expect(controlsIn(controls, 'menu').map((entry) => entry.id)).toEqual(['child-agents', 'commands', 'export']);
  });
});

describe('runningInMenu', () => {
  it('adds up what the menu controls report as working', () => {
    const controls = [
      control('child-agents', 10, { place: 'menu', running: () => 2 }),
      control('commands', 15, { place: 'menu', running: () => 1 }),
      control('export', 30, { place: 'menu' }),
    ];

    expect(runningInMenu(controls, PROPS)).toBe(3);
  });

  it('counts nothing of controls in other places', () => {
    const controls = [
      control('toolbar', 10, { running: () => 4 }),
      control('message-box', 20, { place: 'message-box', running: () => 5 }),
    ];

    expect(runningInMenu(controls, PROPS)).toBe(0);
  });

  it('hands each control the props of the conversation', () => {
    const seen: ConversationControlProps[] = [];
    const controls = [
      control('commands', 15, {
        place: 'menu',
        running: (props) => {
          seen.push(props);
          return 0;
        },
      }),
    ];

    runningInMenu(controls, PROPS);

    expect(seen).toEqual([PROPS]);
  });
});
