// The transcript that the conversation view shows, built from the core's
// updates (docs/features/chat/conversations.md, transcript and events): a
// snapshot first, then events in the order of their sequence numbers. Events
// at or below the cursor were already applied and are skipped. The functions
// change the state in place, so that Svelte's state proxies update only what
// an event touches.
import type {
  CheckpointView,
  ContextUsage,
  ConversationEvent,
  ConversationSummary,
  ConversationUpdate,
  PartView,
  RunId,
  RunState,
  RunView,
  TurnView,
} from '#lib/generated/bindings.js';

/** What the conversation view shows of a conversation. */
export interface TranscriptState {
  /** The latest checkpoint. */
  checkpoint: CheckpointView | null;
  /** How much of the context the conversation uses, once known. */
  context: ContextUsage | null;
  /** The conversation as the list shows it. */
  conversation: ConversationSummary | null;
  /** The sequence number of the last event applied, the subscription's cursor. */
  cursor: number | null;
  /** Whether turns older than the first loaded one exist. */
  hasEarlierTurns: boolean;
  /** The loaded turns, oldest first. */
  turns: TurnView[];
}

const ACTIVE_STATES = new Set<RunState>(['queued', 'running']);
const RETRYABLE_STATES = new Set<RunState>(['failed', 'interrupted', 'stopped']);

/** What an update asks of the subscriber. */
export type UpdateOutcome = 'applied' | 'deleted' | 'resubscribe' | 'shutdown' | 'skipped';

/** A transcript before the first update. */
export function emptyTranscript(): TranscriptState {
  return {
    checkpoint: null,
    context: null,
    conversation: null,
    cursor: null,
    hasEarlierTurns: false,
    turns: [],
  };
}

function byPosition(left: { position: number }, right: { position: number }): number {
  return left.position - right.position;
}

/** Inserts or replaces an entry by its position, keeping the order. */
function putByPosition<T extends { position: number }>(entries: T[], entry: T): void {
  const index = entries.findIndex((existing) => existing.position === entry.position);
  if (index === -1) {
    entries.push(entry);
    entries.sort(byPosition);
  } else {
    entries[index] = entry;
  }
}

/** The turn whose current run wrote a part. Later turns are searched first, where streaming happens. */
function turnOfPart(state: TranscriptState, part: PartView): TurnView | undefined {
  return state.turns.findLast((turn) => {
    const isPartsTurn = turn.id === part.turnId;
    const isCurrentRun = part.runId === null || turn.run?.id === part.runId;
    return isPartsTurn && isCurrentRun;
  });
}

/** Whether a part stays when a retry starts a new run: the owner's message, and the parts of the runs the turn keeps. */
function staysOnRetry(part: PartView, kept: ReadonlySet<RunId>): boolean {
  if (part.runId === null) {
    return true;
  }
  return kept.has(part.runId);
}

function appendText(part: PartView, text: string): void {
  const content = part.content;
  switch (content.kind) {
    case 'reasoning':
    case 'text':
    case 'user_text': {
      content.text += text;
      return;
    }
    case 'error':
    case 'tool_call':
    case 'unknown': {
      // A tool call's arguments arrive whole, with the part's next update.
      return;
    }
  }
}

