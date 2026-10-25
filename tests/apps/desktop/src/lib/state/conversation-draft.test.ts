// What a new conversation starts with attached (ADR-0046, decision 6): the
// draft keeps the owner's order, gives an attachment that it holds its new
// label, detaches, starts over, and holds the home that a plugin gave it
// (ADR-0051).
import { describe, expect, it } from 'vitest';

import { ConversationDraftState } from '../../../../../../apps/desktop/src/lib/state/conversation-draft.svelte.js';

const notebook = (key: string, label: string) => ({ key, label, provider: 'notebooks' });

describe('ConversationDraftState', () => {
  it('keeps the order of the attachments and takes a new label for one that it holds', () => {
    const draft = new ConversationDraftState();

    draft.attach(notebook('n1', 'Physics'));
    draft.attach(notebook('n2', 'Optics'));
    draft.attach(notebook('n1', 'Physics notes'));

    expect(draft.attachments).toEqual([notebook('n1', 'Physics notes'), notebook('n2', 'Optics')]);
    expect(draft.refs()).toEqual([
      { key: 'n1', provider: 'notebooks' },
      { key: 'n2', provider: 'notebooks' },
    ]);
  });

  it('detaches one attachment, and starts over with the attachments given or none', () => {
    const draft = new ConversationDraftState();
    draft.attach(notebook('n1', 'Physics'));
    draft.attach({ key: 'n1', label: 'Wiki', provider: 'wiki' });

    draft.detach({ key: 'n1', provider: 'notebooks' });
    const left = [...draft.attachments];
    draft.detach({ key: 'n9', provider: 'notebooks' });
    draft.reset([notebook('n2', 'Optics')]);
    const reset = [...draft.attachments];
    draft.reset();

    expect(left).toEqual([{ key: 'n1', label: 'Wiki', provider: 'wiki' }]);
    expect(reset).toEqual([notebook('n2', 'Optics')]);
    expect(draft.attachments).toEqual([]);
  });

  it('holds a home first, keeps it attached, sends it apart, and lets go of it (ADR-0051)', () => {
    const draft = new ConversationDraftState();

    draft.reset([notebook('n2', 'Optics'), notebook('n1', 'Physics')], notebook('n1', 'Physics'));
    draft.detach({ key: 'n1', provider: 'notebooks' });
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
});
