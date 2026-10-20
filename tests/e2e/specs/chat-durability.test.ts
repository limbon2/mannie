// Durable replies (docs/features/chat/conversations.md, acceptance; ADR-0012):
// a reply continues while the window reloads and arrives without gaps, Stop
// keeps the text written so far, a retry waits as long as the provider asks,
// a permanent failure says why, and an app that was killed leaves its reply
// interrupted and retryable.
import {
  addProfile,
  COMPOSER,
  latestTurnParts,
  retryLatestReply,
  sendMessage,
  turnCount,
  useProfile,
  waitForCompleteReply,
  waitForLatestPart,
} from '../support/chat.ts';
import { deleteProviderProfiles, processId } from '../support/harness.ts';
import { waitForApp, waitForShownText } from '../support/page.ts';
import {
  MODELS,
  pacedText,
  type ProviderServer,
  RETRY_AFTER_SECONDS,
  startProviderServer,
} from '../support/provider-server.ts';

/** When the owner pressed Escape and when the reply showed as stopped, in the page's clock. */
interface StopTiming {
  keydown: number;
  stopped: number;
}

declare global {
  interface Window {
    stopTiming?: StopTiming;
  }
}

/** Waits until the reply that is being written has at least `words` words. */
async function waitForWords(words: number): Promise<void> {
  await browser.waitUntil(
    async () => {
      const reply = (await latestTurnParts()).find((part) => part.kind === 'text');
      return (reply?.text.split(' ').length ?? 0) >= words;
    },
    { timeout: 15_000, timeoutMsg: `the reply did not reach ${String(words)} words` },
  );
}

describe('durable replies', () => {
  let provider: ProviderServer;

  before(async () => {
    provider = await startProviderServer();
    await waitForApp();
    await addProfile(provider.baseUrl, { model: MODELS.paced, name: 'Paced' });
    await addProfile(provider.baseUrl, { model: MODELS.rateLimited, name: 'Rate limited' });
    await addProfile(provider.baseUrl, { model: MODELS.unauthorized, name: 'Unauthorized' });
  });

  after(async () => {
    // The profiles' credentials live in the system's credential store.
    await deleteProviderProfiles();
    await provider.close();
  });

  it('keeps streaming while the window reloads, and shows the whole reply without gaps', async () => {
    await sendMessage('Count for me');
    await waitForWords(20);

    await browser.refresh();
    await waitForApp();

    expect(await waitForCompleteReply()).toBe(pacedText());
  });

  it('stops a reply within a second and keeps the text written so far', async () => {
    await sendMessage('Count again');
    await waitForWords(10);
    await browser.execute(() => {
      const timing: StopTiming = { keydown: 0, stopped: 0 };
      window.stopTiming = timing;
      document.addEventListener(
        'keydown',
        (event) => {
          if (event.key === 'Escape') {
            timing.keydown = performance.now();
          }
        },
        { capture: true },
      );
      const main = document.querySelector('main');
      if (main === null) {
        throw new Error('the page has no main area');
      }
      const observer = new MutationObserver(() => {
        const turns = main.querySelectorAll('article');
        const reply = turns.item(turns.length - 1).querySelector<HTMLElement>('[data-part-kind="text"]');
        if (reply?.dataset.partState === 'stopped') {
          timing.stopped = performance.now();
          observer.disconnect();
        }
      });
      observer.observe(main, { attributeFilter: ['data-part-state'], attributes: true, subtree: true });
    });

    await $(COMPOSER).click();
    await browser.keys('Escape');

    const stopped = await waitForLatestPart('text', ['stopped'], 5_000);
    const timing = await browser.execute(() => window.stopTiming);
    expect(timing?.stopped).toBeGreaterThan(0);
    expect((timing?.stopped ?? 0) - (timing?.keydown ?? 0)).toBeLessThan(1_000);
    expect(stopped.text).not.toBe('');
    expect(pacedText().startsWith(stopped.text)).toBe(true);
  });

  it('retries a stopped reply, which replaces the stopped text', async () => {
    await retryLatestReply();

    expect(await waitForCompleteReply()).toBe(pacedText());
    expect(await turnCount()).toBe(2);
    expect((await latestTurnParts()).map((part) => part.kind)).toEqual(['user_text', 'text']);
  });

  it('waits as long as the provider asks before it tries again', async () => {
    await useProfile('Rate limited');

    await sendMessage('Try within the limit');

    await waitForShownText($('main'), expect.stringContaining('Trying again in'));
    expect(await waitForCompleteReply()).toBe('Reply to: Try within the limit');
    const calls = provider.requests().filter((request) => request.body?.model === MODELS.rateLimited);
    expect(calls).toHaveLength(2);
    const waited = (calls[1]?.receivedAt ?? 0) - (calls[0]?.receivedAt ?? 0);
    expect(waited).toBeGreaterThanOrEqual(RETRY_AFTER_SECONDS * 1_000 - 100);
  });

  it('says why the provider refused a reply, and offers Retry', async () => {
    await useProfile('Unauthorized');

    await sendMessage('This will fail');

    const error = await waitForLatestPart('error', ['complete']);
    expect(error.text).toContain('The provider refused the API key.');
    await waitForShownText($('main article:last-of-type button'), 'Retry');
    const calls = provider.requests().filter((request) => request.body?.model === MODELS.unauthorized);
    // A refused credential is not tried again.
    expect(calls).toHaveLength(1);
  });

  it('leaves a reply interrupted when the app is killed, and Retry finishes it after a restart', async () => {
    await useProfile('Paced');
    await sendMessage('Count until the end');
    await waitForWords(10);
    const conversationUrl = await browser.getUrl();

    process.kill(await processId(), 'SIGKILL');
    await browser.reloadSession();
    await waitForApp();
    await browser.url(conversationUrl);
    await waitForApp();

    const interrupted = await waitForLatestPart('text', ['interrupted']);
    expect(interrupted.text).not.toBe('');
    expect(pacedText().startsWith(interrupted.text)).toBe(true);
    await waitForShownText($('main'), expect.stringContaining('Mannie quit before this reply finished.'));

    await retryLatestReply();

    expect(await waitForCompleteReply()).toBe(pacedText());
  });
});
