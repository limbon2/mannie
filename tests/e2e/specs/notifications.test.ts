// Desktop notifications, unread marks, the tray badge and the taskbar count
// (docs/features/notifications.md; ADR-0074; V14): unfocused questions,
// approvals, completions and failures record notifications carrying only the
// catalog title and cleaned conversation title, badge the tray and taskbar,
// mark finished conversations and collapsed projects unread, clear only when
// opened in a focused window or started anew, open the conversation on click,
// and follow Settings -> Notifications.
import { mkdtempSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { Key } from 'webdriverio';

import {
  addProfile,
  addProject,
  callMessage,
  clickInLatestTurn,
  COMPOSER,
  LATEST_MAIN_TURN,
  projectToggle,
  sendMessage,
  startNewConversation,
  useProfile,
  waitForCompleteReply,
  waitForLatestPart,
  waitForToolStatus,
} from '../support/chat.ts';
import {
  clickNotification,
  deleteProviderProfiles,
  notificationsState,
  type RecordedNotification,
  setWindowFocused,
  type ValidationNotificationsState,
} from '../support/harness.ts';
import { axeViolations, leaveSettings, openSettings, waitForAnimations, waitForApp } from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '../support/provider-server.ts';

const WHICH_DATABASE = {
  activity: 'Asking which database to use',
  questions: [
    {
      multiple: false,
      options: [
        { description: 'It has the JSON queries.', label: 'PostgreSQL' },
        { description: '', label: 'SQLite' },
      ],
      question: 'Which database should the service use?',
    },
  ],
};

/** Runs axe in the light and dark themes. */
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

/** The conversation ID from the current URL query string, or null. */
async function currentConversationId(): Promise<string | null> {
  return browser.execute(() => new URLSearchParams(location.search).get('conversation'));
}

/** Whether the tray and taskbar are badged with `expectedCount` and `conversationId` is unread. */
function isBadgedAndUnread(
  state: ValidationNotificationsState,
  conversationId: string | null,
  expectedCount = 1,
): boolean {
  if (conversationId === null) {
    return false;
  }
  if (!state.trayBadged) {
    return false;
  }
  if (state.taskbarBadgeCount !== expectedCount) {
    return false;
  }
  return state.unreadConversationIds.includes(conversationId);
}

/** Whether both badges are off and no unread conversations remain. */
function isCleared(state: ValidationNotificationsState): boolean {
  if (state.trayBadged) {
    return false;
  }
  if (state.taskbarBadgeCount !== 0) {
    return false;
  }
  return state.unreadConversationIds.length === 0;
}

/** Whether both badges are off and `conversationId` is no longer marked unread. */
function isClearedFor(state: ValidationNotificationsState, conversationId: string | null): boolean {
  if (state.trayBadged) {
    return false;
  }
  if (state.taskbarBadgeCount !== 0) {
    return false;
  }
  if (conversationId === null) {
    return true;
  }
  return !state.unreadConversationIds.includes(conversationId);
}

/** Whether `conversationId` is read while `remainingCount` unread conversations keep the badges on. */
function isDecrementedFor(
  state: ValidationNotificationsState,
  conversationId: string | null,
  remainingCount: number,
): boolean {
  if (conversationId === null) {
    return false;
  }
  if (!state.trayBadged) {
    return false;
  }
  if (state.taskbarBadgeCount !== remainingCount) {
    return false;
  }
  return !state.unreadConversationIds.includes(conversationId);
}

/** Waits until the validation notifications state satisfies `predicate`. */
async function waitForNotificationsState(
  predicate: (state: ValidationNotificationsState) => boolean,
  message: string,
): Promise<ValidationNotificationsState> {
  let latest = await notificationsState();
  await browser.waitUntil(
    async () => {
      latest = await notificationsState();
      return predicate(latest);
    },
    { timeout: 15_000, timeoutMsg: `${message}; last state: ${JSON.stringify(latest)}` },
  );
  return latest;
}

/** Waits for a recorded notification after `previousCount` matching `kind`. */
async function waitForNotification(
  previousCount: number,
  kind: RecordedNotification['kind'],
): Promise<RecordedNotification> {
  let found: RecordedNotification | undefined;
  await waitForNotificationsState(
    (state) => {
      found = state.notifications.slice(previousCount).find((item) => item.kind === kind);
      return found !== undefined;
    },
    `no ${kind} notification was recorded after index ${String(previousCount)}`,
  );
  if (found === undefined) {
    throw new Error(`unreachable: ${kind} notification missing`);
  }
  return found;
}

/** Toggles a switch in Settings -> Notifications and waits for the saved state. */
async function setSwitch(selector: string, checked: boolean): Promise<void> {
  const control = await $(selector);
  await control.waitForDisplayed();
  const wanted = checked ? 'checked' : 'unchecked';
  if ((await control.getAttribute('data-state')) === wanted) {
    return;
  }
  await control.click();
  await expect(control).toHaveAttribute('data-state', wanted);
  await expect(control).toBeEnabled();
}

describe('desktop notifications, tray badge and taskbar count', () => {
  let provider: ProviderServer;

  before(async () => {
    provider = await startProviderServer();

    await waitForApp();
    await addProfile(provider.baseUrl, { model: MODELS.echo, name: 'Echo' });
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'Tools', useTools: true });
    await addProfile(provider.baseUrl, { model: MODELS.unauthorized, name: 'Unauthorized' });
  });

  after(async () => {
    await deleteProviderProfiles();
    await provider.close();
  });

  it('notifies, badges the tray, counts unread conversations on the taskbar, and decrements only as each is opened while focused', async () => {
    const before = (await notificationsState()).notifications.length;
    await setWindowFocused(false);
    await startNewConversation();
    await sendMessage('Quarterly roadmap summary');
    await waitForCompleteReply();

    const firstId = await currentConversationId();
    expect(firstId).not.toBeNull();
    const completed = await waitForNotification(before, 'completed');
    expect(completed.title).toBe('Reply finished');
    expect(completed.body).toContain('Quarterly roadmap summary');
    expect(completed.body).not.toContain('Reply to:');
    expect(completed.conversationId).toBe(firstId);

    const firstBadged = await waitForNotificationsState(
      (state) => isBadgedAndUnread(state, firstId, 1),
      'first unfocused completion did not mark conversation unread and set taskbar count to 1',
    );
    expect(firstBadged.trayBadged).toBe(true);
    expect(firstBadged.taskbarBadgeCount).toBe(1);
    await expect($(`[data-conversation="${firstId ?? ''}"] [data-unread-mark]`)).toBeExisting();

    // A second unfocused completion raises the taskbar count to 2.
    await startNewConversation();
    await sendMessage('Release checklist notes');
    await waitForCompleteReply();

    const secondId = await currentConversationId();
    expect(secondId).not.toBeNull();
    expect(secondId).not.toBe(firstId);
    const secondBadged = await waitForNotificationsState(
      (state) => isBadgedAndUnread(state, secondId, 2),
      'second unfocused completion did not raise taskbar count to 2',
    );
    expect(secondBadged.trayBadged).toBe(true);
    expect(secondBadged.taskbarBadgeCount).toBe(2);
    await expect($(`[data-conversation="${secondId ?? ''}"] [data-unread-mark]`)).toBeExisting();

    // Navigating away and focusing the window must keep both unread marks, the tray badge and count 2.
    await startNewConversation();
    await setWindowFocused(true);
    const stillUnread = await notificationsState();
    expect(stillUnread.trayBadged).toBe(true);
    expect(stillUnread.taskbarBadgeCount).toBe(2);
    expect(stillUnread.unreadConversationIds).toContain(firstId);
    expect(stillUnread.unreadConversationIds).toContain(secondId);
    await expect($(`[data-conversation="${firstId ?? ''}"] [data-unread-mark]`)).toBeExisting();
    await expect($(`[data-conversation="${secondId ?? ''}"] [data-unread-mark]`)).toBeExisting();
    await axeInBothThemes();

    // Opening the first unread conversation while focused drops the taskbar count from 2 to 1.
    await $(`[data-conversation="${firstId ?? ''}"] a`).click();
    await waitForNotificationsState(
      (state) => isDecrementedFor(state, firstId, 1),
      'opening the first unread conversation did not decrement taskbar count to 1',
    );
    await expect($(`[data-conversation="${firstId ?? ''}"] [data-unread-mark]`)).not.toBeExisting();
    await expect($(`[data-conversation="${secondId ?? ''}"] [data-unread-mark]`)).toBeExisting();

    // Opening the second unread conversation clears the last dot, the tray badge and the taskbar count.
    await $(`[data-conversation="${secondId ?? ''}"] a`).click();
    await waitForNotificationsState(
      (state) => isClearedFor(state, secondId),
      'opening the last unread conversation while focused did not clear the badges',
    );
    await expect($(`[data-conversation="${secondId ?? ''}"] [data-unread-mark]`)).not.toBeExisting();
  });

  it('shows the unread mark on a collapsed project until its conversation is opened while focused', async () => {
    const folder = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-e2e-notify-project-')));
    try {
      const projectId = await addProject(folder);
      await setWindowFocused(false);
      await sendMessage('Project build check');
      await waitForCompleteReply();

      const projectConversation = await currentConversationId();
      expect(projectConversation).not.toBeNull();
      await waitForNotificationsState(
        (state) => isBadgedAndUnread(state, projectConversation),
        'project conversation did not become unread',
      );

      // Leave the project conversation and collapse its project row.
      await startNewConversation();
      await projectToggle(projectId).click();
      await waitForAnimations();
      await expect($(`[data-project="${projectId}"] [data-project-toggle] [data-unread-mark]`)).toBeExisting();
      await setWindowFocused(true);
      await axeInBothThemes();

      // Expand the project and open the unread conversation while focused.
      await projectToggle(projectId).click();
      await $(`[data-conversation="${projectConversation ?? ''}"] a`).click();
      await waitForNotificationsState(isCleared, 'opening the project conversation did not clear unread state');
      await expect($(`[data-project="${projectId}"] [data-unread-mark]`)).not.toBeExisting();
    } finally {
      rmSync(folder, { force: true, recursive: true });
    }
  });

  it('notifies for an unfocused question without leaking question text, opens it on click, and stays silent while focused', async () => {
    await startNewConversation();
    await useProfile('Tools');
    await setWindowFocused(true);
    await sendMessage('Database setup');
    await waitForCompleteReply();

    const before = (await notificationsState()).notifications.length;
    await setWindowFocused(false);
    await sendMessage(callMessage('ask_owner', WHICH_DATABASE));
    await waitForToolStatus('awaiting_answer');

    const questionConversation = await currentConversationId();
    expect(questionConversation).not.toBeNull();
    const note = await waitForNotification(before, 'question');
    expect(note.title).toBe('Waiting for your answer');
    expect(note.conversationId).toBe(questionConversation);
    expect(note.body).toBe('Database setup');
    expect(note.body).not.toContain('Which database should the service use?');
    expect(note.body).not.toContain('PostgreSQL');

    const waitingState = await notificationsState();
    expect(waitingState.trayBadged).toBe(true);
    expect(waitingState.taskbarBadgeCount).toBe(1);

    // Move to another conversation, then simulate clicking the OS notification.
    await startNewConversation();
    expect(await currentConversationId()).not.toBe(questionConversation);
    expect(await clickNotification(note.id)).toBe(true);

    await browser.waitUntil(async () => (await currentConversationId()) === questionConversation, {
      timeout: 10_000,
      timeoutMsg: 'clicking the notification did not open its conversation',
    });
    const card = `${LATEST_MAIN_TURN} [data-question-card]`;
    await $(card).waitForDisplayed();

    // Answer while focused: the finished reply must neither notify nor leave a badge.
    await setWindowFocused(true);
    const countBeforeAnswer = (await notificationsState()).notifications.length;
    await browser.execute((selector: string) => {
      const labels = [...document.querySelectorAll<HTMLElement>(`${selector} label`)];
      labels.find((label) => label.textContent.trim() === 'SQLite')?.click();
    }, card);
    await clickInLatestTurn('Send answer');
    await waitForCompleteReply();

    const afterAnswer = await waitForNotificationsState(
      isCleared,
      'answering and completing a focused conversation left the tray badged',
    );
    expect(afterAnswer.notifications).toHaveLength(countBeforeAnswer);
  });

  it('notifies for an unfocused tool approval without leaking arguments, and stopping the run clears the badge silently', async () => {
    const tempPath = join(tmpdir(), 'mannie-e2e-notify-approval.txt');
    await startNewConversation();
    await useProfile('Tools');
    await setWindowFocused(true);
    await sendMessage('Approval check');
    await waitForCompleteReply();

    const before = (await notificationsState()).notifications.length;
    await setWindowFocused(false);
    await sendMessage(
      callMessage('write_file', {
        activity: 'Writing private draft',
        content: 'SECRET-PAYLOAD-DO-NOT-NOTIFY',
        path: tempPath,
      }),
    );
    await waitForToolStatus('awaiting_approval');

    const approval = await waitForNotification(before, 'approval');
    expect(approval.title).toBe('Approval needed');
    expect(approval.body).toBe('Approval check');
    expect(approval.body).not.toContain('SECRET-PAYLOAD-DO-NOT-NOTIFY');
    expect(approval.body).not.toContain('Writing private draft');
    const approvalState = await notificationsState();
    expect(approvalState.trayBadged).toBe(true);
    expect(approvalState.taskbarBadgeCount).toBe(1);

    const countAfterApproval = (await notificationsState()).notifications.length;
    // Stopping the reply cancels the approval and must not send a completion/failure notification or mark unread.
    await $(COMPOSER).click();
    await browser.keys(Key.Escape);
    await waitForToolStatus('canceled');

    const afterStop = await waitForNotificationsState(
      isCleared,
      'stopping an awaiting approval run did not clear the tray badge',
    );
    expect(afterStop.notifications).toHaveLength(countAfterApproval);
  });

  it('notifies for an unfocused failure without leaking credentials and keeps unread marks across reload', async () => {
    const before = (await notificationsState()).notifications.length;
    await startNewConversation();
    await useProfile('Unauthorized');
    await setWindowFocused(false);
    await sendMessage('Check unauthorized provider');
    await waitForLatestPart('error', ['complete']);

    const failedConversation = await currentConversationId();
    expect(failedConversation).not.toBeNull();
    const failure = await waitForNotification(before, 'failed');
    expect(failure.title).toBe('Reply failed');
    expect(failure.conversationId).toBe(failedConversation);
    expect(failure.body).not.toContain('e2e-credential');
    expect(failure.body).not.toContain('Incorrect API key');

    await waitForNotificationsState(
      (state) => isBadgedAndUnread(state, failedConversation),
      'failed unfocused run did not mark conversation unread',
    );

    // Move away and reload the page: SQLite unread persistence must restore the unread dot and badge.
    await startNewConversation();
    await browser.refresh();
    await waitForApp();
    await waitForNotificationsState(
      (state) => isBadgedAndUnread(state, failedConversation),
      'unread mark and tray badge did not survive reload',
    );
    await expect($(`[data-conversation="${failedConversation ?? ''}"] [data-unread-mark]`)).toBeExisting();
  });

  it('persists Settings -> Notifications switches, disables event switches under master off, and controls both badges independently', async () => {
    await openSettings('notifications.settings');
    await $('[data-notifications-section]').waitForDisplayed();
    await axeInBothThemes();

    for (const id of [
      '#notifications-enabled',
      '#notifications-on-question',
      '#notifications-on-approval',
      '#notifications-on-completion',
      '#notifications-on-failure',
      '#notifications-taskbar-badge',
      '#notifications-tray-badge',
    ]) {
      await expect($(id)).toHaveAttribute('data-state', 'checked');
    }

    // Turning off the tray badge clears it right away while leaving the taskbar count and unread marks intact.
    await $('#notifications-tray-badge').click();
    await expect($('#notifications-tray-badge')).toHaveAttribute('data-state', 'unchecked');
    const afterTrayOff = await waitForNotificationsState(
      (state) => !state.trayBadged,
      'turning off Show badge on tray icon did not clear the tray badge',
    );
    expect(afterTrayOff.taskbarBadgeCount).toBe(1);
    expect(afterTrayOff.unreadConversationIds.length).toBeGreaterThan(0);

    // Turning off the taskbar count clears it right away while leaving unread marks intact.
    await $('#notifications-taskbar-badge').click();
    await expect($('#notifications-taskbar-badge')).toHaveAttribute('data-state', 'unchecked');
    const afterTaskbarOff = await waitForNotificationsState(
      (state) => state.taskbarBadgeCount === 0,
      'turning off Show count on taskbar icon did not clear the taskbar count',
    );
    expect(afterTaskbarOff.trayBadged).toBe(false);
    expect(afterTaskbarOff.unreadConversationIds.length).toBeGreaterThan(0);

    // Turning off the master switch from the keyboard disables the four event switches
    // while leaving the two badge switches enabled.
    await browser.execute(() => {
      document.querySelector<HTMLElement>('#notifications-enabled')?.focus();
    });
    await browser.keys(Key.Space);
    await expect($('#notifications-enabled')).toHaveAttribute('data-state', 'unchecked');
    for (const id of [
      '#notifications-on-question',
      '#notifications-on-approval',
      '#notifications-on-completion',
      '#notifications-on-failure',
    ]) {
      await expect($(id)).toBeDisabled();
    }
    await expect($('#notifications-taskbar-badge')).toBeEnabled();
    await expect($('#notifications-tray-badge')).toBeEnabled();
    await axeInBothThemes();

    // Reload session and confirm the switches stayed persisted.
    await browser.reloadSession();
    await waitForApp();
    await openSettings('notifications.settings');
    await $('[data-notifications-section]').waitForDisplayed();
    await expect($('#notifications-enabled')).toHaveAttribute('data-state', 'unchecked');
    await expect($('#notifications-taskbar-badge')).toHaveAttribute('data-state', 'unchecked');
    await expect($('#notifications-tray-badge')).toHaveAttribute('data-state', 'unchecked');

    // With desktop notifications off and window unfocused, a completed reply records no notification.
    await leaveSettings();
    const beforeMuted = (await notificationsState()).notifications.length;
    await setWindowFocused(false);
    await startNewConversation();
    await useProfile('Echo');
    await sendMessage('Silent completion check');
    await waitForCompleteReply();
    const mutedId = await currentConversationId();
    await waitForNotificationsState(
      (state) => state.unreadConversationIds.includes(mutedId ?? ''),
      'unfocused completion did not mark conversation unread while desktop notifications were off',
    );
    const mutedState = await notificationsState();
    expect(mutedState.notifications).toHaveLength(beforeMuted);
    expect(mutedState.trayBadged).toBe(false);
    expect(mutedState.taskbarBadgeCount).toBe(0);

    // Turn all three switches back on: both badges immediately reflect the unread conversations,
    // and focused completions stay silent and read.
    await openSettings('notifications.settings');
    await setSwitch('#notifications-enabled', true);
    await setSwitch('#notifications-taskbar-badge', true);
    await setSwitch('#notifications-tray-badge', true);
    const restored = await waitForNotificationsState(
      (state) => isBadgedAndUnread(state, mutedId, state.unreadConversationIds.length),
      'turning the badge switches back on did not restore the tray badge and taskbar count',
    );
    expect(restored.taskbarBadgeCount).toBeGreaterThanOrEqual(2);
    await leaveSettings();

    const beforeFocused = (await notificationsState()).notifications.length;
    await setWindowFocused(true);
    await startNewConversation();
    await useProfile('Echo');
    await sendMessage('Focused completion check');
    await waitForCompleteReply();
    const focusedId = await currentConversationId();
    const focusedState = await notificationsState();
    expect(focusedState.notifications).toHaveLength(beforeFocused);
    expect(focusedState.unreadConversationIds).not.toContain(focusedId);
  });
});
