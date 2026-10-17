// The words of the core's mentions of files and folders (docs/features/chat/
// mentions.md), which chat registers as the mention kind `files`: the
// provider's variants, its warning, its refusals and what the model got of a
// sent mention.
import FileIcon from '@lucide/svelte/icons/file';
import FolderIcon from '@lucide/svelte/icons/folder';
import type { Component } from 'svelte';

import type { MentionSummary } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';

/** The ID of the core's provider of files and folders. */
export const FILES_PROVIDER = 'files';

/** The icon of an entry. */
export function fileMentionIcon(variant: string): Component {
  return variant === 'folder' ? FolderIcon : FileIcon;
}

/** What an entry is, for screen readers. */
export function fileMentionVariantName(variant: string): string | null {
  switch (variant) {
    case 'file': {
      return m.chat_mention_files_file();
    }
    case 'folder': {
      return m.chat_mention_files_folder();
    }
    default: {
      return null;
    }
  }
}

/** The words of a warning. */
export function fileMentionWarning(code: string): string | null {
  return code === 'secret' ? m.chat_mention_files_secret() : null;
}

/** Why the provider refused a mention or listed nothing. */
export function fileMentionRefusal(code: string): string | null {
  switch (code) {
    case 'folder_unreadable': {
      return m.chat_mention_files_folder_unreadable();
    }
    case 'no_workspace': {
      return m.chat_mention_files_no_workspace();
    }
    case 'outside_workspace': {
      return m.chat_mention_files_outside_workspace();
    }
    default: {
      return null;
    }
  }
}

/** One of the numbers of a summary, or 0 when the core did not send it. */
function value(summary: MentionSummary, name: string): number {
  return summary.values?.[name] ?? 0;
}

/** What the lines of a file's page say. */
function linesSummary(summary: MentionSummary): string {
  const first = value(summary, 'first');
  const last = value(summary, 'last');
  const total = value(summary, 'total');
  if (total === 0) {
    return m.chat_mention_files_empty_file();
  }
  if (last < first) {
    return m.chat_mention_files_lines_none({ total });
  }
  const coversTheFile = first === 1 && last === total;
  if (coversTheFile) {
    return m.chat_mention_files_lines({ count: total });
  }
  return m.chat_mention_files_lines_cut({ first, last, total });
}

/** What the model got of a sent mention of a file or folder. */
export function fileMentionSummary(summary: MentionSummary): string | null {
  switch (summary.code) {
    case 'entries': {
      const shown = value(summary, 'shown');
      return shown === 0 ? m.chat_mention_files_empty_folder() : m.chat_mention_files_entries({ count: shown });
    }
    case 'entries_cut': {
      return m.chat_mention_files_entries_cut({ count: value(summary, 'shown') });
    }
    case 'lines': {
      return linesSummary(summary);
    }
    case 'needs_approval': {
      return m.chat_mention_files_needs_approval();
    }
    case 'not_found': {
      return m.chat_mention_files_not_found();
    }
    case 'not_text': {
      return m.chat_mention_files_not_text();
    }
    case 'too_large': {
      return m.chat_mention_files_too_large({ limit: value(summary, 'limitMebibytes') });
    }
    case 'unreadable': {
      return m.chat_mention_files_unreadable();
    }
    default: {
      return null;
    }
  }
}
