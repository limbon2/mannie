// Every code of the terminal plugin has its words (ADR-0019): its errors,
// its tools' failures, its cards' lines, its mentions and the states of
// terminals.
import { describe, expect, it } from 'vitest';

import type { TerminalError, TerminalSummary } from '../../../ui/generated/bindings.js';
import {
  endingMessage,
  mentionRefusal,
  mentionSummary,
  mentionVariantName,
  phaseLabel,
  runSummary,
  terminalFailureMessage,
  terminalName,
  toolFailure,
} from '../../../ui/utils/terminal-messages.js';

const ERRORS: TerminalError[] = [
  { code: 'block_not_found' },
  { code: 'conversation_not_found' },
  { code: 'folder_invalid' },
  { code: 'host_not_found' },
  { code: 'host_unavailable', reason: 'key_changed' },
  { code: 'open_failed', reason: 'spawn_failed' },
  { code: 'shell_not_listed' },
  { code: 'storage_failed' },
  { code: 'terminal_ended' },
  { code: 'terminal_not_found' },
  { code: 'too_many', limit: 8 },
  { code: 'unsupported' },
];

describe('the terminal plugin words', () => {
  it('each of its errors differently', () => {
    const messages = ERRORS.map((error) =>
      terminalFailureMessage({ error: { ...error, description: '', origin: 'plugin' }, kind: 'core' }),
    );

    for (const message of messages) {
      expect(message.trim()).not.toBe('');
    }
    expect(new Set(messages).size).toBe(messages.length);
  });

  it('the failures of its tool, and nothing for a code it does not know', () => {
    const codes = [
      'interrupted',
      'owner_typing',
      'still_running',
      'terminal_busy',
      'terminal_changed',
      'terminal_ended',
      'terminal_not_opened',
      'terminal_plain',
    ];

    for (const code of codes) {
      expect(toolFailure(code), code).not.toBeNull();
    }
    expect(toolFailure('other')).toBeNull();
  });

  it('a call that ran by its command, terminal and exit code', () => {
    const call = (data: Record<string, unknown> | null) => ({
      arguments: {},
      data,
      failure: null,
      status: 'completed' as const,
    });

    expect(runSummary(call({ command: 'ls', exitCode: 0, terminal: 2 }))).toBe('Ran ls in terminal 2 · exit 0');
    expect(runSummary(call({ command: 'sleep 9', exitCode: 130, interrupted: true, terminal: 1 }))).toBe(
      'Interrupted sleep 9 in terminal 1',
    );
    expect(runSummary(call(null))).toBeNull();
  });

  it('its mentions, terminals and endings', () => {
    expect(mentionSummary({ code: 'lines', values: { shown: 3, total: 3 } })).toBe('3 lines');
    expect(mentionSummary({ code: 'lines', values: { shown: 2, total: 9 } })).toBe('2 of 9 lines');
    expect(mentionSummary({ code: 'other', values: {} })).toBeNull();
    expect(mentionVariantName('command')).toBe('Command');
    expect(mentionRefusal('not_found')).not.toBeNull();
    const summary: TerminalSummary = {
      ending: null,
      host: { endpoint: 'deploy@build-box:22', key: 'k1', kind: 'remote', label: 'build-box', plugin: 'ssh' },
      marking: true,
      number: 2,
      openedBy: 'owner',
      phase: 'prompt',
      session: '3f9a2c1d',
      shell: 'bash',
      state: null,
    };
    expect(terminalName(summary)).toBe('Terminal 2 · bash · build-box');
    expect(phaseLabel('running')).toBe('Running a command');
    expect(endingMessage({ code: 3, kind: 'exited' })).toBe('The terminal ended. Its shell exited with 3.');
  });
});
