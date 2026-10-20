// Chat end to end (docs/features/chat/README.md, acceptance): a reply streams
// from a provider into the conversation view, the conversation list follows,
// the owner renames and deletes conversations, and the views work from the
// keyboard and pass the WCAG 2.1 AA checks.
import { Key } from 'webdriverio';

import {
  addProfile,
  chooseConversationAction,
  clickWithText,
  COMPOSER,
  latestTurnParts,
  listedConversations,
  send,
  sendMessage,
  startNewConversation,
  TEST_CREDENTIAL,
  turnCount,
  useProfile,
  waitForCompleteReply,
  waitForComposer,
  waitForLatestPart,
  waitForNewTurn,
} from '../support/chat.ts';
import { deleteProviderProfiles } from '../support/harness.ts';
import {
  axeViolations,
  focusedElement,
  leaveSettings,
  waitForAnimations,
  waitForApp,
  waitForShownText,
} from '../support/page.ts';
import { MODELS, type ProviderServer, type RecordedRequest, startProviderServer } from '../support/provider-server.ts';

/** The calls of the model, without the model lists and the server probes. */
function modelCalls(provider: ProviderServer): RecordedRequest[] {
  return provider.requests().filter((request) => request.method === 'POST');
}

describe('chat', () => {
  let provider: ProviderServer;

  before(async () => {
    provider = await startProviderServer();
    await waitForApp();
  });

  after(async () => {
    // The profiles' credentials live in the system's credential store.
    await deleteProviderProfiles();
    await provider.close();
  });

  it('opens into a new conversation that asks for a provider profile first', async () => {
    await expect($('main [data-slot="alert"]')).toBeDisplayed();
    await expect($(COMPOSER)).toBeDisabled();
    expect(await axeViolations()).toEqual([]);
  });

  it('leads from the notice to the provider settings page and back', async () => {
    await clickWithText('main [data-slot="alert"] button', 'Open the provider settings');

    await expect($('[data-slot="sidebar"] a[href$="?section=chat.providers"]')).toHaveAttribute('aria-current', 'page');
    await waitForShownText($('main'), expect.stringContaining('No provider profiles yet'));

    await leaveSettings();

    await expect($('main [data-slot="alert"]')).toBeDisplayed();
  });

  it('streams a reply into a new conversation, which the list then shows', async () => {
    await addProfile(provider.baseUrl, { model: MODELS.echo });

    await sendMessage('Hello from the test');

    expect(await waitForCompleteReply()).toBe('Reply to: Hello from the test');
    // The conversation's own pane replaced the new conversation's, and its
    // message box took the focus over.
    expect(await focusedElement()).toMatchObject({ tag: 'textarea' });
    expect(await listedConversations()).toEqual(['Hello from the test']);
    const call = modelCalls(provider).at(-1);
    expect(call?.path).toBe('/v1/chat/completions');
    expect(call?.authorization).toBe(`Bearer ${TEST_CREDENTIAL}`);
  });

  it('sends the earlier turns with the next message', async () => {
    await sendMessage('Second message');

    expect(await waitForCompleteReply()).toBe('Reply to: Second message');
    expect(await turnCount()).toBe(2);
    const messages = modelCalls(provider).at(-1)?.body?.messages as { content: string; role: string }[];
    expect(messages.map((message) => message.role)).toEqual(['system', 'user', 'assistant', 'user']);
    expect(messages.at(-2)?.content).toBe('Reply to: Hello from the test');
  });

  it('renames the conversation', async () => {
    await chooseConversationAction('Rename');
    const title = await $('#rename-conversation-title');
    await title.waitForDisplayed();
    await waitForAnimations();
    await title.setValue('Renamed by the test');
    await browser.keys(Key.Enter);

    await browser.waitUntil(async () => (await listedConversations()).includes('Renamed by the test'), {
      timeoutMsg: 'the list did not show the new title',
    });
    await waitForShownText($('h1'), 'Renamed by the test');
  });

  it('refuses an empty title and says why', async () => {
    await chooseConversationAction('Rename');
    const title = await $('#rename-conversation-title');
    await title.waitForDisplayed();
    await waitForAnimations();
    // WebDriver's clear fires no input event, which the field's binding needs.
    await title.click();
    await browser.keys([Key.Ctrl, 'a']);
    await browser.keys(Key.Backspace);
    await browser.keys(Key.Enter);

    await expect($('[data-slot="field-error"]')).toBeDisplayed();
    expect(await axeViolations()).toEqual([]);
    await browser.keys(Key.Escape);
    await title.waitForDisplayed({ reverse: true });
  });

  it('keeps conversations apart, the most recent first', async () => {
    await startNewConversation();
    await sendMessage('A second conversation');
    expect(await waitForCompleteReply()).toBe('Reply to: A second conversation');

    expect(await listedConversations()).toEqual(['A second conversation', 'Renamed by the test']);
    const messages = modelCalls(provider).at(-1)?.body?.messages as { role: string }[];
    expect(messages.map((message) => message.role)).toEqual(['system', 'user']);
  });

  it('deletes a conversation after the owner confirms', async () => {
    await chooseConversationAction('Delete');
    const dialog = await $('[data-slot="alert-dialog-content"]');
    await dialog.waitForDisplayed();
    await waitForAnimations();
    await clickWithText('[data-slot="alert-dialog-content"] button', 'Delete');

    await browser.waitUntil(async () => (await listedConversations()).length === 1, {
      timeoutMsg: 'the deleted conversation stayed in the list',
    });
    expect(await listedConversations()).toEqual(['Renamed by the test']);
    // Deleting the open conversation leaves the view on a new one.
    expect(await turnCount()).toBe(0);
  });

  it('opens a conversation from the list with its whole transcript', async () => {
    await clickWithText('[data-slot="sidebar"] a[href*="conversation="]', 'Renamed by the test');

    await browser.waitUntil(async () => (await turnCount()) === 2, { timeoutMsg: 'the transcript did not load' });
    const parts = await latestTurnParts();
    expect(parts.map((part) => part.kind)).toEqual(['user_text', 'text']);
    // Only a conversation that the first message just opened takes the focus.
    await waitForComposer();
    expect(await focusedElement()).not.toMatchObject({ tag: 'textarea' });
  });

  it('sends with Enter from the keyboard alone, and Shift+Enter starts a new line', async () => {
    await waitForComposer();
    await browser.execute((selector: string) => {
      document.querySelector<HTMLTextAreaElement>(selector)?.focus();
    }, COMPOSER);
    await browser.keys('First line'.split(''));
    await browser.keys([Key.Shift, Key.Enter]);
    await browser.keys('second line'.split(''));
    expect(await focusedElement()).toMatchObject({ tag: 'textarea' });
    const before = await turnCount();

    await browser.keys(Key.Enter);

    // The new turn shows first, so the latest turn is the one just sent.
    await waitForNewTurn(before);
    const owner = await waitForLatestPart('user_text', ['complete']);
    expect(owner.text).toBe('First line\nsecond line');
    expect(await waitForCompleteReply()).toBe('Reply to: First line\nsecond line');
  });

  it('switches the conversation to another profile, and shows its reasoning apart from the answer', async () => {
    await addProfile(provider.baseUrl, { interface: 'responses', model: MODELS.reasoning, name: 'Reasoning' });
    await useProfile('Reasoning');

    await sendMessage('Think first');

    const reasoning = await waitForLatestPart('reasoning', ['complete']);
    expect(reasoning.text).toContain('Thinking about the question.');
    expect(await waitForCompleteReply()).toBe('The answer after reasoning.');
    const call = modelCalls(provider).at(-1);
    expect(call?.path).toBe('/v1/responses');
    // The earlier turns, which another profile wrote, still go to the model.
    expect(JSON.stringify(call?.body?.input)).toContain('Reply to: Hello from the test');
  });

  it('sends the encrypted reasoning back to the model that wrote it', async () => {
    // The reply repeats the previous one, so the new turn must show first.
    await send('Think again');

    expect(await waitForCompleteReply()).toBe('The answer after reasoning.');
    const input = modelCalls(provider).at(-1)?.body?.input as { encrypted_content?: string; type?: string }[];
    expect(input.filter((item) => item.type === 'reasoning').map((item) => item.encrypted_content)).toEqual([
      'encrypted-rs_e2e_1',
    ]);
  });

  it('renames a conversation from the keyboard alone', async () => {
    // As if the owner had tabbed to the conversation's menu.
    await browser.execute(() => {
      document.querySelector<HTMLButtonElement>('main button[aria-label^="More actions for"]')?.focus();
    });
    await browser.keys(Key.Enter);
    await $('[data-slot="dropdown-menu-content"]').waitForDisplayed();
    // The first item, Rename, has the focus when the menu opens from the keyboard.
    await browser.keys(Key.Enter);
    const title = await $('#rename-conversation-title');
    await title.waitForDisplayed();
    await waitForAnimations();
    expect(await focusedElement()).toMatchObject({ tag: 'input' });
    await browser.keys([Key.Ctrl, 'a']);
    await browser.keys('Renamed from the keyboard'.split(''));
    await browser.keys(Key.Enter);

    await title.waitForDisplayed({ reverse: true });
    await browser.waitUntil(async () => (await listedConversations()).includes('Renamed from the keyboard'), {
      timeoutMsg: 'the list did not show the title typed from the keyboard',
    });
  });

  it('passes the WCAG 2.1 AA checks with a conversation open, in both themes', async () => {
    for (const dark of [false, true]) {
      await browser.execute((isDark: boolean) => {
        document.documentElement.classList.toggle('dark', isDark);
      }, dark);
      expect(await axeViolations()).toEqual([]);
    }
  });
});
