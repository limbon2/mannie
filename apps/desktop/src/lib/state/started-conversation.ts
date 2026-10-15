// The conversation that the first message of a new conversation started
// (docs/features/chat/conversation-view.md, the message box; projects.md, the
// sidebar). The chat view shows it in a pane of its own, in place of the new
// conversation's pane whose message box the owner wrote in, once the core
// has started it. That pane gives its message box the focus once, and until
// the view shows it, the sidebar counts it as the open conversation, since
// its reply can move it into a project before then.

/** The conversation that the latest first message started. */
let started: string | null = null;
/** Whether its pane still gives its message box the focus. */
let focusPending = false;

/** Records the conversation that a first message started, before the view shows it. */
export function markStartedConversation(conversationId: string): void {
  started = conversationId;
  focusPending = true;
}

/** The conversation that the latest first message started, if any. */
export function startedConversationId(): string | null {
  return started;
}

/** Whether the pane of a conversation gives its message box the focus: once, for the conversation that a first message started. */
export function takeComposerFocus(conversationId: string): boolean {
  const takesFocus = focusPending && started === conversationId;
  if (takesFocus) {
    focusPending = false;
  }
  return takesFocus;
}
