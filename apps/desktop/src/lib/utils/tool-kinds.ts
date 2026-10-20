// How the transcript shows the calls of plugins' tools (ADR-0039): through
// the tool kind that a call's own plugin registered for its tool, or with
// the core's texts without one, such as while the plugin is off. A kind
// reads the call's arguments, the data that its plugin returned for the card
// and the plugin's failure code; the accesses, the approval and the outputs
// that did not fit stay the core's.
import type { ToolCallView, ToolOutcome } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import type { ToolKind, ToolKindCall } from '#lib/types/shell-slots.js';

import { callArguments } from './tools-summaries.js';

/** Finds the tool kind that the plugin `plugin` registered for its tool `tool`. */
export type FindToolKind = (tool: string, plugin: string) => ToolKind | undefined;

type Fields = Readonly<Record<string, unknown>>;

/** The argument that the core adds to every call for the owner, which no tool reads. */
const ACTIVITY = 'activity';

function isFields(value: unknown): value is Fields {
  const isObject = typeof value === 'object' && value !== null;
  return isObject && !Array.isArray(value);
}

/** The tool kind of a call of a plugin's tool, when that plugin registered one for the tool. */
export function callToolKind(call: ToolCallView, findKind: FindToolKind): ToolKind | undefined {
  const plugin = call.plugin ?? null;
  return plugin === null ? undefined : findKind(call.tool, plugin);
}

/** The outcome of a finished call of a plugin's tool, or `null`. */
function pluginOutcome(call: ToolCallView): Extract<ToolOutcome, { kind: 'plugin' }> | null {
  const outcome = call.result?.outcome;
  return outcome?.kind === 'plugin' ? outcome : null;
}

/** The data of a call's card, the JSON object that the core kept, or `null`. */
function cardData(text: string | null): Fields | null {
  if (text === null) {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(text);
    return isFields(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

/** What a tool kind reads of a call. */
export function toolKindCall(call: ToolCallView): ToolKindCall {
  const outcome = pluginOutcome(call);
  const values = Object.entries(callArguments(call)).filter(([name]) => name !== ACTIVITY);
  return {
    arguments: Object.fromEntries(values),
    data: cardData(outcome?.data ?? null),
    failure: outcome?.failure ?? null,
    status: call.status,
  };
}

/** The plugin's failure code of a finished call of its tool, or `null`. */
export function pluginFailure(call: ToolCallView): string | null {
  return pluginOutcome(call)?.failure ?? null;
}

/** Why a call of a plugin's tool failed: its kind's sentence for the code, or the core's, which names the code. */
export function pluginFailureMessage(kind: ToolKind | undefined, code: string): string {
  return kind?.failure(code) ?? m.tools_plugin_failed({ code });
}
