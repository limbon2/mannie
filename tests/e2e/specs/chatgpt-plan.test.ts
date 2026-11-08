// The ChatGPT plan (docs/features/chatgpt-plan.md, acceptance; ADR-0062): the
// owner signs in with ChatGPT through the browser, sees the notice once, makes
// a profile on the plan from its catalog, gets replies billed to the plan,
// crosses a refresh of the tokens, meets the plan's usage limit with "Manage
// usage", and signs out. The provider server plays OpenAI's sign-in and the
// plan; the validation build records the pages that it would open, and the
// test plays the browser.
import {
  clickWithText,
  sendMessage,
  startNewConversation,
  waitForCompleteReply,
  waitForLatestPart,
} from '../support/chat.ts';
import { deleteProviderProfiles, expireChatGptToken, openedLinks, setChatGptMock } from '../support/harness.ts';
import { openSettings, waitForAnimations, waitForApp, waitForShownText } from '../support/page.ts';
import {
  PLAN_EMAIL,
  PLAN_MODEL,
  type ProviderServer,
  type RecordedRequest,
  startProviderServer,
  USAGE_LIMIT_MARKER,
} from '../support/provider-server.ts';

const DIALOG = '[data-slot="dialog-content"]';
const ACCOUNT = '[data-chatgpt-account]';
const USAGE_URL = 'https://chatgpt.com/settings/usage';

/** The requests of the provider server to `path`. */
function requestsTo(provider: ProviderServer, path: string): RecordedRequest[] {
  return provider.requests().filter((request) => request.path === path);
}

/** Waits until the validation build recorded a page with `fragment` after the first `after` pages, and returns it. */
async function waitForOpenedLink(after: number, fragment: string): Promise<string> {
  let found: string | undefined;
  await browser.waitUntil(
    async () => {
      found = (await openedLinks()).slice(after).find((link) => link.includes(fragment));
      return found !== undefined;
    },
    { timeout: 15_000, timeoutMsg: `no page with ${fragment} was opened` },
  );
  if (found === undefined) {
    throw new Error('unreachable: waitUntil resolved without a link');
  }
  return found;
}

