// The chat feature's security (docs/features/chat/README.md, threat notes;
// ADR-0004, ADR-0010, ADR-0021): hostile Markdown from a provider runs no
// script, loads nothing and leads nowhere; a link opens only after the owner
// confirms it, through the core; and the provider credential appears nowhere
// in Mannie's data folder, also when the provider repeats it in an error.
import { randomUUID } from 'node:crypto';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

import { dataDirectory } from '../support/app.ts';
import {
  addProfile,
  clickWithText,
  sendMessage,
  useProfile,
  waitForCompleteReply,
  waitForLatestPart,
} from '../support/chat.ts';
import { type ConnectionCounter, countConnections } from '../support/connections.ts';
import { chooseTrayItem, cspViolations, deleteProviderProfiles, openedLinks, processId } from '../support/harness.ts';
import { hostileMarkdown } from '../support/hostile-html.ts';
import { axeViolations, waitForAnimations, waitForApp, waitForShownText } from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '../support/provider-server.ts';

/** A credential that must never appear outside the system's credential store. */
const PLANTED_CREDENTIAL = `sk-e2e-planted-${randomUUID()}`;

/** Whether the process with this ID still runs. */
function isRunning(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

/** Waits for `condition` in this process, without the WebDriver session, which ends when Mannie quits. */
async function poll(condition: () => boolean, timeoutMillis: number, message: () => string): Promise<void> {
  const deadline = Date.now() + timeoutMillis;
  while (!condition()) {
    if (Date.now() > deadline) {
      throw new Error(message());
    }
    await new Promise((resolve) => {
      setTimeout(resolve, 250);
    });
  }
}

interface Scan {
  /** Files that contain the text. */
  readonly found: string[];
  /** Files that were searched. */
  readonly searched: string[];
  /** Files that could not be read. */
  readonly unreadable: string[];
}

/** Searches every file under `directory` for `text`, in UTF-8 and in UTF-16. */
function scanFiles(directory: string, text: string): Scan {
  const needles = [Buffer.from(text, 'utf8'), Buffer.from(text, 'utf16le')];
  const found: string[] = [];
  const searched: string[] = [];
  const unreadable: string[] = [];
  for (const entry of readdirSync(directory, { recursive: true, withFileTypes: true })) {
    if (!entry.isFile()) {
      continue;
    }
    const path = join(entry.parentPath, entry.name);
    try {
      const bytes = readFileSync(path);
      searched.push(entry.name);
      if (needles.some((needle) => bytes.includes(needle))) {
        found.push(path);
      }
    } catch {
      unreadable.push(path);
    }
  }
  return { found, searched, unreadable };
}

describe('chat security', () => {
  let provider: ProviderServer;
  let counter: ConnectionCounter;

  before(async () => {
    provider = await startProviderServer();
    counter = await countConnections();
    await waitForApp();
    await addProfile(provider.baseUrl, { credential: PLANTED_CREDENTIAL, model: MODELS.hostile, name: 'Hostile' });
    await addProfile(provider.baseUrl, {
      credential: PLANTED_CREDENTIAL,
      model: MODELS.unauthorized,
      name: 'Unauthorized',
    });
  });

  after(async () => {
    // The profiles' credentials live in the system's credential store.
    await deleteProviderProfiles();
    await provider.close();
    await counter.close();
  });

  it('shows hostile Markdown from the provider as inert text', async () => {
    provider.setHostileText(hostileMarkdown(counter.origin).join('\n\n'));
    const violationsBefore = (await cspViolations()).length;

    await sendMessage('Show me everything');
    await waitForCompleteReply();
    await browser.pause(1_000);

    expect(counter.connections()).toBe(0);
    expect(await browser.execute(() => window.compromised === true)).toBe(false);
    const markup = await browser.execute(() => ({
      active: document.querySelectorAll(
        'main [role="region"] :is(a[href], img, iframe, script, style, object, embed, form, details)',
      ).length,
      urls: [...document.querySelectorAll('main [role="region"] *')].flatMap((element) =>
        [...element.attributes]
          // An icon's namespace declaration names no resource.
          .filter((attribute) => !attribute.name.startsWith('xmlns'))
          .filter((attribute) => /https?:|javascript:/iu.test(attribute.value))
          .map((attribute) => `${element.tagName}[${attribute.name}]`),
      ),
    }));
    expect(markup).toEqual({ active: 0, urls: [] });
    const blocked = (await cspViolations()).slice(violationsBefore);
    expect(blocked).toEqual([]);
    expect(await axeViolations()).toEqual([]);
  });

  it('opens a link only after the owner confirms it, through the core', async () => {
    await clickWithText('main button', 'remote');

    const dialog = await $('[data-slot="alert-dialog-content"]');
    await dialog.waitForDisplayed();
    await waitForAnimations();
    await waitForShownText(dialog, expect.stringContaining(`${counter.origin}/link`));
    await clickWithText('[data-slot="alert-dialog-content"] button', 'Open in browser');
    await dialog.waitForDisplayed({ reverse: true });

    expect(await openedLinks()).toEqual([`${counter.origin}/link`]);
    expect(counter.connections()).toBe(0);
  });

  it('refuses a link that is not a web address', async () => {
    await clickWithText('main button', 'click');

    const dialog = await $('[data-slot="alert-dialog-content"]');
    await dialog.waitForDisplayed();
    await waitForAnimations();
    await waitForShownText(dialog, expect.stringContaining('Links of the type javascript do not open from Mannie.'));
    const buttons = await browser.execute(() =>
      [...document.querySelectorAll('[data-slot="alert-dialog-content"] button')].map((button) =>
        button.textContent.trim(),
      ),
    );
    expect(buttons).toEqual(['Cancel']);
    await clickWithText('[data-slot="alert-dialog-content"] button', 'Cancel');
    await dialog.waitForDisplayed({ reverse: true });

    expect(await openedLinks()).toEqual([`${counter.origin}/link`]);
    expect(await browser.execute(() => window.compromised === true)).toBe(false);
  });

  it('keeps the credential out of the data folder when the provider repeats it in an error', async () => {
    await useProfile('Unauthorized');
    await sendMessage('Refuse me');
    await waitForLatestPart('error', ['complete']);
    const sent = provider.requests().filter((request) => request.body?.model === MODELS.unauthorized);
    expect(sent.map((request) => request.authorization)).toEqual([`Bearer ${PLANTED_CREDENTIAL}`]);

    // Quitting closes the database and flushes the logs, so every byte that
    // Mannie wrote is on disk when the folder is searched.
    const pid = await processId();
    await chooseTrayItem('quit');
    await poll(
      () => !isRunning(pid),
      15_000,
      () => 'Mannie did not quit',
    );
    let scan: Scan = { found: [], searched: [], unreadable: [] };
    // The webview's processes can hold their files for a moment after Mannie ends.
    await poll(
      () => {
        scan = scanFiles(dataDirectory(), PLANTED_CREDENTIAL);
        return scan.unreadable.length === 0;
      },
      15_000,
      () => `files stayed unreadable: ${scan.unreadable.join(', ')}`,
    );

    expect(scan.found).toEqual([]);
    expect(scan.searched).toContain('mannie.sqlite');
    expect(scan.searched.some((name) => name.endsWith('.log'))).toBe(true);

    // The next start removes the profiles and their credentials.
    await browser.reloadSession();
    await waitForApp();
  });
});
