// Network Client in the app (plugins/network/docs/README.md; ADR-0075,
// ADR-0076), with a local loopback HTTP server: the Plugins page lists the
// plugin off with its secrets, file dialog, store, YAML parser crate and
// socket grants; turning it on adds Network to the activity bar; the sidebar's
// overflow menu imports an OpenAPI 3.0 YAML specification from a file; editing
// the collection keeps a stable modal height across tabs and stores a Bearer
// token in the credential store; the request workbench synchronizes URL path
// and query parameters, resizes with the keyboard splitter, runs a live
// unredacted HTTP request into the CodeMirror response viewer with host-only
// cookies under strict CSP; and an agent tool call runs under approval while
// redacting the Bearer token and cookies from the transcript and provider
// requests. Every step passes axe in both themes.
import { mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
  addProfile,
  callMessage,
  clickInLatestTurn,
  send,
  startNewConversation,
  waitForCompleteReply,
  waitForToolStatus,
} from '#e2e/chat.ts';
import { type CspViolation, cspViolations, deleteProviderProfiles, setFileChoice } from '#e2e/harness.ts';
import {
  axeViolations,
  leaveSettings,
  openPluginUses,
  openSettings,
  runCommand,
  setPluginEnabled,
  waitForAnimations,
  waitForApp,
  waitForHidden,
  waitForShownText,
  waitForTextAbsent,
} from '#e2e/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '#e2e/provider-server.ts';
import { Key } from 'webdriverio';

const PLUGIN = '[data-plugin="network"]';
const PLANTED_BEARER = 'e2e-planted-network-bearer-77c4';
const PLANTED_COOKIE = 'e2e-planted-network-cookie-55f2';

const RESTORED_INLINE_STYLES = [
  'position: absolute; left: 0; top: 0; clip: rect(0 0 0 0); clip-path: inset(50%); overflow: hidden; white-space: nowrap; width: 1px; height: 1px',
  'display: contents;',
];

function isRestoredInlineStyle(violation: CspViolation): boolean {
  const isStyleAttribute = violation.directive === 'style-src-attr' || violation.directive === 'style-src';
  const isInline = violation.blockedUri === 'inline';
  const hasSample = violation.sample !== '';
  const isKnown = RESTORED_INLINE_STYLES.some((style) => style.startsWith(violation.sample));
  return isStyleAttribute && isInline && hasSample && isKnown;
}

interface LocalApiServer {
  readonly baseUrl: string;
  readonly close: () => Promise<void>;
}

async function startLocalApiServer(): Promise<LocalApiServer> {
  const server: Server = createServer((req: IncomingMessage, res: ServerResponse) => {
    const url = new URL(req.url ?? '/', 'http://127.0.0.1');
    const auth = req.headers.authorization ?? '';
    const cookie = req.headers.cookie ?? '';
    const payload = JSON.stringify({
      authorization: auth,
      cookie,
      expand: url.searchParams.get('expand'),
      path: url.pathname,
      tag: 'e2e-item-ok',
    });

    res.setHeader('Content-Type', 'application/json');
    res.setHeader('Set-Cookie', `sid=${PLANTED_COOKIE}; Path=/; HttpOnly`);
    res.end(payload);
  });

  await new Promise<void>((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      resolve();
    });
  });

  const address = server.address() as AddressInfo;
  return {
    baseUrl: `http://127.0.0.1:${String(address.port)}`,
    close: async () => {
      await new Promise<void>((resolve, reject) => {
        server.close((error) => {
          if (error) {
            reject(error);
          } else {
            resolve();
          }
        });
      });
    },
  };
}

function sampleOpenApiYaml(): string {
  return [
    'openapi: 3.0.3',
    'info:',
    '  title: Inventory E2E API',
    '  version: "1.0.0"',
    'components:',
    '  securitySchemes:',
    '    bearerAuth:',
    '      type: http',
    '      scheme: bearer',
    'security:',
    '  - bearerAuth: []',
    'paths:',
    '  /items/{itemId}:',
    '    get:',
    '      operationId: getItem',
    '      summary: Get item',
    '      parameters:',
    '        - name: itemId',
    '          in: path',
    '          required: true',
    '          schema:',
    '            type: string',
    '            example: "42"',
    '      responses:',
    '        "200":',
    '          description: OK',
  ].join('\n');
}

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

async function fill(selector: string, text: string): Promise<void> {
  const field = $(selector);
  await field.waitForDisplayed();
  await field.click();
  await field.clearValue();
  await field.addValue(text);
}

