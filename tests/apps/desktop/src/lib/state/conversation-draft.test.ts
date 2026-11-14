// What the draft conversation of a new conversation's place gets when it
// opens next (ADR-0068; ADR-0046, decision 6): the attachments in the order
// given, the home that a plugin gave (ADR-0051), and an agent, which the view
// takes once while the home stays.
import { describe, expect, it } from 'vitest';

import { ConversationDraftState } from '../../../../../../apps/desktop/src/lib/state/conversation-draft.svelte.js';

const notebook = (key: string, label: string) => ({ key, label, provider: 'notebooks' });

describe('ConversationDraftState', () => {
  it('asks for nothing until it is given attachments or an agent', () => {
    const draft = new ConversationDraftState();
    const empty = draft.hasIntent;

    draft.agentKey = 'plan';

    expect(empty).toBe(false);
    expect(draft.hasIntent).toBe(true);
  });

  it('hands over the attachments and the agent once, and keeps the home (ADR-0068)', () => {
    const draft = new ConversationDraftState();
    draft.reset([notebook('n2', 'Optics'), notebook('n1', 'Physics')], notebook('n1', 'Physics'));
    draft.agentKey = 'plan';

    const intent = draft.takeIntent();

    expect(intent).toEqual({ agentKey: 'plan', attachments: [{ key: 'n2', provider: 'notebooks' }] });
    expect(draft.hasIntent).toBe(false);
    expect(draft.home).toEqual({ key: 'n1', provider: 'notebooks' });
    expect(draft.attachments).toEqual([notebook('n1', 'Physics')]);
  });

  it("is for the place it names, so another place's draft that still shows leaves it alone", () => {
    const draft = new ConversationDraftState();
    const anyPlace = [draft.isFor(null), draft.isFor('p1')];

    draft.reset([notebook('n2', 'Optics')], null, 'p1');
    const forProject = [draft.isFor(null), draft.isFor('p1')];
    draft.takeIntent();

    expect(anyPlace).toEqual([true, true]);
    expect(forProject).toEqual([false, true]);
    expect(draft.isFor(null)).toBe(true);
  });

  it('writes nothing when it hands over no intent, so the view that opens the draft does not run again', () => {
    const draft = new ConversationDraftState();
    draft.reset([notebook('n1', 'Physics')], notebook('n1', 'Physics'));
    const attachments = draft.attachments;

    const intent = draft.takeIntent();

    expect(intent).toEqual({ agentKey: null, attachments: [] });
    expect(draft.attachments).toBe(attachments);
  });

  it('holds a home first, leaves it out of the references, and lets go of it (ADR-0051)', () => {
    const draft = new ConversationDraftState();

    draft.reset([notebook('n2', 'Optics'), notebook('n1', 'Physics')], notebook('n1', 'Physics'));
    const withHome = { attachments: [...draft.attachments], home: draft.home, refs: draft.refs() };
    draft.leaveHome();

    expect(withHome).toEqual({
      attachments: [notebook('n1', 'Physics'), notebook('n2', 'Optics')],
      home: { key: 'n1', provider: 'notebooks' },
      refs: [{ key: 'n2', provider: 'notebooks' }],
    });
    expect(draft.home).toBeNull();
    expect(draft.attachments).toEqual([notebook('n2', 'Optics')]);
  });

  it('forgets the agent and the attachments when it starts over', () => {
    const draft = new ConversationDraftState();
    draft.reset([notebook('n2', 'Optics')]);
    draft.agentKey = 'plan';

    draft.reset();

    expect(draft.agentKey).toBeNull();
    expect(draft.attachments).toEqual([]);
  });
});
