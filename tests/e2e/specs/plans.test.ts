// Plans (docs/features/plans.md, acceptance): the Plan agent saves a plan with
// `save_plan`, a card shows it, the side panel shows, edits and deletes it, the
// sidebar lists it, and both ways to run it fill a message box and send nothing
// until the owner presses Send.
import { Key } from 'webdriverio';

import {
  addProfile,
  callMessage,
  COMPOSER,
  LATEST_MAIN_TURN,
  send,
  startNewConversation,
  turnCount,
  waitForCompleteReply,
  waitForComposer,
} from '../support/chat.ts';
import { deleteProviderProfiles } from '../support/harness.ts';
import { axeViolations, waitForApp, waitForShownText } from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '../support/provider-server.ts';

const PICKER = 'main [data-agent-picker]';
const CARD = `${LATEST_MAIN_TURN} [data-plan-card]`;
const PANEL = '[data-plan-panel]';
const PLAN_BODY = '# Steps\n\n1. Add the switch.\n2. Add the tests.\n';
/** The bodies of the requests for the tools scenario so far, as text to search, the latest last. */
function toolsBodies(provider: ProviderServer): Record<string, unknown>[] {
  return provider.requests().flatMap(({ body }) => (body?.model === MODELS.tools ? [body] : []));
}

/** The latest request for the tools scenario, as text. */
function lastRequestText(provider: ProviderServer): string {
  return JSON.stringify(toolsBodies(provider).at(-1) ?? null);
}

/** Chooses an agent in the message box's picker. */
async function chooseAgent(key: string): Promise<void> {
  await $(PICKER).click();
  await $(`[data-agent-option="${key}"]`).click();
}

/** The text of the message box. */
async function composerText(): Promise<string> {
  return browser.execute((box: string) => document.querySelector<HTMLTextAreaElement>(box)?.value ?? '', COMPOSER);
}

/** Waits until the message box holds this text and a plan's token. */
async function waitForFilledBox(sentence: string): Promise<void> {
  await browser.waitUntil(
    async () => {
      const text = await composerText();
      return text.startsWith(sentence) && /@plan-\d+/u.test(text);
    },
    { timeoutMsg: 'the message box did not take the plan' },
  );
}

describe('plans', () => {
  let provider: ProviderServer;

  before(async () => {
    provider = await startProviderServer();
    await waitForApp();
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'Tools', useTools: true });
  });

  after(async () => {
    await deleteProviderProfiles();
    await provider.close();
  });

  it('saves a plan with the Plan agent, offers it no tool that acts, and shows a card', async () => {
    await startNewConversation();
    await waitForComposer();
    await chooseAgent('custom.plan');

    await send(callMessage('save_plan', { body: PLAN_BODY, title: 'Add dark mode' }));
    await waitForCompleteReply();

    const request = lastRequestText(provider);
    expect(request).toContain('"save_plan"');
    expect(request).toContain('"read_file"');
    expect(request).not.toContain('"write_file"');
    expect(request).not.toContain('"run_command"');
    await $(CARD).waitForDisplayed();
    await waitForShownText($(CARD), expect.stringContaining('Add dark mode'));
    expect(await axeViolations()).toEqual([]);
  });

  it('lists the plan in the sidebar, and shows it rendered in a side panel', async () => {
    await $('[data-plans-panel]').waitForDisplayed();

    await $(`${CARD} [data-plan-open]`).click();
    await $(PANEL).waitForDisplayed();
    await waitForShownText($('[data-plan-body]'), expect.stringContaining('Add the switch.'));
    // The plan's text renders as Markdown, whose heading is a heading.
    expect(await browser.execute(() => document.querySelector('[data-plan-body] :is(h1, h2, h3, h4)') !== null)).toBe(
      true,
    );
    expect(await axeViolations()).toEqual([]);
  });

  it('edits a plan', async () => {
    await $('[data-plan-edit-start]').click();
    await $('[data-plan-edit-body]').setValue("# Steps\n\n1. The owner's step.\n");
    await $('[data-plan-save]').click();

    await waitForShownText($('[data-plan-body]'), expect.stringContaining('The owner'));
  });

  it('runs a plan in a new conversation, and sends nothing before Send', async () => {
    await $(`${PANEL} [data-plan-run-new]`).click();
    await waitForFilledBox('Carry out this plan.');
    const requests = provider.requests().length;

    expect(await turnCount()).toBe(0);
    expect(provider.requests()).toHaveLength(requests);
    await waitForShownText($(PICKER), expect.stringContaining('Orchestrator'));

    await chooseAgent('conversation');
    await $(COMPOSER).click();
    await browser.keys(Key.Enter);
    await waitForCompleteReply();

    const said = lastRequestText(provider);
    expect(said).toContain('The owner');
    expect(said).toContain('A planning agent wrote it');
  });

  it('runs a plan in its own conversation, whose agent changes at once', async () => {
    await $('[data-plans-panel] [data-plan-row]').click();
    await $(PANEL).waitForDisplayed();
    const requests = provider.requests().length;

    await $(`${PANEL} [data-plan-run-here]`).click();
    await waitForFilledBox('Carry out this plan.');

    await waitForShownText($(PICKER), expect.stringContaining('Orchestrator'));
    expect(provider.requests()).toHaveLength(requests);
  });

  it('deletes a plan, and the sidebar lets it go', async () => {
    await $('[data-plans-panel] [data-plan-row]').click();
    await $(PANEL).waitForDisplayed();
    await $(`${PANEL} [data-plan-delete]`).click();
    await $('[data-plan-delete-confirm]').click();

    await $('[data-plans-panel]').waitForExist({ reverse: true });
  });
});
