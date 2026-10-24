// The words of attached context (ADR-0046, decision 6): what the chips of the
// message box, the parts of the transcript and the citations say of one
// provider's attachments and items, through the provider's kind of context
// while its plugin is on, and through the core's generic texts otherwise.
import PaperclipIcon from '@lucide/svelte/icons/paperclip';
import type { Component } from 'svelte';

import type { ContextLocation, ContextView } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import { getLocale } from '#lib/paraglide/runtime.js';
import type { ContextKind } from '#lib/types/shell-slots.js';

/** Finds the kind of context of a provider, as the slot of context kinds knows it. */
export type FindContextKind = (provider: string) => ContextKind | undefined;

/** The core's failure of a provider whose plugin was off. */
const PLUGIN_OFF = 'plugin_off';

/** The core's failure of a provider that took longer than the core waits. */
const TIMED_OUT = 'timed_out';

/**
 * Labels as one list, such as "Physics and Optics", in the language of the
 * interface, so that the list's "and" is in the language of the sentence
 * around it.
 */
export function labelList(labels: readonly string[]): string {
  return new Intl.ListFormat([getLocale()], { style: 'long', type: 'conjunction' }).format(labels);
}

/** The icon of a provider's attachments and parts. */
export function contextIcon(kind: ContextKind | undefined): Component {
  return kind?.icon ?? PaperclipIcon;
}

/** What one of a provider's attachments is, for screen readers. */
export function attachmentNoun(kind: ContextKind | undefined): string {
  return kind?.noun() ?? m.chat_context_attachment();
}

/** Why a provider gave nothing, as a sentence: the core's own failures, or the provider's code in its kind's words. */
export function contextFailure(kind: ContextKind | undefined, code: string): string {
  if (code === PLUGIN_OFF) {
    return m.chat_context_failure_plugin_off();
  }
  if (code === TIMED_OUT) {
    return m.chat_context_failure_timed_out();
  }
  return kind?.failure(code) ?? m.chat_context_failure_other();
}

/** The row of a part: what was searched, and what it gave or why it gave nothing. */
export function contextSummary(kind: ContextKind | undefined, context: ContextView): string {
  const labels = context.attachments.map((attachment) => attachment.label);
  if (context.failure !== null) {
    return m.chat_context_failed({ labels: labelList(labels), reason: contextFailure(kind, context.failure) });
  }
  if (context.items.length === 0) {
    return m.chat_context_nothing({ labels: labelList(labels) });
  }
  return (
    kind?.found(labels, context.items.length) ??
    m.chat_context_found({
      count: context.items.length,
      labels: labelList(labels),
    })
  );
}

/** Where an item stands, as the owner reads it, or `null` for an item without a place. */
export function locationText(location: ContextLocation | null): string | null {
  if (location === null) {
    return null;
  }
  switch (location.kind) {
    case 'headings': {
      return location.path.join(' › ');
    }
    case 'page': {
      return m.chat_context_page({ page: location.number });
    }
  }
}

/** Why a provider refused a key, as a sentence for the error of attaching it. */
export function contextRefusal(kind: ContextKind | undefined, code: string): string {
  if (code === TIMED_OUT) {
    return m.chat_context_failure_timed_out();
  }
  return kind?.failure(code) ?? m.chat_context_refusal_other();
}
