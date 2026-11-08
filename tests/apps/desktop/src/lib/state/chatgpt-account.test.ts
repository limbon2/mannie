// The ChatGPT account as the window follows it (docs/features/
// chatgpt-plan.md, the ChatGPT account; ADR-0062): the account follows the
// core's channel, a sign-in sends the catalog's texts of the browser's page,
// a sign-in that does not start or that fails says why until the next one
// starts, the notice is due once after the first sign-in, and "Manage usage"
// opens ChatGPT's usage settings through the core's link opener.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  ChatGptAccountUpdate,
  ChatGptAccountView,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { m } from '../../../../../../apps/desktop/src/lib/paraglide/messages.js';
import {
  CHATGPT_USAGE_URL,
  ChatGptAccount,
} from '../../../../../../apps/desktop/src/lib/state/chatgpt-account.svelte.js';
import type * as Calls from '../../../../../../apps/desktop/src/lib/utils/calls.js';

const core = vi.hoisted(() => ({
  acknowledgeChatgptNotice: vi.fn(),
  cancelChatgptSignIn: vi.fn(),
  listChatgptModels: vi.fn(),
  openLink: vi.fn(),
  reopenChatgptSignInPage: vi.fn(),
  signOutChatgpt: vi.fn(),
  startChatgptSignIn: vi.fn(),
  subscribeChatgptAccount: vi.fn(),
  unsubscribe: vi.fn(),
}));

/** Hands batches of updates to the account, as the core's channel does. */
const channel = vi.hoisted(() => ({ deliver: (_updates: readonly ChatGptAccountUpdate[]): void => undefined }));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));
vi.mock('../../../../../../apps/desktop/src/lib/utils/calls.js', async (importOriginal) => ({
  ...(await importOriginal<typeof Calls>()),
  batchChannel: (onItems: (updates: readonly ChatGptAccountUpdate[]) => void) => {
    channel.deliver = onItems;
    return {};
  },
}));

function view(changes: Partial<ChatGptAccountView> = {}): ChatGptAccountView {
  return {
    email: null,
    known: false,
    noticeSeen: false,
    signIn: { step: 'idle' },
    state: 'signed_out',
    ...changes,
  };
}

/** Delivers the account as the core reports it. */
function deliver(account: ChatGptAccountView): void {
  channel.deliver([{ account, type: 'account' }]);
}

/** Starts following the account, once the core accepted the subscription. */
async function started(): Promise<ChatGptAccount> {
  const account = new ChatGptAccount();
  account.start();
  await vi.waitFor(() => {
    expect(core.subscribeChatgptAccount).toHaveBeenCalled();
  });
  return account;
}

beforeEach(() => {
  for (const command of Object.values(core)) {
    command.mockReset();
  }
  core.subscribeChatgptAccount.mockResolvedValue({ data: 7, status: 'ok' });
  for (const command of [
    core.acknowledgeChatgptNotice,
    core.cancelChatgptSignIn,
    core.openLink,
    core.reopenChatgptSignInPage,
    core.startChatgptSignIn,
  ]) {
    command.mockResolvedValue({ data: null, status: 'ok' });
  }
});

describe('the ChatGPT account', () => {
  it('loads until the core sends the account, then follows it', async () => {
    const account = await started();
    const loading = account.status;

    deliver(view({ email: 'owner@example.com', known: true, state: 'signed_in' }));

    expect(loading).toBe('loading');
    expect(account.status).toBe('ready');
    expect(account.signedIn).toBe(true);
    expect(account.account?.email).toBe('owner@example.com');
  });

  it('starts following once, however often it is asked', async () => {
    const account = await started();

    account.start();

    expect(core.subscribeChatgptAccount).toHaveBeenCalledTimes(1);
  });

  it('says why it could not follow the account, and follows it again on request', async () => {
    core.subscribeChatgptAccount.mockResolvedValueOnce({ error: { code: 'store_unavailable' }, status: 'error' });
    const account = new ChatGptAccount();
    account.start();
    await vi.waitFor(() => {
      expect(account.status).toBe('error');
    });
    const failure = account.failure;

    account.retry();
    deliver(view());

    expect(failure).toEqual({ error: { code: 'store_unavailable' }, kind: 'core' });
    expect(account.status).toBe('ready');
    expect(account.failure).toBeNull();
    expect(core.subscribeChatgptAccount).toHaveBeenCalledTimes(2);
  });

  it('follows the account again when its channel falls behind, and lets the old subscription go', async () => {
    const account = await started();
    deliver(view());

    channel.deliver([{ reason: 'lagged', type: 'ended' }]);

    await vi.waitFor(() => {
      expect(core.subscribeChatgptAccount).toHaveBeenCalledTimes(2);
    });
    expect(core.unsubscribe).toHaveBeenCalledWith({ subscriptionId: 7 });
    expect(account.status).toBe('ready');
  });

  it('shows the notice once, after the first sign-in, until the owner saw it', async () => {
    const account = await started();
    const dueBefore = account.noticeDue;

    deliver(view({ known: true, state: 'signed_in' }));
    const dueAfterSignIn = account.noticeDue;
    deliver(view({ known: true, noticeSeen: true, state: 'signed_in' }));

    expect(dueBefore).toBe(false);
    expect(dueAfterSignIn).toBe(true);
    expect(account.noticeDue).toBe(false);
  });

  it('tells a sign-in that runs from one that does not', async () => {
    const account = await started();
    const steps: boolean[] = [];

    for (const signIn of [{ step: 'idle' }, { step: 'waiting_for_browser' }, { step: 'finishing' }] as const) {
      deliver(view({ signIn }));
      steps.push(account.signingIn);
    }

    expect(steps).toEqual([false, true, true]);
  });
});

