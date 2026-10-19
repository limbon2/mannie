// Images in the message box and the transcript (plugins/images/docs/README.md,
// acceptance): while Images is off, nothing takes an image; turned on, the
// button, a paste and a drop add images, which the core shrinks before the
// model's request holds them, for Chat Completions and the Responses API; a
// paste that carries text stays a text paste; an eleventh image and a file
// that is not an image are refused; the transcript shows thumbnails that open
// at full size; a model without images fails with the provider's error, and a
// retry sends the image again; and turning Images off keeps the images of
// earlier messages. Axe finds nothing with the chips and the full image open.
import { mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
  addProfile,
  clickInLatestTurn,
  clickWithText,
  COMPOSER,
  listedConversations,
  MAIN_TURNS,
  send,
  startNewConversation,
  turnCount,
  useProfile,
  waitForCompleteReply,
  waitForComposer,
  waitForLatestPart,
  waitForNewTurn,
} from '#e2e/chat.ts';
import { deleteProviderProfiles, setFileChoice } from '#e2e/harness.ts';
import { axeViolations, leaveSettings, openPluginUses, openSettings, setPluginEnabled, waitForApp } from '#e2e/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '#e2e/provider-server.ts';
import { Key } from 'webdriverio';

import {
  dropFiles,
  imageFailures,
  imageUrls,
  pasteFiles,
  pngFile,
  pngSize,
  readyChips,
  requestImages,
  solidPng,
  waitForReadyChips,
} from './support/images.ts';

/** The Images plugin's button in the message box. */
const ATTACH = '[data-attach-images]';

/** Sends what the message box holds with Enter, and waits until its turn shows. */
async function sendWithEnter(): Promise<void> {
  await waitForComposer();
  const before = await turnCount();
  await $(COMPOSER).click();
  await browser.keys(Key.Enter);
  await waitForNewTurn(before);
}

/** The thumbnails that the latest turn shows, once loaded. */
async function shownThumbnails(): Promise<number> {
  return browser.execute((turns: string) => {
    const all = document.querySelectorAll(turns);
    const latest = all.item(all.length - 1) as Element | null;
    return latest?.querySelectorAll('[data-image-part] button img[src^="data:image/"]').length ?? 0;
  }, MAIN_TURNS);
}

/** Waits until the latest turn shows `count` thumbnails. */
async function waitForThumbnails(count: number): Promise<void> {
  await browser.waitUntil(async () => (await shownThumbnails()) === count, {
    timeout: 30_000,
    timeoutMsg: `the latest turn did not show ${String(count)} thumbnails`,
  });
}