describe('the ChatGPT plan', () => {
  let provider: ProviderServer;

  before(async () => {
    provider = await startProviderServer();
    await waitForApp();
    await setChatGptMock(provider.origin, provider.baseUrl);
    await openSettings('chat.providers');
    await waitForShownText('h1', 'Settings');
  });

  after(async () => {
    await deleteProviderProfiles();
    await provider.close();
  });

  it('signs in through the browser, keeps the account and shows the notice once', async () => {
    await $(`${ACCOUNT}[data-chatgpt-account="signed_out"]`).waitForDisplayed({ timeout: 15_000 });
    const before = (await openedLinks()).length;

    await $('button[data-chatgpt-sign-in]').click();
    const page = await waitForOpenedLink(before, '/api/accounts/authorize');
    // The browser follows OpenAI's redirect back to Mannie's loopback listener.
    const answer = await fetch(page);

    expect(answer.status).toBe(200);
    expect(await answer.text()).toContain('You are signed in to ChatGPT.');
    await $(`${ACCOUNT}[data-chatgpt-account="signed_in"]`).waitForDisplayed({ timeout: 15_000 });
    await waitForShownText($(ACCOUNT), expect.stringContaining(`Signed in as ${PLAN_EMAIL}`));
    const authorize = new URL(page);
    expect(authorize.searchParams.get('client_id')).toBe('dynamic_agent_client');
    expect(authorize.searchParams.get('code_challenge_method')).toBe('S256');
    expect(authorize.searchParams.get('scope')).toContain('chatgpt.tokens.use.direct');
    const exchange = requestsTo(provider, '/api/accounts/oauth/token')[0];
    expect(exchange?.body?.grant_type).toBe('authorization_code');
    expect(exchange?.body?.client_id).toBe('oaiapp_e2e');
    expect(requestsTo(provider, '/.well-known/jwks.json')).toHaveLength(1);

    const notice = await $('[data-chatgpt-notice]');
    await notice.waitForDisplayed({ timeout: 10_000 });
    await expect(notice).toHaveText(expect.stringContaining("You're using your ChatGPT plan"));
    await clickWithText('[data-chatgpt-notice] button', 'Got it');
    await notice.waitForDisplayed({ reverse: true, timeout: 10_000 });
  });

  it('makes a profile on the plan from its catalog', async () => {
    await clickWithText('main button', 'Add a profile');
    await $(DIALOG).waitForDisplayed();
    await $('#profile-name').setValue('Plan');
    await $('#profile-access-chatgpt_plan').click();

    await expect($('#profile-base-url')).not.toExist();
    await expect($('#profile-credential')).not.toExist();
    const model = await $('#profile-plan-model');
    await model.waitForEnabled({ timeout: 15_000 });
    await model.click();
    await $(`[data-slot="select-item"][data-value="${PLAN_MODEL}"]`).click();
    await waitForAnimations();
    await $('#profile-context-window').setValue('65536');
    await $('#profile-output-limit').setValue('4096');
    await $(`${DIALOG} button[type="submit"]`).click();
    await $(DIALOG).waitForDisplayed({ reverse: true, timeout: 10_000 });

    await $('[data-chatgpt-plan]').waitForDisplayed({ timeout: 10_000 });
    expect(
      requestsTo(provider, '/v1/models').some((request) => request.authorization?.startsWith('Bearer plan-access-')),
    ).toBe(true);
  });

  it('replies on the plan without an output limit and shows the indicator', async () => {
    await startNewConversation();
    await $('[data-chatgpt-indicator="signed_in"]').waitForDisplayed({ timeout: 15_000 });

    await sendMessage('Hello on the plan');
    const reply = await waitForCompleteReply();

    expect(reply).toContain('Hello on the plan');
    const call = requestsTo(provider, '/v1/responses').at(-1);
    expect(call?.authorization).toMatch(/^Bearer plan-access-/u);
    expect(call?.body?.model).toBe(PLAN_MODEL);
    expect(call?.body).not.toHaveProperty('max_output_tokens');
  });

  it('refreshes the tokens before a call when they expire', async () => {
    const refreshesBefore = requestsTo(provider, '/api/accounts/oauth/token').length;
    const tokenBefore = requestsTo(provider, '/v1/responses').at(-1)?.authorization;
    await expireChatGptToken();

    await sendMessage('After the refresh');
    await waitForCompleteReply();

    const grants = requestsTo(provider, '/api/accounts/oauth/token').slice(refreshesBefore);
    expect(grants.map((request) => request.body?.grant_type)).toEqual(['refresh_token']);
    const tokenAfter = requestsTo(provider, '/v1/responses').at(-1)?.authorization;
    expect(tokenAfter).toMatch(/^Bearer plan-access-/u);
    expect(tokenAfter).not.toBe(tokenBefore);
  });

  it('shows the usage limit with Manage usage first', async () => {
    const before = (await openedLinks()).length;

    await sendMessage(`${USAGE_LIMIT_MARKER}: one more reply`);
    const failed = await waitForLatestPart('error', ['complete']);

    expect(failed.text).toContain('Usage limit reached');
    expect(failed.text).toContain("Review your plan or this app's limit in ChatGPT settings.");
    await clickWithText('main [data-part-kind="error"] button', 'Manage usage');
    expect(await waitForOpenedLink(before, USAGE_URL)).toBe(USAGE_URL);
  });

  it('signs out, revokes the session and keeps the registration', async () => {
    await openSettings('chat.providers');
    await $(`${ACCOUNT}[data-chatgpt-account="signed_in"]`).waitForDisplayed({ timeout: 15_000 });

    await clickWithText(`${ACCOUNT} button`, 'Sign out');

    await $(`${ACCOUNT}[data-chatgpt-account="signed_out"]`).waitForDisplayed({ timeout: 15_000 });
    const revoke = requestsTo(provider, '/api/accounts/oauth/revoke').at(-1);
    expect(revoke?.body?.token_type_hint).toBe('refresh_token');
    expect(revoke?.body?.client_id).toBe('oaiapp_e2e');
    // A later sign-in reuses the registration, or registers another account.
    const offersAnotherAccount = await browser.execute(
      (account: string) =>
        [...document.querySelectorAll(`${account} button`)].some(
          (button) => button.textContent.trim() === 'Use another account',
        ),
      ACCOUNT,
    );
    expect(offersAnotherAccount).toBe(true);
  });
});
