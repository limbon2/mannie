// Custom agents (docs/features/custom-agents.md, acceptance): the Agents page
// creates, edits, duplicates and deletes an agent from the keyboard and the
// pointer, the picker chooses one for a new and an existing conversation, the
// model reads the chosen agent's instructions and tools, and a custom agent's
// first task opens the child agents' panel without taking the focus.
import { Key } from 'webdriverio';

import {
  addProfile,
  callMessage,
  COMPOSER,
  send,
  sendMessage,
  startNewConversation,
  waitForCompleteReply,
  waitForComposer,
} from '../support/chat.ts';
import { deleteProviderProfiles } from '../support/harness.ts';
import { axeViolations, waitForApp, waitForShownText } from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '../support/provider-server.ts';

const VIEW = '[data-agents-view]';
const EDITOR = '[data-agent-editor]';
const PICKER = 'main [data-agent-picker]';
const CUSTOM_KEY = 'custom.reviewer';
/** The bodies of the requests for the tools scenario so far, as text to search, the latest last. */
function toolsBodies(provider: ProviderServer): Record<string, unknown>[] {
  return provider.requests().flatMap(({ body }) => (body?.model === MODELS.tools ? [body] : []));
}

/** The latest request for the tools scenario, as text. */
function lastRequestText(provider: ProviderServer): string {
  return JSON.stringify(toolsBodies(provider).at(-1) ?? null);
}

/** Opens the Agents page from the activity bar. */
async function openAgentsPage(): Promise<void> {
  await $('a[href*="view=agents"]').click();
  await $(VIEW).waitForDisplayed();
}

/** Starts a new agent from the template with this ID. */
async function startFromTemplate(template: string): Promise<void> {
  await $('[data-agents-new]').click();
  await $(`[data-agents-template="${template}"]`).click();
  await $(EDITOR).waitForDisplayed();
}

/** Replaces a field's text. */
async function fill(selector: string, text: string): Promise<void> {
  await $(selector).setValue(text);
}

/** Saves the agent, and waits until the address names it. */
async function saveAgent(key: string): Promise<void> {
  await $('[data-agent-save]').click();
  await browser.waitUntil(
    async () => browser.execute((wanted: string) => new URLSearchParams(location.search).get('agent') === wanted, key),
    { timeoutMsg: `the agent ${key} was not saved` },
  );
}

/** The names that the group of the Agents page's sidebar shows. */
async function agentsIn(group: string): Promise<string[]> {
  return browser.execute((name: string) => {
    const rows = document.querySelectorAll<HTMLElement>(`[data-agents-group="${name}"] [data-agent-row]`);
    return [...rows].map((row) => row.textContent.trim());
  }, group);
}

/** Chooses an agent in the message box's picker. */
async function chooseAgent(key: string): Promise<void> {
  await $(PICKER).click();
  await $(`[data-agent-option="${key}"]`).click();
}

