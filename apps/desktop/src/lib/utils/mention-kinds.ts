// How the "@" list, the message box's chips and the transcript name a
// mention (ADR-0032, decision 4): through the entry of its provider in the
// slot of mention kinds, or generic texts without one, such as while the
// provider's plugin is off.
import AtSignIcon from '@lucide/svelte/icons/at-sign';
import type { Component } from 'svelte';

import type { MentionSummary } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import type { MentionKind } from '#lib/types/shell-slots.js';

/** Finds the kind of a provider, as the slot of mention kinds knows it. */
export type FindMentionKind = (provider: string) => MentionKind | undefined;

/** The order of a group that no kind places, after every kind's. */
const UNPLACED = Number.MAX_SAFE_INTEGER;

/** The heading of a provider's group in the list. */
export function mentionGroupHeading(kind: MentionKind | undefined): string {
  return kind?.group() ?? m.chat_mention_group_other();
}

/** The icon of an entry of a variant. */
export function mentionIcon(kind: MentionKind | undefined, variant: string): Component {
  return kind?.icon(variant) ?? AtSignIcon;
}

/** What an entry of a variant is, for screen readers. */
export function mentionVariantName(kind: MentionKind | undefined, variant: string): string {
  return kind?.variantName(variant) ?? m.chat_mention_variant_other();
}

/** The words of a warning on an entry or a mention. */
export function mentionWarning(kind: MentionKind | undefined, code: string): string {
  return kind?.warning(code) ?? m.chat_mention_warning_other();
}

/** Why a provider listed nothing or refused a mention. */
export function mentionRefusal(kind: MentionKind | undefined, code: string): string {
  return kind?.refusal(code) ?? m.chat_mention_refusal_other();
}

/** What the model got of a sent mention, or `null` when its kind does not say. */
export function mentionSummary(kind: MentionKind | undefined, summary: MentionSummary): string | null {
  return kind?.summary(summary) ?? null;
}

/** The providers of the list's groups, sorted by the order of their kinds. Providers that no kind places come last, as the core sent them. */
export function sortMentionProviders(providers: readonly string[], findKind: FindMentionKind): string[] {
  const placed = providers.map((provider, index) => ({
    index,
    order: findKind(provider)?.order ?? UNPLACED,
    provider,
  }));
  placed.sort((left, right) => left.order - right.order || left.index - right.index);
  return placed.map((entry) => entry.provider);
}