describe('Network Client', () => {
  let apiServer: LocalApiServer;
  let provider: ProviderServer;
  let folder: string;

  before(async () => {
    folder = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-e2e-network-')));
    apiServer = await startLocalApiServer();
    provider = await startProviderServer();
    await waitForApp();
  });

  after(async () => {
    await deleteProviderProfiles();
    await provider.close();
    await apiServer.close();
    rmSync(folder, { force: true, maxRetries: 20, recursive: true, retryDelay: 250 });
  });

  it('lists Network Client off with its grants, then turns on and opens its view', async () => {
    await openSettings('plugins.settings');
    await expect($(PLUGIN)).toHaveAttribute('data-plugin-state', 'off');

    await openPluginUses('network');

    await expect($(`${PLUGIN} [data-plugin-service="secrets"] [data-plugin-service-sensitive]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-service="store"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-service="file-dialog"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-crate="saphyr-parser"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-native="sockets"]`)).toBeDisplayed();

    await setPluginEnabled('network', true);
    await leaveSettings();

    await runCommand('Show Network');
    await $('[data-network-panel]').waitForDisplayed();
    await $('[data-network-empty]').waitForDisplayed();

    // Verify Network Client opts into per-activity shell sidebar resizing (ADR-0077).
    const sidebarResizer = $('[data-shell-sidebar-resizer]');
    await expect(sidebarResizer).toBeDisplayed();
    const beforeSidebar = Number(await sidebarResizer.getAttribute('aria-valuenow'));
    await browser.execute(() => {
      document.querySelector<HTMLElement>('[data-shell-sidebar-resizer]')?.focus();
    });
    await browser.keys(Key.ArrowRight);
    const afterSidebar = Number(await sidebarResizer.getAttribute('aria-valuenow'));
    expect(afterSidebar).toBeGreaterThan(beforeSidebar);

    await axeInBothThemes();
  });

  it('imports an OpenAPI 3.0 YAML spec, dismisses fixed import notes, highlights URL tokens and runs an unredacted owner request under strict CSP', async () => {
    const specFile = join(folder, 'inventory.yaml');
    writeFileSync(specFile, sampleOpenApiYaml(), 'utf8');
    await setFileChoice([specFile]);

    await $('[data-network-panel-actions]').click();
    await $('[data-network-panel-import]').waitForDisplayed();
    await axeInBothThemes();
    await $('[data-network-panel-import]').click();

    await $('[data-network-import-dialog]').waitForDisplayed();
    await $('[data-network-import-choose-file]').click();
    await $('[data-network-import-confirm]').waitForEnabled({ timeout: 15_000 });

    // Spec declared no servers: Import notes opens, then closes once a valid Base URL is entered.
    await expect($('[data-network-import-notes]')).toBeDisplayed();
    await waitForShownText('[data-network-import-notes]', expect.stringContaining('Specification declared no servers'));
    await axeInBothThemes();

    await fill('#network-import-base-url', apiServer.baseUrl);
    await waitForHidden('[data-network-import-notes]');

    await $('[data-network-import-confirm]').click();

    await $('[data-network-collection]').waitForDisplayed({ timeout: 15_000 });
    await waitForShownText('[data-network-collection]', /Inventory E2E API/u);

    // Edit the collection: dialog height stays stable across tabs, add a variable and store a Bearer secret.
    await $('[data-network-collection-edit]').click();
    await $('[data-network-collection-dialog]').waitForDisplayed();
    await waitForAnimations();
    const generalHeight = (await $('[data-network-collection-dialog]').getSize()).height;
    await $('[data-network-collection-dialog] [role="tab"][data-value="variables"]').click();
    const activePanelSelector = '[data-network-collection-dialog] [role="tabpanel"][data-state="active"]';
    await $(activePanelSelector).$('button*=Add row').click();
    await expect($(`${activePanelSelector} tbody tr:last-child [data-network-token-kind="variable"]`)).toBeDisplayed();
    await fill(`${activePanelSelector} tbody tr:last-child [data-network-token-kind="variable"] input`, 'expandMode');
    await fill(`${activePanelSelector} tbody tr:last-child td:nth-child(3) input`, 'full');

    await $('[data-network-collection-dialog] [role="tab"][data-value="auth"]').click();
    const authHeight = (await $('[data-network-collection-dialog]').getSize()).height;
    expect(Math.abs(authHeight - generalHeight)).toBeLessThanOrEqual(2);
    await fill('#network-auth-bearer-token', PLANTED_BEARER);
    await $('[data-network-collection-submit]').click();
    await waitForHidden('[data-network-collection-dialog]');

    // Open the imported request in the workbench.
    await $('[data-network-panel-request]').click();
    await $('[data-network-workbench]').waitForDisplayed();
    await expect($('[data-testid="network-request-url"]')).toHaveAttribute('data-code-editor-ready', 'true');
    await expect($('[data-network-token-legend]')).toBeDisplayed();

    // Verify separate Query and Path tabs, URL token highlighting and URL <-> Query sync.
    await expect($('[data-network-tab="query"]')).toBeDisplayed();
    await expect($('[data-network-tab="path"]')).toBeDisplayed();
    const urlEditor = $('#network-request-url');
    const urlTemplate = '/items/{itemId}?expand={{expandMode}}&mode=live';
    await urlEditor.click();
    await browser.keys([Key.Ctrl, 'a']);
    await urlEditor.execute((editor, text) => {
      const clipboardData = new DataTransfer();
      clipboardData.setData('text/plain', text);
      editor.dispatchEvent(new ClipboardEvent('paste', { bubbles: true, cancelable: true, clipboardData }));
    }, urlTemplate);
    await waitForShownText(urlEditor, urlTemplate);

    // CodeMirror can split a highlighted range across several DOM nodes.
    const highlighted = await urlEditor.execute((editor) => {
      const markedText = (selector: string): string =>
        Array.from(editor.querySelectorAll(selector), (mark) => mark.textContent).join('');
      return {
        path: markedText('.cm-tok-path'),
        queryKeys: markedText('.cm-tok-query-key'),
        queryValue: markedText('.cm-tok-query-val'),
        variable: markedText('.cm-tok-var'),
      };
    });
    expect(highlighted).toEqual({
      path: '{itemId}',
      queryKeys: 'expandmode',
      queryValue: 'live',
      variable: '{{expandMode}}',
    });
    await waitForShownText('[data-network-tab="query"]', expect.stringContaining('(2)'));
    await waitForShownText('[data-network-tab="path"]', expect.stringContaining('(1)'));
    await expect($$('[data-network-token-kind="query"]')).toBeElementsArrayOfSize(2);

    await $('[data-network-tab="path"]').click();
    await waitForShownText('[data-network-token-kind="path"]', '{itemId}');
    await $('[data-network-tab="query"]').click();

    // Resize vertically from the keyboard splitter.
    const splitter = $('[data-network-splitter]');
    const beforeSplit = Number(await splitter.getAttribute('aria-valuenow'));
    await browser.execute(() => {
      document.querySelector<HTMLElement>('[data-network-splitter]')?.focus();
    });
    await browser.keys(Key.ArrowDown);
    const afterSplit = Number(await splitter.getAttribute('aria-valuenow'));
    expect(afterSplit).toBeGreaterThan(beforeSplit);

    // Send live owner request: owner sees unredacted Bearer token and response cookie.
    await $('[data-network-send]').click();
    await waitForShownText('[data-testid="network-response-body"]', new RegExp(PLANTED_BEARER, 'u'));
    await waitForShownText('[data-testid="network-response-body"]', /"expand": "full"/u);

    // Verify CodeMirror mounted and zero unexpected CSP violations occurred.
    await expect($('[data-testid="network-response-body"]')).toHaveAttribute('data-code-editor-ready', 'true');
    const violations = await cspViolations();
    expect(violations.filter((violation) => !isRestoredInlineStyle(violation))).toEqual([]);

    await axeInBothThemes();
  });

  it('runs a saved request through the agent under approval while redacting secrets and cookies', async () => {
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'Network Tools', useTools: true });
    await runCommand('Show Chat');
    expect(Number(await $('[data-shell-sidebar-resizer]').getAttribute('aria-valuenow'))).toBe(256);
    await startNewConversation();

    await send(
      callMessage('network_send_request', {
        collection: 'Inventory E2E API',
        path_params: { itemId: '42' },
        request: 'Get item',
      }),
    );

    await waitForToolStatus('awaiting_approval');
    await axeInBothThemes();
    await clickInLatestTurn('Allow once');

    const reply = await waitForCompleteReply();
    expect(reply).toContain('e2e-item-ok');
    expect(reply).toContain('[redacted]');
    expect(reply).not.toContain(PLANTED_BEARER);
    expect(reply).not.toContain(PLANTED_COOKIE);

    await waitForToolStatus('completed');
    await waitForAnimations();
    await waitForTextAbsent('main', PLANTED_BEARER);
    await waitForTextAbsent('main', PLANTED_COOKIE);

    for (const request of provider.requests()) {
      const serialized = JSON.stringify(request.body);
      expect(serialized).not.toContain(PLANTED_BEARER);
      expect(serialized).not.toContain(PLANTED_COOKIE);
    }
  });
});
