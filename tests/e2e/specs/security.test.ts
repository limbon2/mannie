// V1: the security baseline of the webview (ADR-0004). The policy holds with
// the whole component stack, a command without a grant is refused, and
// hostile HTML runs no script and loads nothing remote.
import { Key } from 'webdriverio';

import { type ConnectionCounter, countConnections } from '../support/connections.ts';
import {
  callUndeclaredPluginCommand,
  callUngrantedCommand,
  type CspViolation,
  cspViolations,
  insertHostileHtml,
  probeScripts,
} from '../support/harness.ts';
import { hostileHtml, hostileNavigations } from '../support/hostile-html.ts';
import { waitForApp, waitForShownText } from '../support/page.ts';

/**
 * Static inline style attributes in SvelteKit's route announcer and in
 * bits-ui's command wrappers. The policy blocks them, and app.css gives the
 * elements the same presentation.
 */
const RESTORED_INLINE_STYLES = [
  'position: absolute; left: 0; top: 0; clip: rect(0 0 0 0); clip-path: inset(50%); overflow: hidden; white-space: nowrap; width: 1px; height: 1px',
  'display: contents;',
];

/**
 * Whether the violation is one of the blocked attributes that app.css
 * restores. The validation build's policy reports a sample of each blocked
 * value: its first 40 characters.
 */
function isRestoredInlineStyle(violation: CspViolation): boolean {
  const isStyleAttribute = violation.directive === 'style-src-attr' || violation.directive === 'style-src';
  const isInline = violation.blockedUri === 'inline';
  const hasSample = violation.sample !== '';
  const isKnown = RESTORED_INLINE_STYLES.some((style) => style.startsWith(violation.sample));
  return isStyleAttribute && isInline && hasSample && isKnown;
}

describe('the webview security baseline (V1)', () => {
  let counter: ConnectionCounter;

  before(async () => {
    counter = await countConnections();
    await waitForApp();
  });

  after(async () => {
    await counter.close();
  });

  it('renders the shell, the settings and the command palette under the policy', async () => {
    await browser.keys([Key.Ctrl, ',']);
    await waitForShownText('h1', 'Settings');
    await browser.keys([Key.Ctrl, 'k']);
    await $('[data-slot="command-input"]').waitForDisplayed();
    await browser.keys(Key.Escape);
    await $('[data-slot="command-input"]').waitForDisplayed({ reverse: true });

    const violations = await cspViolations();

    expect(violations.filter((violation) => !isRestoredInlineStyle(violation))).toEqual([]);
  });

  it('does not expose the global Tauri object', async () => {
    expect(await browser.execute(() => '__TAURI__' in window)).toBe(false);
  });

  it('refuses a command that no capability grants', async () => {
    const result = await callUngrantedCommand();

    expect(result).not.toBe('allowed');
    expect(result).toMatch(/not allowed/i);
  });

  it('refuses a plugin command that the plugin does not declare, which the transport reports as not_allowed', async () => {
    expect(await callUndeclaredPluginCommand()).toBe('not_allowed');
  });

  it('refuses eval, the Function constructor, inline script and inline handlers', async () => {
    expect(await probeScripts()).toEqual({
      evalRan: false,
      functionConstructorRan: false,
      inlineHandlerRan: false,
      inlineScriptRan: false,
    });
  });

  it('runs no script and loads nothing remote from hostile HTML', async () => {
    const earlier = (await cspViolations()).length;
    const connectionsBefore = counter.connections();

    const compromised = await insertHostileHtml(hostileHtml(counter.origin));
    await browser.pause(1_000);

    expect(compromised).toBe(false);
    expect(counter.connections() - connectionsBefore).toBe(0);
    const blocked = (await cspViolations()).slice(earlier);
    expect(blocked.some((violation) => violation.blockedUri.startsWith(counter.origin))).toBe(true);
  });

  it('refuses the navigations that hostile HTML asks for', async () => {
    const origin = await browser.execute(() => location.origin);
    const earlier = (await cspViolations()).length;
    const connectionsBefore = counter.connections();

    const compromised = await insertHostileHtml(hostileNavigations(counter.origin));
    await browser.pause(1_000);

    expect(compromised).toBe(false);
    expect(await browser.execute(() => location.origin)).toBe(origin);
    const blocked = (await cspViolations()).slice(earlier);
    expect(blocked.some((violation) => violation.directive === 'frame-src')).toBe(true);
    // The engine connects to a navigation's host before the navigation is
    // refused. The V1 record lists this; M1's Markdown renderer must keep it
    // at zero by never producing navigation markup.
    console.log(`V1 connections opened by refused navigations: ${String(counter.connections() - connectionsBefore)}`);
  });

  it('stays on its origin when a script navigates away', async () => {
    const origin = await browser.execute(() => location.origin);

    await browser.execute((target: string) => {
      location.href = target;
    }, `${counter.origin}/navigation`);
    await browser.pause(1_000);

    expect(await browser.execute(() => location.origin)).toBe(origin);
  });

  it('opens no window and no connection when a script asks for a window', async () => {
    const before = await browser.getWindowHandles();
    const connectionsBefore = counter.connections();

    await browser.execute((target: string) => {
      window.open(target, '_blank');
    }, `${counter.origin}/window`);
    await browser.pause(1_000);

    expect(await browser.getWindowHandles()).toHaveLength(before.length);
    expect(counter.connections() - connectionsBefore).toBe(0);
  });
});