function applyEvent(state: TranscriptState, event: ConversationEvent): void {
  switch (event.kind) {
    case 'turn_started': {
      putByPosition(state.turns, event.turn);
      return;
    }
    case 'run_started': {
      // A retry replaces the parts of the turn's earlier runs. The owner's
      // message stays, and so do the parts of the runs that the turn keeps
      // because they made tool calls.
      const turn = state.turns.find((candidate) => candidate.id === event.run.turnId);
      if (turn !== undefined) {
        const kept = new Set(event.keptRunIds ?? []);
        turn.run = event.run;
        turn.parts = turn.parts.filter((part) => staysOnRetry(part, kept));
      }
      return;
    }
    case 'run_updated': {
      const turn = state.turns.findLast((candidate) => candidate.run?.id === event.run.id);
      if (turn !== undefined) {
        turn.run = event.run;
      }
      return;
    }
    case 'part_finished':
    case 'part_started':
    case 'part_updated': {
      const turn = turnOfPart(state, event.part);
      if (turn === undefined) {
        return;
      }
      if (event.part.state === 'discarded') {
        turn.parts = turn.parts.filter((part) => part.id !== event.part.id);
      } else {
        putByPosition(turn.parts, event.part);
      }
      return;
    }
    case 'part_delta': {
      for (const turn of state.turns.toReversed()) {
        const part = turn.parts.find((candidate) => candidate.id === event.partId);
        if (part !== undefined) {
          appendText(part, event.text);
          return;
        }
      }
      return;
    }
    case 'attempt_discarded': {
      const discarded = new Set(event.partIds);
      for (const turn of state.turns) {
        turn.parts = turn.parts.filter((part) => !discarded.has(part.id));
      }
      return;
    }
    case 'checkpoint_created': {
      state.checkpoint = event.checkpoint;
      return;
    }
    case 'context_usage_changed': {
      state.context = event.usage;
      return;
    }
    case 'conversation_changed': {
      state.conversation = event.conversation;
      return;
    }
    case 'unknown': {
      // An event of a newer version of Mannie changes nothing that this
      // version shows.
      return;
    }
  }
}

/** Applies one update of a conversation's subscription. */
export function applyUpdate(state: TranscriptState, update: ConversationUpdate): UpdateOutcome {
  switch (update.type) {
    case 'snapshot': {
      const { snapshot } = update;
      state.conversation = snapshot.conversation;
      state.turns = [...snapshot.turns].sort(byPosition);
      state.hasEarlierTurns = snapshot.hasEarlierTurns;
      state.checkpoint = snapshot.checkpoint;
      state.context = snapshot.context;
      state.cursor = snapshot.throughSeq;
      return 'applied';
    }
    case 'event': {
      const isApplied = state.cursor !== null && update.seq <= state.cursor;
      if (isApplied) {
        return 'skipped';
      }
      applyEvent(state, update.event);
      state.cursor = update.seq;
      return 'applied';
    }
    case 'ended': {
      switch (update.reason) {
        case 'deleted': {
          return 'deleted';
        }
        case 'lagged': {
          return 'resubscribe';
        }
        case 'shutdown': {
          return 'shutdown';
        }
      }
    }
  }
}

/** Adds older turns in front of the loaded ones. */
export function prependTurns(state: TranscriptState, turns: readonly TurnView[], hasEarlierTurns: boolean): void {
  const first = state.turns.at(0)?.position ?? Number.POSITIVE_INFINITY;
  const older = turns.filter((turn) => turn.position < first);
  state.turns = [...older, ...state.turns].sort(byPosition);
  state.hasEarlierTurns = hasEarlierTurns;
}

/** The latest turn, whose reply the status line and the message box follow. */
export function latestTurn(state: TranscriptState): TurnView | undefined {
  return state.turns.at(-1);
}

/** The latest turn's reply while it is being written. */
export function activeRun(state: TranscriptState): RunView | null {
  const run = latestTurn(state)?.run ?? null;
  if (run === null) {
    return null;
  }
  return ACTIVE_STATES.has(run.state) ? run : null;
}

/** Whether the latest turn's reply is still being written. */
export function hasActiveRun(state: TranscriptState): boolean {
  return activeRun(state) !== null;
}

/** The run with this ID once it has ended, or `null` while it runs or when no loaded turn has it. */
export function endedRun(state: TranscriptState, id: RunId): RunView | null {
  const run = state.turns.findLast((turn) => turn.run?.id === id)?.run ?? null;
  if (run === null) {
    return null;
  }
  return ACTIVE_STATES.has(run.state) ? null : run;
}

/** Whether the latest turn's reply can be retried: it failed, stopped or was interrupted. */
export function canRetry(state: TranscriptState): boolean {
  const run = latestTurn(state)?.run ?? null;
  if (run === null) {
    return false;
  }
  return RETRYABLE_STATES.has(run.state);
}