describe('a sign-in', () => {
  it('sends the texts of the browser’s page from the catalog, and whether it is for another account', async () => {
    const account = await started();

    expect(await account.signIn(true)).toBeNull();

    expect(core.startChatgptSignIn).toHaveBeenCalledWith({
      anotherAccount: true,
      page: {
        failed: m.chat_chatgpt_page_failed(),
        signedIn: m.chat_chatgpt_page_signed_in(),
        title: m.chat_chatgpt_page_title(),
      },
    });
    for (const text of [m.chat_chatgpt_page_failed(), m.chat_chatgpt_page_signed_in(), m.chat_chatgpt_page_title()]) {
      // The core refuses texts that are empty or longer than 500 characters.
      expect(text.length).toBeGreaterThan(0);
      expect(text.length).toBeLessThanOrEqual(500);
    }
  });

  it('says why it did not start until the next one starts', async () => {
    core.startChatgptSignIn.mockResolvedValueOnce({ error: { code: 'port_in_use', port: 1455 }, status: 'error' });
    const account = await started();
    deliver(view());

    const returned = await account.signIn(false);
    const shown = account.signInFailure;
    await account.signIn(false);

    expect(returned).toEqual({ error: { code: 'port_in_use', port: 1455 }, kind: 'core' });
    expect(shown).toEqual(returned);
    expect(account.signInFailure).toBeNull();
  });

  it('says why it failed after it started, from the account’s sign-in step', async () => {
    const account = await started();

    deliver(view({ signIn: { error: { code: 'plan_usage_not_granted' }, step: 'failed' } }));

    expect(account.signInFailure).toEqual({ error: { code: 'plan_usage_not_granted' }, kind: 'core' });
  });

  it('keeps the account’s buttons off while a command waits for the core', async () => {
    let answer: (result: { data: null; status: 'ok' }) => void = () => undefined;
    core.startChatgptSignIn.mockReturnValueOnce(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    const account = await started();

    const signingIn = account.signIn(false);
    const pending = account.pending;
    answer({ data: null, status: 'ok' });
    await signingIn;

    expect(pending).toBe('sign_in');
    expect(account.pending).toBeNull();
  });

  it('opens the page again, cancels, and records the notice through the core', async () => {
    core.cancelChatgptSignIn.mockResolvedValueOnce({ error: { code: 'not_signing_in' }, status: 'error' });
    const account = await started();

    const reopened = await account.reopenPage();
    const canceled = await account.cancel();
    const acknowledged = await account.acknowledgeNotice();

    expect(reopened).toBeNull();
    expect(canceled).toEqual({ error: { code: 'not_signing_in' }, kind: 'core' });
    expect(acknowledged).toBeNull();
    expect(core.reopenChatgptSignInPage).toHaveBeenCalledTimes(1);
    expect(core.acknowledgeChatgptNotice).toHaveBeenCalledTimes(1);
  });
});

describe('the signed-in account', () => {
  it('reports whether OpenAI confirmed the sign-out', async () => {
    core.signOutChatgpt.mockResolvedValueOnce({ data: { revoked: false }, status: 'ok' });
    const account = await started();

    const outcome = await account.signOut();

    expect(outcome).toEqual({ data: { revoked: false }, ok: true });
    expect(account.pending).toBeNull();
  });

  it('reads the plan’s models through the core', async () => {
    core.listChatgptModels.mockResolvedValueOnce({ data: [{ displayName: 'GPT-5', slug: 'gpt-5' }], status: 'ok' });
    const account = new ChatGptAccount();

    expect(await account.models()).toEqual({ data: [{ displayName: 'GPT-5', slug: 'gpt-5' }], ok: true });
  });

  it('opens ChatGPT’s usage settings through the core’s link opener', async () => {
    core.openLink.mockResolvedValueOnce({ error: { code: 'open_failed' }, status: 'error' });
    const account = new ChatGptAccount();

    const opened = await account.manageUsage();
    const failed = await account.manageUsage();

    expect(opened).toEqual({ error: { code: 'open_failed' }, kind: 'core' });
    expect(failed).toBeNull();
    expect(core.openLink).toHaveBeenCalledWith({ url: CHATGPT_USAGE_URL });
    expect(CHATGPT_USAGE_URL).toBe('https://chatgpt.com/settings/usage');
  });
});
