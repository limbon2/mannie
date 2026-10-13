import { describe, expect, it } from 'vitest';

import {
  settingsFailureMessage,
  startupProblemMessage,
} from '../../../../../../apps/desktop/src/lib/utils/error-messages.js';

describe('settingsFailureMessage', () => {
  it('names the rejected language', () => {
    const message = settingsFailureMessage({ error: { code: 'invalid_locale', tag: 'xx-INVALID' }, kind: 'core' });

    expect(message).toBe('Mannie does not recognize the language xx-INVALID. Choose a language from the list.');
  });

  it('has a message for every code the core reports', () => {
    const codes = ['storage_failed', 'store_unavailable', 'written_by_newer_version'] as const;

    const messages = codes.map((code) => settingsFailureMessage({ error: { code }, kind: 'core' }));

    expect(new Set(messages).size).toBe(codes.length);
    for (const message of messages) {
      expect(message).not.toBe('');
    }
  });

  it('says that the window could not reach the core', () => {
    expect(settingsFailureMessage({ kind: 'ipc' })).toBe("Mannie's window could not reach its core. Restart Mannie.");
  });
});

describe('startupProblemMessage', () => {
  it('names the database and both schema versions', () => {
    const message = startupProblemMessage({
      code: 'database_too_new',
      databaseVersion: 1000,
      path: String.raw`C:\Users\owner\AppData\Local\io.github.limbon2.mannie\mannie.sqlite`,
      supportedVersion: 1,
    });

    expect(message).toContain(String.raw`C:\Users\owner\AppData\Local\io.github.limbon2.mannie\mannie.sqlite`);
    // Schema versions are identifiers, so they have no digit grouping.
    expect(message).toContain('schema version 1000');
    expect(message).toContain('up to version 1');
  });

  it('names the backup folder after a failed migration', () => {
    const message = startupProblemMessage({
      backupDirectory: '/home/owner/.local/share/io.github.limbon2.mannie/backups',
      code: 'migration_failed',
      path: '/home/owner/.local/share/io.github.limbon2.mannie/mannie.sqlite',
      version: 2,
    });

    expect(message).toContain('/home/owner/.local/share/io.github.limbon2.mannie/backups');
    expect(message).toContain('schema version 2');
  });
});
