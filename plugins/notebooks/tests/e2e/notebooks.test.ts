// Notebooks in the app (plugins/notebooks/docs/README.md, acceptance): once it
// is on, the activity bar opens Notebooks, which offers "New notebook" while
// there is none; search profiles for the test provider's embedding and rerank
// models are added in the Providers settings and chosen in the Notebooks
// settings; a notebook takes a Markdown file and shows it ready; a chat of
// the notebook shows under it and not in the Chat list, and its reply cites
// the passage by a link that opens the viewer at it; "@" in a regular chat
// attaches the notebook, whose tool a later reply calls; and turning
// Notebooks off leaves the chat's parts readable. The steps pass axe in both
// themes.
import { mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
  addProfile,
  clickWithText,
  COMPOSER,
  send,
  startNewConversation,
  useProfile,
  waitForCompleteReply,
  waitForComposer,
} from '#e2e/chat.ts';
import { deleteProviderProfiles, setFileChoice } from '#e2e/harness.ts';
import {
  axeViolations,
  leaveSettings,
  openSettings,
  setPluginEnabled,
  waitForApp,
  waitForShownText,
} from '#e2e/page.ts';
import { CITED_REPLY, MODELS, type ProviderServer, startProviderServer } from '#e2e/provider-server.ts';
import { Key } from 'webdriverio';

/** The Notebooks entry of the activity bar, found by its address. */
const NOTEBOOKS_ENTRY = '[data-shell-sidebar="bar"] nav a[href*="view=notebooks"]';

/**
 * The entries of the activity bar. An entry's address is where the main area
 * last was under it, which can be the chat view under Notebooks, so the Chat
 * entry is found by its label.
 */
const BAR_ENTRIES = '[data-shell-sidebar="bar"] nav a';

/** The rows of the Chat activity's conversations. */
const CHAT_ROWS = '[data-shell-sidebar="content"] a[href*="conversation="]';

/** The passage that the message asks about, in the first section of the file. */
const PASSAGE = 'A convex lens brings parallel rays to a focus.';

/** A Markdown file of two sections, of which only the first shares words with the message. */
const OPTICS = `# Optics\n\n${PASSAGE}\n\n# Sound\n\nSound waves travel through the air as changes of pressure.\n`;

/** The message, whose words the first section holds and the second does not. */
const QUESTION = 'convex lens focus';

/** The label of the first section's cite link, as the core writes it. */
const OPTICS_LABEL = 'optics.md › Optics';

/** The message of the regular chat that names the notebook, which is also its title. */
const NAMED = `@Physics ${QUESTION}`;

/** Runs axe in the light and the dark theme. */
async function axeInBothThemes(): Promise<void> {
  for (const dark of [false, true]) {
    await browser.execute((isDark: boolean) => {
      document.documentElement.classList.toggle('dark', isDark);
    }, dark);
    expect(await axeViolations()).toEqual([]);
  }
  await browser.execute(() => {
    document.documentElement.classList.remove('dark');
  });
}

/** Adds a search profile of the test provider through the Providers settings. */
async function addSearchProfile(
  baseUrl: string,
  kind: 'embeddings' | 'rerank',
  model: string,
  name: string,
): Promise<void> {
  await clickWithText('button', 'Add a search profile');
  await $('#search-profile-name').waitForDisplayed();
  await $('#search-profile-name').setValue(name);
  await $(`#search-profile-kind-${kind}`).click();
  await $('#search-profile-base-url').setValue(baseUrl);
  await $('#search-profile-model').setValue(model);
  await clickWithText('[role="dialog"] button[type="submit"]', 'Save');
  await $('#search-profile-name').waitForDisplayed({ reverse: true });
  await waitForShownText($('[data-search-profiles]'), expect.stringContaining(name));
}

/** The labels of the Chat activity's conversations. */
async function chatTitles(): Promise<string[]> {
  return browser.execute(
    (selector: string) => [...document.querySelectorAll(selector)].map((row) => row.textContent.trim()),
    CHAT_ROWS,
  );
}

/** The newest request to the model, as JSON text. */
function latestModelRequest(provider: ProviderServer): string {
  const asked = provider.requests().filter((request) => request.path.endsWith('/chat/completions'));
  return JSON.stringify(asked.at(-1)?.body ?? null);
}

