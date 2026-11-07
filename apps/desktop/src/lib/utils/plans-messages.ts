// The words of plans (docs/features/plans.md): the mention kind `plans`, and why
// a command of plans failed.
import ClipboardListIcon from '@lucide/svelte/icons/clipboard-list';
import type { Component } from 'svelte';

import type { MentionSummary, PlanError } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import type { Failure } from '#lib/types/calls.js';

/** The kind of side panel that shows a plan, with its number as its argument. */
export const PLAN_PANEL = 'plans.plan';

/** The icon of a plan's mention. */
export function planMentionIcon(): Component {
  return ClipboardListIcon;
}

/** What an entry is, for screen readers. */
export function planMentionVariantName(): string {
  return m.plans_mention_variant();
}

/** Why the provider refused a mention or listed nothing. */
export function planMentionRefusal(code: string): string | null {
  switch (code) {
    case 'plan_not_found': {
      return m.plans_mention_not_found();
    }
    case 'plan_unreadable': {
      return m.plans_mention_unreadable();
    }
    default: {
      return null;
    }
  }
}

/** What the model got of a sent mention. */
export function planMentionSummary(summary: MentionSummary): string | null {
  if (summary.code === 'cut') {
    return m.plans_mention_summary_cut({
      shown: String(summary.values?.shown ?? 0),
      total: String(summary.values?.total ?? 0),
    });
  }
  return m.plans_mention_summary_whole();
}

/** Why a command of plans failed, and what the owner can do. */
export function planFailureMessage(failure: Failure<PlanError>): string {
  if (failure.kind === 'ipc') {
    return m.shell_error_ipc_failed();
  }
  const { error } = failure;
  switch (error.code) {
    case 'field_not_valid': {
      return error.field === 'title' ? m.plans_error_title() : m.plans_error_body();
    }
    case 'not_found': {
      return m.plans_error_not_found();
    }
    case 'storage_failed': {
      return m.plans_error_storage_failed();
    }
    case 'store_unavailable': {
      return m.plans_error_store_unavailable();
    }
  }
}
