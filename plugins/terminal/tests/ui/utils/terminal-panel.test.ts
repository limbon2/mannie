// The addresses of terminals' panels and the cite keys of their commands
// (ADR-0054, decision 2): what the plugin makes reads back, and nothing else
// names a terminal or a command.
import { describe, expect, it } from 'vitest';

import { panelArgument, parseCiteKey, parsePanelArgument } from '../../../ui/utils/terminal-panel.js';

describe('terminal panels', () => {
  it('name a conversation and a terminal, and read them back', () => {
    const argument = panelArgument({ conversationId: '0199e0f2-0000-7000-8000-000000000001', terminal: 2 });

    expect(argument).toBe('0199e0f2-0000-7000-8000-000000000001.2');
    expect(parsePanelArgument(argument)).toEqual({
      conversationId: '0199e0f2-0000-7000-8000-000000000001',
      terminal: 2,
    });
    for (const broken of ['', 'c1', 'c1.', '.2', 'c1.0', 'c1.x']) {
      expect(parsePanelArgument(broken), broken).toBeNull();
    }
  });
});

describe('cite keys', () => {
  it('name a terminal and a command, with lines when a fragment names them', () => {
    expect(parseCiteKey('3f9a2c1d.5')).toEqual({ reveal: { block: 5, lines: null }, session: '3f9a2c1d' });
    expect(parseCiteKey('3f9a2c1d.5#L3-L4')).toEqual({
      reveal: { block: 5, lines: { first: 3, last: 4 } },
      session: '3f9a2c1d',
    });
    expect(parseCiteKey('3f9a2c1d.5#L7')?.reveal.lines).toEqual({ first: 7, last: 7 });
    expect(parseCiteKey('3f9a2c1d.5#L4-L3')?.reveal.lines).toBeNull();
    for (const broken of ['', '3f9a2c1d', '3f9a2c1d.0', 'zzzzzzzz.5', '3f9a2c1d.s1a2b3c4d', '3f9a2c1d.5#x']) {
      expect(parseCiteKey(broken), broken).toBeNull();
    }
  });
});
