// The indicator beside the profile menu (docs/features/chatgpt-plan.md, in
// conversations), rendered on the server side: it shows only while the
// profile in effect uses the ChatGPT plan, "Using ChatGPT plan" with "Manage
// usage" while the account is signed in, and "Sign in to ChatGPT" otherwise.
import { render } from 'svelte/server';
import { afterEach, describe, expect, it, vi } from 'vitest';

import ChatGptPlanIndicator from '../../../../../../../../apps/desktop/src/lib/components/app/chat/ChatGptPlanIndicator.svelte';
import type {
  ChatGptAccountState,
  ProviderProfile,
} from '../../../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { chatgptAccount } from '../../../../../../../../apps/desktop/src/lib/state/chatgpt-account.svelte.js';
import { providerProfiles } from '../../../../../../../../apps/desktop/src/lib/state/provider-profiles.svelte.js';

vi.mock('../../../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: {} }));

const PLAN_PROFILE = '0199e0f2-0000-7000-8000-0000000000a1';
const ENDPOINT_PROFILE = '0199e0f2-0000-7000-8000-0000000000a2';

function profile(id: string, access: ProviderProfile['access'], isDefault: boolean): ProviderProfile {
  return {
    access,
    baseUrl: 'https://api.openai.com/v1',
    contextWindow: 400_000,
    createdAt: 0,
    hasCredential: false,
    id,
    interface: 'responses',
    isDefault,
    model: 'gpt-5',
    name: access === 'chatgpt_plan' ? 'ChatGPT' : 'Local',
    outputLimit: 128_000,
    reasoningEffort: 'medium',
    server: null,
    updatedAt: 0,
    useTools: true,
  };
}

/** The indicator of a conversation whose profile is `profileId`, or the default, with the account in `state`. */
function shown(profileId: string | null, state: ChatGptAccountState): string {
  providerProfiles.profiles = [
    profile(PLAN_PROFILE, 'chatgpt_plan', false),
    profile(ENDPOINT_PROFILE, 'endpoint', true),
  ];
  providerProfiles.status = 'ready';
  chatgptAccount.account = { email: null, known: true, noticeSeen: true, signIn: { step: 'idle' }, state };
  chatgptAccount.status = 'ready';
  return render(ChatGptPlanIndicator, { props: { profileId } }).body;
}

afterEach(() => {
  providerProfiles.profiles = [];
  providerProfiles.status = 'loading';
  chatgptAccount.account = null;
  chatgptAccount.status = 'loading';
});

describe('ChatGptPlanIndicator', () => {
  it('says that the reply uses the plan, with Manage usage, while the account is signed in', () => {
    const body = shown(PLAN_PROFILE, 'signed_in');

    expect(body).toContain('Using ChatGPT plan');
    expect(body).toContain('Manage usage');
    expect(body).not.toContain('Sign in to ChatGPT');
  });

  it('leads to the sign-in while the account is signed out or its session ended', () => {
    const signedOut = shown(PLAN_PROFILE, 'signed_out');
    const sessionEnded = shown(PLAN_PROFILE, 'sign_in_needed');

    for (const body of [signedOut, sessionEnded]) {
      expect(body).toContain('Sign in to ChatGPT');
      expect(body).not.toContain('Using ChatGPT plan');
    }
  });

  it('shows nothing for a profile on an endpoint, the default included', () => {
    const chosen = shown(ENDPOINT_PROFILE, 'signed_in');
    const byDefault = shown(null, 'signed_in');

    expect(chosen).not.toContain('data-chatgpt-indicator');
    expect(byDefault).not.toContain('data-chatgpt-indicator');
  });
});
