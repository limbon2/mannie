// Provider profiles in the settings (docs/features/chat/providers.md,
// acceptance): the owner adds, tests, edits, chooses the default and deletes
// profiles. The core checks every field, and a stored credential is never
// shown again; the owner replaces it by typing a new one, or removes it.
import { Key } from 'webdriverio';

import { clickWithText, waitForPointerInput } from '../support/chat.ts';
import { deleteProviderProfiles } from '../support/harness.ts';
import { axeViolations, openSettings, waitForAnimations, waitForApp, waitForShownText } from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '../support/provider-server.ts';

const DIALOG = '[data-slot="dialog-content"]';

/** Replaces a field's text as the owner types it, so that its binding sees each change. */
async function fill(selector: string, value: string): Promise<void> {
  const field = await $(selector);
  if (value === '') {
    // WebKit rejects an empty send-keys request, and a native clear alone
    // can leave the binding unchanged. Delete through real keyboard events.
    await field.click();
    await browser.keys([Key.Ctrl, 'a']);
    await browser.keys(Key.Backspace);
  } else {
    await field.setValue(value);
  }
  await expect(field).toHaveValue(value);
}

/** The profiles in the list, each as the text of its row. */
async function profileRows(): Promise<string[]> {
  return browser.execute(() =>
    [...document.querySelectorAll('main li')]
      .filter((row) => row.querySelector('button') !== null)
      .map((row) => row.textContent.replaceAll(/\s+/gu, ' ').trim()),
  );
}

/** Opens a profile's menu in the list and chooses an item. */
async function chooseProfileAction(name: string, label: string): Promise<void> {
  await waitForPointerInput();
  await $(`main button[aria-label="More actions for ${name}"]`).click();
  await clickWithText('[data-slot="dropdown-menu-item"]', label);
}

/** Whether a row shows the profile `name` as the default. */
function isDefaultRow(row: string, name: string): boolean {
  const isProfile = row.startsWith(`${name} `);
  const isDefault = row.includes(' Default ');
  return isProfile && isDefault;
}

async function save(): Promise<void> {
  await $(`${DIALOG} button[type="submit"]`).click();
}

/** How far the dialog's fields reach past their scroll area to the side, in pixels; 0 when they fit. */
async function sidewaysOverflow(): Promise<number> {
  return browser.execute(() => {
    const fields = document.querySelector('[data-profile-fields]');
    if (fields === null) {
      throw new Error('the profile dialog is not open');
    }
    return fields.scrollWidth - fields.clientWidth;
  });
}

