// The words of updates (docs/features/updates.md): every code of the core
// has a sentence from the catalog, the question before a restart names the
// first of what the restart would end, and a download shows as a share of its
// size, or in megabytes when the server names no size.
import { describe, expect, it } from 'vitest';

import type { UpdateFailureCode, UpdatesError } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import type { UpdateAction } from '../../../../../../apps/desktop/src/lib/types/updates.js';
import {
  actionFailureTitle,
  checkTime,
  downloadedAmount,
  downloadedPercent,
  restartQuestionTitle,
  updateFailureText,
  updatesFailureMessage,
} from '../../../../../../apps/desktop/src/lib/utils/updates-messages.js';

const ERRORS: readonly UpdatesError[] = [
  { code: 'not_available_in_this_build' },
  { code: 'version_not_ready' },
  { code: 'written_by_newer_version' },
  { code: 'storage_failed' },
  { code: 'store_unavailable' },
  { code: 'browser_unavailable' },
];

const FAILURE_CODES: readonly UpdateFailureCode[] = [
  'unreachable',
  'no_update_information',
  'no_package_for_this_installation',
  'download_failed',
  'signature_invalid',
];

const ACTIONS: readonly UpdateAction[] = ['check', 'page', 'restart'];

/** The formats of the helper, which follow the system's language. */
const shares = new Intl.NumberFormat(undefined, { maximumFractionDigits: 0, style: 'percent' });
const megabytes = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1, style: 'unit', unit: 'megabyte' });
const times = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' });

describe('updates messages', () => {
  it('say what each error of an updates command means', () => {
    const messages = [
      ...ERRORS.map((error) => updatesFailureMessage({ error, kind: 'core' })),
      updatesFailureMessage({ kind: 'ipc' }),
    ];

    for (const message of messages) {
      expect(message.trim()).not.toBe('');
    }
    expect(new Set(messages).size).toBe(messages.length);
  });

  it('title each failed action of the owner differently', () => {
    const titles = ACTIONS.map((action) => actionFailureTitle(action));

    for (const title of titles) {
      expect(title.trim()).not.toBe('');
    }
    expect(new Set(titles).size).toBe(titles.length);
  });

  it('word every failed check with a title and with what the owner can do', () => {
    const texts = FAILURE_CODES.map((code) => updateFailureText(code));

    for (const text of texts) {
      expect(text.title.trim()).not.toBe('');
      expect(text.description.trim()).not.toBe('');
    }
    const sentences = texts.map((text) => `${text.title}\n${text.description}`);
    expect(new Set(sentences).size).toBe(sentences.length);
  });

  it('offer the releases page only for a release that the owner must install by hand', () => {
    const offering = FAILURE_CODES.filter((code) => updateFailureText(code).offersReleasesPage);

    expect(offering).toEqual(['no_update_information', 'no_package_for_this_installation']);
  });

  it('title the question before a restart after the first of what the restart ends', () => {
    expect(restartQuestionTitle({ activeRuns: 1, backgroundCommands: 1, terminals: 1 })).toBe(
      'Restart while replies are being written?',
    );
    expect(restartQuestionTitle({ activeRuns: 0, backgroundCommands: 2, terminals: 1 })).toBe(
      'Restart while background commands run?',
    );
    expect(restartQuestionTitle({ activeRuns: 0, backgroundCommands: 0, terminals: 1 })).toBe(
      'Restart while terminals run commands?',
    );
  });

  it('count the share of a download in whole percent, rounded down', () => {
    expect(downloadedPercent(0, 200)).toBe(0);
    expect(downloadedPercent(50, 200)).toBe(25);
    expect(downloadedPercent(199, 200)).toBe(99);
    expect(downloadedPercent(200, 200)).toBe(100);
  });

  it('show a download that brought more than its named size as complete', () => {
    expect(downloadedPercent(300, 200)).toBe(100);
  });

  it('know no share of a download whose server names no size', () => {
    expect(downloadedPercent(50, null)).toBeNull();
    expect(downloadedPercent(50, 0)).toBeNull();
  });

  it('show how much of a download arrived as a percent of its size', () => {
    expect(downloadedAmount(50, 200)).toBe(shares.format(0.25));
    expect(downloadedAmount(200, 200)).toBe(shares.format(1));
  });

  it('show how much of a download arrived in megabytes when the server names no size', () => {
    expect(downloadedAmount(2_500_000, null)).toBe(megabytes.format(2.5));
    expect(downloadedAmount(0, null)).toBe(megabytes.format(0));
  });

  it('show the time of a check with its date', () => {
    const morning = Date.UTC(2026, 10, 10, 9, 30);
    const nextDay = Date.UTC(2026, 10, 11, 9, 30);

    expect(checkTime(morning)).toBe(times.format(morning));
    expect(checkTime(morning)).not.toBe(checkTime(nextDay));
  });
});
