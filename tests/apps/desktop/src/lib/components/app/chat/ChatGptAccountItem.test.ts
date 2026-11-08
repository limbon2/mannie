// The ChatGPT account above the provider profiles (docs/features/
// chatgpt-plan.md, the ChatGPT account), rendered on the server side in each
// of its states. A server render runs no effect, so the item follows nothing
// here, and the tests set the account as the core would report it.
import { render } from 'svelte/server';
import { afterEach, describe, expect, it, vi } from 'vitest';

import ChatGptAccountItem from '../../../../../../../../apps/desktop/src/lib/components/app/chat/ChatGptAccountItem.svelte';
import type { ChatGptAccountView } from '../../../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { chatgptAccount } from '../../../../../../../../apps/desktop/src/lib/state/chatgpt-account.svelte.js';

vi.mock('../../../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: {} }));

const EMAIL = 'owner@example.com';

function account(changes: Partial<ChatGptAccountView> = {}): ChatGptAccountView {
  return {
    email: null,
    known: false,
    noticeSeen: true,
    signIn: { step: 'idle' },
    state: 'signed_out',
    ...changes,
  };
}

/** The item as it shows an account that the core reported. */
function shown(view: ChatGptAccountView): string {
  chatgptAccount.account = view;
  chatgptAccount.status = 'ready';
  return render(ChatGptAccountItem).body;
}

afterEach(() => {
  chatgptAccount.account = null;
  chatgptAccount.failure = null;
  chatgptAccount.startFailure = null;
  chatgptAccount.status = 'loading';
});

describe('ChatGptAccountItem', () => {
  it('says that it loads until the core reports the account', () => {
    const body = render(ChatGptAccountItem).body;

    expect(body).toContain('Loading the ChatGPT account…');
    expect(body).not.toContain('Continue with ChatGPT');
  });

  it('says why the account could not be read, and offers to try again', () => {
    chatgptAccount.status = 'error';
    chatgptAccount.failure = { error: { code: 'store_unavailable' }, kind: 'core' };

    const body = render(ChatGptAccountItem).body;

    expect(body).toContain('The ChatGPT account could not be read');
    expect(body).toContain('Try again');
  });

  it('says what the plan does before the first sign-in, with OpenAI’s button', () => {
    const body = shown(account());

    expect(body).toContain('Use your ChatGPT plan for replies, in place of an API key.');
    expect(body).toContain('Continue with ChatGPT');
    expect(body).toContain('data-oai-siwc-button');
    // The two logos, one for each theme, are decorative.
    expect(body.match(/<img[^>]*alt=""/gu)).toHaveLength(2);
    expect(body).not.toContain('Use another account');
  });

  it('offers another account once Mannie knows one, and names the known one', () => {
    const body = shown(account({ email: EMAIL, known: true }));

    expect(body).toContain(`${EMAIL} is signed out of ChatGPT.`);
    expect(body).toContain('Continue with ChatGPT');
    expect(body).toContain('Use another account');
  });

  it('asks for a new sign-in after the session ended', () => {
    const body = shown(account({ email: EMAIL, known: true, state: 'sign_in_needed' }));

    expect(body).toContain(`The ChatGPT session of ${EMAIL} ended.`);
    expect(body).toContain('Continue with ChatGPT');
    expect(body).toContain('Use another account');
  });

  it('shows a sign-in that waits for the browser, with its page again and Cancel', () => {
    const body = shown(account({ signIn: { step: 'waiting_for_browser' } }));

    expect(body).toContain('Finish signing in in your browser.');
    expect(body).toContain('Open the sign-in page again');
    expect(body).toContain('Cancel');
    expect(body).not.toContain('Continue with ChatGPT');
  });

  it('checks the account without offering its page again', () => {
    const body = shown(account({ signIn: { step: 'finishing' } }));

    expect(body).toContain('Checking the account…');
    expect(body).toContain('Cancel');
    expect(body).not.toContain('Open the sign-in page again');
  });

  it('shows the signed-in account with Manage usage and Sign out', () => {
    const body = shown(account({ email: EMAIL, known: true, state: 'signed_in' }));

    expect(body).toContain(`Signed in as ${EMAIL}`);
    expect(body).toContain('Using ChatGPT plan');
    expect(body).toContain('Manage usage');
    expect(body).toContain('Sign out');
    expect(body).not.toContain('Continue with ChatGPT');
  });

  it('says why the last sign-in failed and what the owner can do', () => {
    const body = shown(account({ signIn: { error: { code: 'port_in_use', port: 1455 }, step: 'failed' } }));

    expect(body).toContain('The sign-in to ChatGPT failed');
    expect(body).toContain('Another program uses port 1455');
    expect(body).toContain('Continue with ChatGPT');
  });

  it('says why a sign-in did not start', () => {
    chatgptAccount.startFailure = { error: { code: 'browser_unavailable' }, kind: 'core' };

    const body = shown(account());

    expect(body).toContain('The sign-in to ChatGPT failed');
    expect(body).toContain('The system browser could not be opened.');
  });
});