describe('images', () => {
  let provider: ProviderServer;
  let folder: string;

  before(async () => {
    provider = await startProviderServer();
    folder = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-e2e-images-')));
    writeFileSync(join(folder, 'wide.png'), solidPng(3_000, 1_000, [200, 60, 60]));
    writeFileSync(join(folder, 'small.png'), solidPng(40, 30, [60, 200, 60]));
    await waitForApp();
    await addProfile(provider.baseUrl, { model: MODELS.echo, name: 'Echo' });
    await addProfile(provider.baseUrl, { interface: 'responses', model: MODELS.echo, name: 'Echo Responses' });
    await addProfile(provider.baseUrl, { model: MODELS.withoutImages, name: 'Without images' });
  });

  after(async () => {
    await setPluginEnabled('images', false);
    await leaveSettings();
    await deleteProviderProfiles();
    await provider.close();
    rmSync(folder, { force: true, maxRetries: 20, recursive: true, retryDelay: 250 });
  });

  it('lists Images off in a new installation, with the file dialog, its commands and the library that it may use', async () => {
    await openSettings('plugins.settings');
    await expect($('[data-plugin="images"]')).toHaveAttribute('data-plugin-state', 'off');

    await openPluginUses('images');

    await expect($('[data-plugin="images"] [data-plugin-service="file-dialog"]')).toBeDisplayed();
    await expect($('[data-plugin="images"] [data-plugin-command="pick_images"]')).toBeDisplayed();
    await expect($('[data-plugin="images"] [data-plugin-crate="image"]')).toBeDisplayed();
    await expect($('[data-plugin="images"] [data-plugin-native]')).not.toBeExisting();
    await leaveSettings();
  });

  it('takes no image while Images is off', async () => {
    await startNewConversation();

    const pasted = await pasteFiles([pngFile('shot.png', 40, 30)]);
    const dropped = await dropFiles('[data-composer]', [pngFile('shot.png', 40, 30)]);

    expect(await $(ATTACH).isExisting()).toBe(false);
    expect(pasted).toBe(false);
    // The shell takes the stray drop, so that the webview opens nothing.
    expect(dropped).toBe(true);
    expect(await readyChips()).toEqual([]);
  });

  it('adds the chosen files with the button, shrinks them for the model and shows their thumbnails', async () => {
    await setPluginEnabled('images', true);
    await startNewConversation();
    await setFileChoice([join(folder, 'wide.png'), join(folder, 'small.png')]);

    await $(ATTACH).click();
    const chips = await waitForReadyChips(2);
    expect(await axeViolations()).toEqual([]);
    await $(COMPOSER).setValue('What do these show?');
    await sendWithEnter();
    await waitForCompleteReply();

    expect(chips).toEqual(['wide.png', 'small.png']);
    expect(requestImages(provider.requests()).map((image) => pngSize(image))).toEqual([
      { height: 683, width: 2_048 },
      { height: 30, width: 40 },
    ]);
    await waitForThumbnails(2);
    expect(await readyChips()).toEqual([]);
  });

  it('opens an image at full size, and Escape closes it', async () => {
    const thumbnails = await $$('[data-image-part] button');
    await thumbnails[0]?.click();
    const dialog = await $('[data-slot="dialog-content"]');
    await dialog.waitForDisplayed();
    await dialog.$('img').waitForExist({ timeout: 10_000 });

    expect(await dialog.$('img').getAttribute('alt')).toBe('wide.png');
    expect(await axeViolations()).toEqual([]);
    await browser.keys(Key.Escape);
    await dialog.waitForDisplayed({ reverse: true });
  });

  it('adds a pasted image, leaves a paste with text to the text, and names a conversation after its image', async () => {
    await startNewConversation();

    const withText = await pasteFiles([pngFile('image.png', 64, 64)], 'A cell of a table');
    const chipsAfterText = await readyChips();
    const alone = await pasteFiles([pngFile('image.png', 64, 64)]);
    await waitForReadyChips(1);
    await sendWithEnter();
    await waitForCompleteReply();

    expect(withText).toBe(false);
    expect(chipsAfterText).toEqual([]);
    expect(alone).toBe(true);
    await browser.waitUntil(async () => (await listedConversations()).includes('image.png'), {
      timeout: 10_000,
      timeoutMsg: 'the conversation did not take the name of its image',
    });
    await waitForThumbnails(1);
  });

  it('adds images dropped on the message box, and a drop elsewhere adds nothing', async () => {
    await startNewConversation();

    const elsewhere = await dropFiles('header h1', [pngFile('elsewhere.png', 8, 8)]);
    const onBox = await dropFiles('[data-composer]', [pngFile('dropped.png', 8, 8)]);
    const chips = await waitForReadyChips(1);
    await $('button[aria-label="Remove dropped.png"]').click();

    expect(elsewhere).toBe(true);
    expect(onBox).toBe(true);
    expect(chips).toEqual(['dropped.png']);
    await browser.waitUntil(async () => (await readyChips()).length === 0, {
      timeoutMsg: 'the removed image stayed',
    });
  });

  it('refuses an eleventh image, a file that is not an image and a damaged one', async () => {
    await startNewConversation();
    const eleven = Array.from({ length: 11 }, (_unused, index) => pngFile(`${String(index)}.png`, 8, 8));

    await pasteFiles(eleven);
    await waitForReadyChips(10);
    const afterEleven = await imageFailures();
    // The message box keeps its draft, so the images go one by one.
    for (const name of await readyChips()) {
      await $(`button[aria-label="Remove ${name}"]`).click();
    }
    await waitForReadyChips(0);
    await pasteFiles([
      { bytes: Buffer.from('plain words'), name: 'notes.txt', type: 'text/plain' },
      { bytes: Buffer.from('not an image'), name: 'fake.png', type: 'image/png' },
    ]);

    expect(afterEleven).toEqual(['A message holds at most 10 images.']);
    await browser.waitUntil(async () => (await imageFailures()).length === 2, {
      timeout: 10_000,
      timeoutMsg: 'the refusals did not show',
    });
    expect(await imageFailures()).toEqual([
      'notes.txt is not an image that can be added to a message.',
      'fake.png is not a PNG, JPEG, GIF or WebP image.',
    ]);
  });

  it('sends an image to the Responses API as an input image, and text alone stays a string', async () => {
    await startNewConversation();
    await send('Hello');
    await waitForCompleteReply();
    const greeting = provider.requests().at(-1)?.body ?? {};
    await useProfile('Echo Responses');

    await pasteFiles([pngFile('blue.png', 20, 10)]);
    await waitForReadyChips(1);
    await sendWithEnter();
    await waitForCompleteReply();
    const responses = provider.requests().filter((request) => request.path === '/v1/responses');
    const urls = imageUrls(responses.at(-1)?.body ?? null);

    expect((greeting.messages as { content: unknown }[]).every((message) => typeof message.content === 'string')).toBe(
      true,
    );
    expect(urls).toHaveLength(1);
    expect(pngSize(Buffer.from(urls[0]?.split(',')[1] ?? '', 'base64'))).toEqual({ height: 10, width: 20 });
  });

  it('shows the provider’s error for a model without images, and a retry sends the image again', async () => {
    await startNewConversation();
    await send('Hello');
    await waitForCompleteReply();
    await useProfile('Without images');

    await pasteFiles([pngFile('red.png', 10, 10)]);
    await waitForReadyChips(1);
    await sendWithEnter();
    const failed = await waitForLatestPart('error', ['complete']);
    const before = provider.requests().length;
    await clickInLatestTurn('Retry');
    await browser.waitUntil(() => provider.requests().length > before, {
      timeout: 10_000,
      timeoutMsg: 'the retry did not reach the provider',
    });
    await waitForLatestPart('error', ['complete']);
    const refused = provider.requests().filter((request) => request.body?.model === MODELS.withoutImages);

    expect(failed.text).toContain('400');
    expect(refused.slice(-2).map((request) => imageUrls(request.body).length)).toEqual([1, 1]);
  });

  it('keeps the images of earlier messages when Images is off', async () => {
    await setPluginEnabled('images', false);
    await leaveSettings();

    await clickWithText('[data-slot="sidebar"] a[href*="conversation="]', 'image.png');
    await waitForThumbnails(1);

    expect(await $(ATTACH).isExisting()).toBe(false);
  });
});
