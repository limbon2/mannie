// Every code of the window and tray feature has a message, and the tray's
// labels pass the core's check (ADR-0019).
import { describe, expect, it } from 'vitest';

import type { WindowError } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { trayLabels, windowFailureMessage } from '../../../../../../apps/desktop/src/lib/utils/window-messages.js';

const ERRORS: WindowError[] = [
  { code: 'invalid_label' },
  { code: 'request_not_pending' },
  { code: 'written_by_newer_version' },
  { code: 'storage_failed' },
  { code: 'store_unavailable' },
];

/** The core's limit for a tray label (mannie-types, `MAX_TRAY_LABEL_CHARACTERS`). */
const MAX_TRAY_LABEL_CHARACTERS = 100;

describe('window messages', () => {
  it('say what each window error means', () => {
    const messages = [
      ...ERRORS.map((error) => windowFailureMessage({ error, kind: 'core' })),
      windowFailureMessage({ kind: 'ipc' }),
    ];

    for (const message of messages) {
      expect(message.trim()).not.toBe('');
    }
    expect(new Set(messages).size).toBe(messages.length);
  });

  it('give the tray labels that the core accepts', () => {
    const labels = Object.values(trayLabels());

    for (const label of labels) {
      expect(label.trim()).not.toBe('');
      // Code points, as Rust's `chars()` counts them in the core.
      expect(Array.from(label).length).toBeLessThanOrEqual(MAX_TRAY_LABEL_CHARACTERS);
      expect(label).not.toMatch(/\p{Cc}/u);
    }
  });
});