describe('the notebooks plugin', () => {
  let provider: ProviderServer;
  let folder: string;

  before(async () => {
    provider = await startProviderServer();
    folder = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-e2e-notebooks-')));
    writeFileSync(join(folder, 'optics.md'), OPTICS);
    await waitForApp();
    await addProfile(provider.baseUrl, { model: MODELS.cites, name: 'Cites' });
    await addProfile(provider.baseUrl, { model: MODELS.searches, name: 'Searches', useTools: true });
    await setPluginEnabled('notebooks', true);
    await leaveSettings();
  });

  after(async () => {
    await setPluginEnabled('notebooks', false);
    await leaveSettings();
    await deleteProviderProfiles();
    await provider.close();
    rmSync(folder, { force: true, maxRetries: 20, recursive: true, retryDelay: 250 });
  });

  it('opens Notebooks from the activity bar, which offers New notebook while there is none', async () => {
    await $(NOTEBOOKS_ENTRY).click();

    await $('[data-notebooks-none]').waitForDisplayed();
    await expect($(NOTEBOOKS_ENTRY)).toHaveAttribute('aria-current', 'page');
    await expect($('[data-notebooks-panel]')).toBeDisplayed();
    await axeInBothThemes();
  });

  it('adds search profiles for the embedding and rerank models in the Providers settings', async () => {
    await openSettings('chat.providers');

    await addSearchProfile(provider.baseUrl, 'embeddings', MODELS.embeddings, 'Test embeddings');
    await addSearchProfile(provider.baseUrl, 'rerank', MODELS.rerank, 'Test reranker');

    await expect($$('[data-search-profiles] > li')).toBeElementsArrayOfSize(2);
    await axeInBothThemes();
  });

  it('chooses them in the Notebooks settings', async () => {
    await openSettings('notebooks.settings');
    await $('[data-notebooks-embedding]').waitForDisplayed();

    await $('[data-notebooks-embedding]').click();
    await clickWithText('[data-slot="select-item"]', `Test embeddings (${MODELS.embeddings})`);
    await $('[data-notebooks-rerank]').click();
    await clickWithText('[data-slot="select-item"]', `Test reranker (${MODELS.rerank})`);
    await $('[data-notebooks-save-settings]').click();

    await $('[data-notebooks-settings-saved]').waitForDisplayed();
    await axeInBothThemes();
  });

  it('creates a notebook that takes a Markdown file and shows it ready', async () => {
    await leaveSettings();
    await $(NOTEBOOKS_ENTRY).click();
    await $('[data-notebooks-new]').click();
    await $('#notebooks-name').waitForDisplayed();
    await $('#notebooks-name').setValue('Physics');
    await browser.keys(Key.Enter);
    await $('[data-notebooks-page]').waitForDisplayed();

    await setFileChoice([join(folder, 'optics.md')]);
    await $('[data-notebooks-add-sources]').click();

    await $('[data-notebooks-state="ready"]').waitForDisplayed({ timeout: 30_000 });
    await waitForShownText($('[data-notebooks-sources]'), expect.stringContaining('optics.md'));
    await waitForShownText($('[data-notebooks-panel]'), expect.stringContaining('Physics'));
    await axeInBothThemes();
  });

  it('starts a chat of the notebook, whose reply cites the passage by a link that opens the viewer at it', async () => {
    await $('[data-notebooks-page] [data-notebooks-new-chat]').click();
    await waitForComposer();
    await waitForShownText($('[data-attachment-chips] [data-home]'), expect.stringContaining('Physics'));

    await send(QUESTION);
    const reply = await waitForCompleteReply();

    expect(reply).toContain(CITED_REPLY);
    await waitForShownText($('[data-citation]'), OPTICS_LABEL);
    await waitForShownText($('[data-context]'), expect.stringContaining('Searched Physics'));
    expect(latestModelRequest(provider)).toContain(PASSAGE);
    expect(provider.requests().some((request) => request.path.endsWith('/rerank'))).toBe(true);
    // The chat belongs to the notebook: it shows under it, not in the Chat list.
    await waitForShownText($('[data-notebooks-panel-chat]'), QUESTION);

    await $('[data-citation]').click();
    await clickWithText('[data-slot="popover-content"] button', 'Open source');
    await $('[data-notebooks-source-dialog] [data-notebooks-passage]').waitForDisplayed();
    await waitForShownText($('[data-notebooks-passage]'), PASSAGE);
    await axeInBothThemes();
    await browser.keys(Key.Escape);
    await $('[data-notebooks-source-dialog]').waitForDisplayed({ reverse: true });

    await clickWithText(BAR_ENTRIES, 'Chat');
    expect(await chatTitles()).not.toContain(QUESTION);
  });

  it('attaches the notebook with "@" in a regular chat, and a later reply searches it with the tool', async () => {
    await clickWithText(BAR_ENTRIES, 'Chat');
    await startNewConversation();
    await $(COMPOSER).click();
    await browser.keys('@Phys');
    await clickWithText('[data-mention-list] [role="option"] [data-content]', 'Physics');
    await browser.keys(QUESTION);
    await browser.keys(Key.Enter);
    const named = await waitForCompleteReply();

    // The message that names the notebook attaches it and gets its passages.
    expect(named).toContain(CITED_REPLY);
    await waitForShownText($('[data-citation]'), OPTICS_LABEL);
    await waitForShownText($('[data-attachment-chips]'), expect.stringContaining('Physics'));
    await expect($('[data-attachment-chips] [data-home]')).not.toBeExisting();
    expect(await chatTitles()).toContain(NAMED);

    // A later message gathers nothing, and the model searches with the tool.
    await useProfile('Searches');
    await send('and how is it focused?');
    const later = await waitForCompleteReply();

    expect(later).toContain(CITED_REPLY);
    const request = latestModelRequest(provider);
    expect(request).toContain('search_notebooks');
    expect(request).toContain(PASSAGE);
    await axeInBothThemes();
  });

  it("keeps the chat's parts readable once Notebooks is off", async () => {
    await setPluginEnabled('notebooks', false);
    await leaveSettings();
    await clickWithText(CHAT_ROWS, NAMED);

    // The core words the part while its plugin is off, from what it stored.
    await waitForShownText($('[data-context]'), expect.stringContaining('Searched Physics'));
    // Both replies cite, and their citations stay buttons with the stored excerpts.
    await expect($$('[data-citation]')).toBeElementsArrayOfSize(2);
    await expect($$('[data-citation]')[0]).toBeDisplayed();
    await expect($(NOTEBOOKS_ENTRY)).not.toBeExisting();
    await axeInBothThemes();
  });
});