describe('provider profiles', () => {
  let provider: ProviderServer;

  before(async () => {
    provider = await startProviderServer();
    await waitForApp();
    await openSettings('chat.providers');
    await waitForShownText('h1', 'Settings');
  });

  after(async () => {
    // The profiles' credentials live in the system's credential store.
    await deleteProviderProfiles();
    await provider.close();
  });

  it('starts without profiles, and offers to add one', async () => {
    await waitForShownText($('main'), expect.stringContaining('No provider profiles yet'));
    expect(await axeViolations()).toEqual([]);
  });

  it('refuses to send a credential over plain HTTP to a public address, and says why', async () => {
    await clickWithText('main button', 'Add a profile');
    await $(DIALOG).waitForDisplayed();
    await waitForAnimations();
    // The fields scroll only down. The click area of the Use tools switch at
    // their end reached past the edge once, and they scrolled sideways.
    expect(await sidewaysOverflow()).toBe(0);
    await fill('#profile-name', 'Mock');
    await $('#profile-interface-chat_completions').click();
    await fill('#profile-base-url', 'http://example.com/v1');
    await fill('#profile-model', MODELS.echo);
    await fill('#profile-credential', 'e2e-key');
    await fill('#profile-context-window', '32768');
    await fill('#profile-output-limit', '4096');

    await save();

    await waitForShownText(
      $(DIALOG),
      expect.stringContaining(
        'Over plain HTTP, Mannie sends an API key only to this computer or to a private network address.',
      ),
    );
    expect(await axeViolations()).toEqual([]);
  });

  it('adds the profile, which becomes the default', async () => {
    await fill('#profile-base-url', provider.baseUrl);

    await save();

    await $(DIALOG).waitForDisplayed({ reverse: true });
    expect(await profileRows()).toEqual([`Mock ${MODELS.echo} · Chat Completions Default Test`]);
  });

  it('tests the profile against its server and applies the limits it reports', async () => {
    await clickWithText('main button', 'Test');

    const dialog = await $(DIALOG);
    await waitForShownText(dialog, expect.stringContaining('Server: Other server'));
    const text = await dialog.getText();
    // Every check passes but reasoning, including the Tools check, which the
    // test provider answers with a call of its tool.
    expect(text.match(/Passed\./gu)?.length).toBe(6);
    expect(text).toContain('Skipped because the profile sets no reasoning effort.');
    const contextWindow = await browser.execute(() => new Intl.NumberFormat().format(65_536));
    expect(text).toContain(`The server reports a context window of ${contextWindow} tokens.`);
    const reply = provider.requests().find((request) => {
      const content = JSON.stringify(request.body?.messages ?? null);
      return content.includes('Reply with the single word: ready');
    });
    expect(reply?.authorization).toBe('Bearer e2e-key');

    await clickWithText(`${DIALOG} button`, 'Use these limits');
    await waitForShownText(dialog, expect.stringContaining('The profile uses these limits now.'));
    await clickWithText(`${DIALOG} button`, 'Close');
    await dialog.waitForDisplayed({ reverse: true });
    expect(await profileRows()).toEqual([`Mock ${MODELS.echo} · Chat Completions Default Other server Test`]);
  });

  it('edits the profile without showing its stored credential, and warns before replacing it', async () => {
    await chooseProfileAction('Mock', 'Edit');
    await $(DIALOG).waitForDisplayed();
    await waitForAnimations();

    const credential = await $('#profile-credential');
    await expect(credential).toHaveValue('');
    await expect(credential).toHaveAttribute('placeholder', 'A key is saved. Type a new key to replace it.');
    await fill('#profile-credential', 'replacement-key');
    await waitForShownText($(DIALOG), expect.stringContaining('Saving replaces the saved key.'));
    expect(await axeViolations()).toEqual([]);
    // An empty field keeps the stored credential.
    await fill('#profile-credential', '');
    await expect($(DIALOG)).not.toHaveText(expect.stringContaining('Saving replaces the saved key.'));
    await expect($('#profile-context-window')).toHaveValue('65536');
    await fill('#profile-name', 'Mock edited');
    await save();

    await $(DIALOG).waitForDisplayed({ reverse: true });
    expect(await profileRows()).toEqual([`Mock edited ${MODELS.echo} · Chat Completions Default Other server Test`]);
  });

  it('removes the stored credential, after which requests carry none', async () => {
    await chooseProfileAction('Mock edited', 'Edit');
    await $(DIALOG).waitForDisplayed();
    await waitForAnimations();
    await clickWithText(`${DIALOG} button`, 'Remove the key');
    await expect($('#profile-credential')).toBeDisabled();
    await clickWithText(`${DIALOG} button`, 'Keep the key');
    await expect($('#profile-credential')).toBeEnabled();
    await clickWithText(`${DIALOG} button`, 'Remove the key');
    await waitForShownText($(DIALOG), expect.stringContaining('Saving removes the saved key.'));
    await save();
    await $(DIALOG).waitForDisplayed({ reverse: true });

    const requestsBefore = provider.requests().length;
    await clickWithText('main button', 'Test');
    const dialog = await $(DIALOG);
    await waitForShownText(dialog, expect.stringContaining('Skipped because the profile has no API key.'));
    await clickWithText(`${DIALOG} button`, 'Close');
    await dialog.waitForDisplayed({ reverse: true });

    const authorized = provider
      .requests()
      .slice(requestsBefore)
      .filter((request) => request.authorization !== null);
    expect(authorized).toEqual([]);
  });

  it('makes another profile the default', async () => {
    await clickWithText('main button', 'Add a profile');
    await $(DIALOG).waitForDisplayed();
    await waitForAnimations();
    await fill('#profile-name', 'Second');
    await $('#profile-interface-chat_completions').click();
    await fill('#profile-base-url', provider.baseUrl);
    await fill('#profile-model', MODELS.paced);
    await fill('#profile-context-window', '32768');
    await fill('#profile-output-limit', '4096');
    await save();
    await $(DIALOG).waitForDisplayed({ reverse: true });

    await chooseProfileAction('Second', 'Make default');

    await browser.waitUntil(async () => (await profileRows()).some((row) => isDefaultRow(row, 'Second')), {
      timeoutMsg: 'the second profile did not become the default',
    });
    expect((await profileRows()).filter((row) => row.includes('Default'))).toHaveLength(1);
  });

  it('deletes a profile after the owner confirms', async () => {
    await chooseProfileAction('Mock edited', 'Delete');
    const dialog = await $('[data-slot="alert-dialog-content"]');
    await dialog.waitForDisplayed();
    await waitForAnimations();
    // The destructive button's colors in both themes. The command palette
    // cannot open over the dialog, so the theme's class changes directly.
    for (const dark of [false, true]) {
      await browser.execute((isDark: boolean) => {
        document.documentElement.classList.toggle('dark', isDark);
      }, dark);
      expect(await axeViolations()).toEqual([]);
    }
    await clickWithText('[data-slot="alert-dialog-content"] button', 'Delete');

    await browser.waitUntil(async () => (await profileRows()).length === 1, {
      timeoutMsg: 'the deleted profile stayed in the list',
    });
    expect(await profileRows()).toEqual([`Second ${MODELS.paced} · Chat Completions Default Test`]);
  });
});
