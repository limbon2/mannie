import {
  axeViolations,
  leaveSettings,
  openSettings,
  runCommand,
  setPluginEnabled,
  waitForApp,
  waitForShownText,
} from '#e2e/page.ts';
// The plugin in the app: once it is on, its command opens its settings
// section, which adds a note through the
// plugin's command and shows it, and the sidebar's group shows the notes
// that the plugin's channel brings; "Add note" waits for text; and turning
// the plugin off takes its group away, and on again starts with no notes.
// Every step works from the keyboard and passes axe in both themes.
import { Key } from 'webdriverio';

/** The plugin's field for a new note. */
const NOTE_FIELD = '#example-note';

/** The plugin's button that adds the note. */
const ADD_NOTE = '[data-example-add-note]';

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

/** Types a note into the focused field and adds it with Enter. */
async function addNote(text: string): Promise<void> {
  await $(NOTE_FIELD).click();
  await browser.keys(text.split(''));
  await browser.keys(Key.Enter);
}

describe('the example plugin', () => {
  before(async () => {
    await waitForApp();
    await setPluginEnabled('example', true);
    await leaveSettings();
  });

  it('opens its settings section from its command, which starts with no notes', async () => {
    await runCommand('Add an example note…');

    await $('[data-example-settings]').waitForDisplayed();
    await expect($(ADD_NOTE)).toBeDisabled();
    await expect($('[data-example-settings] [data-slot="empty"]')).toBeDisplayed();
    await axeInBothThemes();
  });

  it('adds a note from the keyboard, which shows in its section and in the sidebar', async () => {
    await addNote('Water the plants');

    await $('[data-example-note]').waitForDisplayed();
    await waitForShownText($('[data-example-note]'), 'Water the plants');
    await expect($(NOTE_FIELD)).toHaveValue('');
    await axeInBothThemes();
    await leaveSettings();
    await $('[data-example-panel]').waitForDisplayed();
    await waitForShownText($('[data-example-panel] [data-example-panel-note]'), 'Water the plants');
    await axeInBothThemes();
  });

  it('takes its group away while it is off, and starts with no notes once it is on again', async () => {
    await setPluginEnabled('example', false);
    await leaveSettings();

    await expect($('[data-example-panel]')).not.toBeExisting();
    await setPluginEnabled('example', true);
    await openSettings('example.settings');
    await $('[data-example-settings] [data-slot="empty"]').waitForDisplayed();
    await expect($('[data-example-note]')).not.toBeExisting();
  });
});
