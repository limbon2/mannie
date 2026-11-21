// Every code of the notifications feature has a distinct message, and the
// localized notification titles and tray tooltip pass the core's check
// (ADR-0019; ADR-0074).
import { describe, expect, it } from 'vitest';

import type { NotificationError } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  notificationFailureMessage,
  notificationLabels,
} from '../../../../../../apps/desktop/src/lib/utils/notifications-messages.js';

const ERRORS: NotificationError[] = [
  { code: 'invalid_label' },
  { code: 'written_by_newer_version' },
  { code: 'storage_failed' },
  { code: 'store_unavailable' },
];

/** The core's limit for a notification label (mannie-types, `MAX_NOTIFICATION_LABEL_CHARACTERS`). */
const MAX_NOTIFICATION_LABEL_CHARACTERS = 100;

describe('notification messages', () => {
  it('say what each notification error means', () => {
    const messages = [
      ...ERRORS.map((error) => notificationFailureMessage({ error, kind: 'core' })),
      notificationFailureMessage({ kind: 'ipc' }),
    ];

    for (const message of messages) {
      expect(message.trim()).not.toBe('');
    }
    expect(new Set(messages).size).toBe(messages.length);
  });

  it('give notification labels that the core accepts', () => {
    const labels = Object.values(notificationLabels());

    expect(labels.length).toBeGreaterThan(0);
    for (const label of labels) {
      expect(label.trim()).not.toBe('');
      // Code points, as Rust's `chars()` counts them in the core.
      expect(Array.from(label).length).toBeLessThanOrEqual(MAX_NOTIFICATION_LABEL_CHARACTERS);
      expect(label).not.toMatch(/\p{Cc}/u);
    }
  });
});