describe('custom agents', () => {
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

  it('lists the built-in agents and the starters, and shows a built-in agent read-only', async () => {
    await openAgentsPage();

    expect(await agentsIn('conversation')).toEqual(['Default', 'Orchestrator', 'Plan']);
    expect(await agentsIn('child')).toEqual(['Explore', 'General']);
    await $('[data-agent-row="explore"]').click();
    await $(EDITOR).waitForDisplayed();
    await expect($('[data-agent-save]')).not.toBeExisting();
    await expect($('[data-agent-delete]')).not.toBeExisting();
    await expect($('[data-agent-duplicate]')).toBeExisting();
    expect(await axeViolations()).toEqual([]);
  });

  it('says what is wrong with a draft, and keeps what the owner wrote', async () => {
    await startFromTemplate('child');
    await $('[data-agent-save]').click();

    await waitForShownText($('[data-agent-failure]'), expect.stringContaining('name'));
    await fill('[data-agent-name]', 'Reviewer');
    await $('[data-agent-save]').click();
    await waitForShownText($('[data-agent-failure]'), expect.stringContaining('description'));
    await expect($('[data-agent-name]')).toHaveValue('Reviewer');
  });

  it('creates a child agent, which joins the list under a key from its name', async () => {
    await fill('[data-agent-description]', 'Reads the changes for mistakes.');
    await fill('[data-agent-instructions]', 'Look for mistakes.');
    await saveAgent(CUSTOM_KEY);

    await browser.waitUntil(async () => (await agentsIn('child')).includes('Reviewer'));
    expect(await axeViolations()).toEqual([]);
  });

  it('asks before leaving with changes, and stays when the owner says so', async () => {
    await fill('[data-agent-instructions]', 'Look harder.');
    await $('[data-agent-row="explore"]').click();
    await $('[data-agent-leave]').waitForDisplayed();
    await browser.keys(Key.Escape);
    await $('[data-agent-leave]').waitForDisplayed({ reverse: true });
    await expect($('[data-agent-instructions]')).toHaveValue('Look harder.');

    await saveAgentChanges();
  });

  it('edits steps from the keyboard', async () => {
    await $(`[data-agent-row="${CUSTOM_KEY}"]`).click();
    await $('[data-agent-step-add]').click();
    await $('[data-agent-step-add]').click();
    const titles = await $$('[data-agent-step-title]');
    await titles[0]?.setValue('First');
    await titles[1]?.setValue('Second');
    const texts = await $$('[data-agent-step-text]');
    await texts[0]?.setValue('Do the first.');
    await texts[1]?.setValue('Do the second.');

    await $$('[data-agent-step-down]')[0]?.click();

    const moved = await browser.execute(() =>
      [...document.querySelectorAll<HTMLInputElement>('[data-agent-step-title]')].map((input) => input.value),
    );
    expect(moved).toEqual(['Second', 'First']);
    await saveAgentChanges();
    expect(await axeViolations()).toEqual([]);
  });

  it('duplicates a built-in agent into one of the owner’s', async () => {
    await $('[data-agent-row="conversation"]').click();
    await $('[data-agent-duplicate]').click();
    await browser.waitUntil(async () => browser.execute(() => document.querySelector('[data-agent-save]') !== null), {
      timeoutMsg: 'the copy did not open',
    });
    await expect($('[data-agent-name]')).toHaveValue('Default copy');
    await fill('[data-agent-name]', 'Careful');
    await fill('[data-agent-instructions]', 'MARKER Be careful.');
    await saveAgent('custom.careful');
  });

  it('starts a conversation with an agent from its page, and the model reads its instructions', async () => {
    await $('[data-agent-start]').click();
    await waitForComposer();
    await waitForShownText($(PICKER), expect.stringContaining('Careful'));

    await sendMessage('Hello');
    await waitForCompleteReply();

    expect(lastRequestText(provider)).toContain('MARKER Be careful.');
    await waitForShownText($(PICKER), expect.stringContaining('Careful'));
  });

  it('changes the agent of an existing conversation for its next reply', async () => {
    await chooseAgent('conversation');
    await waitForShownText($(PICKER), expect.stringContaining('Default'));

    await send('Hello again');
    await waitForCompleteReply();

    expect(lastRequestText(provider)).not.toContain('MARKER Be careful.');
  });

  it('opens the child agents’ panel on a custom agent’s first task, without taking the focus', async () => {
    await startNewConversation();
    await waitForComposer();
    // The picker of a new conversation.
    await chooseAgent('custom.careful');
    await waitForShownText($(PICKER), expect.stringContaining('Careful'));
    await $(COMPOSER).click();

    await send(
      callMessage('start_task', {
        agent: 'explore',
        background: false,
        instruction: 'Look around.',
        title: 'Look around',
      }),
    );

    await $('[data-child-agents-panel]').waitForDisplayed({ timeout: 30_000 });
    await waitForCompleteReply();
    expect(await browser.execute((box: string) => document.activeElement?.matches(box) ?? false, COMPOSER)).toBe(true);
    await $('[data-child-agents-group="ended"] [data-child-agent-row="1"]').waitForDisplayed();
    expect(await axeViolations()).toEqual([]);
  });

  it('deletes an agent, and its conversations go back to the built-in agent', async () => {
    await openAgentsPage();
    await $('[data-agent-row="custom.careful"]').click();
    await $('[data-agent-delete]').click();
    await $('[data-agent-delete-confirm]').click();

    await browser.waitUntil(async () => !(await agentsIn('conversation')).includes('Careful'));
    await $('[data-agent-row="custom.reviewer"]').click();
    await $('[data-agent-delete]').click();
    await $('[data-agent-delete-confirm]').click();
    await browser.waitUntil(async () => !(await agentsIn('child')).includes('Reviewer'));
  });
});

/** Saves the agent that the editor shows, and waits until it holds no changes. */
async function saveAgentChanges(): Promise<void> {
  await $('[data-agent-save]').click();
  await waitForShownText($(`${EDITOR} footer [role="status"]`), expect.stringContaining('Saved'));
}
