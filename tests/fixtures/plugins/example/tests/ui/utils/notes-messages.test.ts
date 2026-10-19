// The plugin's messages (ADR-0019): a call that got no answer and the host's
// refusals come in the core's words, which name the plugin, and the plugin's
// own error in the plugin's.
import { describe, expect, it } from 'vitest';

import { notesFailureMessage } from '../../../ui/utils/notes-messages.js';

describe('notesFailureMessage', () => {
  it('words a call without an answer, a refusal of the host and the plugin’s own error apart', () => {
    const messages = [
      notesFailureMessage({ kind: 'ipc' }),
      notesFailureMessage({
        error: { code: 'plugin_off', description: '', origin: 'host', plugin: 'example' },
        kind: 'core',
      }),
      notesFailureMessage({ error: { code: 'storage_failed', description: '', origin: 'plugin' }, kind: 'core' }),
    ];

    expect(new Set(messages).size).toBe(3);
    expect(messages[1]).toContain('Example');
    expect(messages[2]).toBe('The example plugin could not store the note. Try again.');
  });
});
