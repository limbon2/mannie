// Context budget and compaction (docs/features/chat/conversations.md,
// context budget and compaction): when a conversation outgrows its profile's
// context, a checkpoint summary replaces its oldest turns in what the model
// reads, while the owner's transcript stays whole; the owner can also compact
// a conversation at any time, but never its latest turn.
import {
  addProfile,
  chooseContextAction,
  clickWithText,
  send,
  startNewConversation,
  turnCount,
  waitForCompleteReply,
} from '../support/chat.ts';
import { deleteProviderProfiles } from '../support/harness.ts';
import { axeViolations, waitForAnimations, waitForApp, waitForShownText } from '../support/page.ts';
import {
  isSummaryRequest,
  MODELS,
  type ProviderServer,
  type RecordedRequest,
  startProviderServer,
  SUMMARY_PREFIX,
} from '../support/provider-server.ts';

/** A small context window, which a few long replies fill. */
const SMALL_CONTEXT_WINDOW = 4_096;

/** Enough turns to fill the small context several times over. */
const MAX_TURNS = 12;

/** The numbers of the long replies that a request's text contains. */
function longReplyNumbers(request: RecordedRequest | undefined): number[] {
  const text = JSON.stringify(request?.body ?? null);
  return [...text.matchAll(/Long reply (\d+)\./gu)].map((match) => Number(match[1]));
}

function summaryRequests(provider: ProviderServer): RecordedRequest[] {
  return provider.requests().filter((request) => isSummaryRequest(request.body));
}

describe('context budget and compaction', () => {
  let provider: ProviderServer;

  before(async () => {
    provider = await startProviderServer();
    await waitForApp();
    await addProfile(provider.baseUrl, {
      contextWindow: SMALL_CONTEXT_WINDOW,
      model: MODELS.long,
      name: 'Small context',
      outputLimit: 512,
    });
  });

  after(async () => {
    // The profiles' credentials live in the system's credential store.
    await deleteProviderProfiles();
    await provider.close();
  });

  it('summarizes the oldest turns when the context runs low, and keeps the transcript whole', async () => {
    let turns = 0;
    while (summaryRequests(provider).length === 0) {
      turns += 1;
      expect(turns).toBeLessThanOrEqual(MAX_TURNS);
      await send(`Message ${String(turns)}`);
      await waitForCompleteReply();
    }

    const summary = summaryRequests(provider)[0];
    const covered = longReplyNumbers(summary);
    expect(covered.length).toBeGreaterThan(0);
    const after = provider.requests().filter((request) => {
      const isCall = request.method === 'POST' && !isSummaryRequest(request.body);
      const isLater = request.receivedAt >= (summary?.receivedAt ?? 0);
      return isCall && isLater;
    });
    const next = after[0];
    const system = (next?.body?.messages as { content: string; role: string }[])[0];
    expect(system?.content).toContain(SUMMARY_PREFIX);
    // The covered replies left the model's context; the owner's latest message stayed.
    const newestCovered = Math.max(...covered);
    expect(longReplyNumbers(next).every((number) => number > newestCovered)).toBe(true);
    expect(JSON.stringify(next?.body)).toContain(`Message ${String(turns)}`);
    // The owner's transcript still has every turn.
    expect(await turnCount()).toBe(turns);
    await waitForShownText(
      $('main'),
      expect.stringContaining('Messages above this line are summarized for the model.'),
    );
  });

  it('shows the summary that the model reads instead of the covered turns', async () => {
    await clickWithText('main button', 'Show the summary');

    const dialog = await $('[data-slot="dialog-content"]');
    await dialog.waitForDisplayed();
    await waitForAnimations();
    await waitForShownText(dialog, expect.stringContaining(SUMMARY_PREFIX));
    expect(await axeViolations()).toEqual([]);
    await clickWithText('[data-slot="dialog-content"] button', 'Close');
    await dialog.waitForDisplayed({ reverse: true });
  });

  it('says that a conversation with one turn has nothing to compact', async () => {
    await startNewConversation();
    await send('Only one turn');
    await waitForCompleteReply();
    const summariesBefore = summaryRequests(provider).length;

    await chooseContextAction('Compact now');

    await waitForShownText(
      $('main [data-slot="alert"]'),
      expect.stringContaining('There are not enough earlier messages to summarize yet.'),
    );
    expect(summaryRequests(provider)).toHaveLength(summariesBefore);
  });

  it('compacts every turn but the latest when the owner asks', async () => {
    await clickWithText('main [data-slot="alert"] button', 'Dismiss');
    await send('A second turn');
    await waitForCompleteReply();
    const summariesBefore = summaryRequests(provider).length;

    await chooseContextAction('Compact now');

    await browser.waitUntil(() => summaryRequests(provider).length > summariesBefore, {
      timeoutMsg: 'the conversation was not compacted',
    });
    await waitForShownText(
      $('main'),
      expect.stringContaining('Messages above this line are summarized for the model.'),
    );
    const summary = summaryRequests(provider).at(-1);
    expect(JSON.stringify(summary?.body)).toContain('Only one turn');
    expect(JSON.stringify(summary?.body)).not.toContain('A second turn');
  });
});
