import type { FileChange, ToolCallView } from '#lib/generated/bindings.js';

/** A successful core file operation with a saved historical comparison. */
export interface SavedFileChange {
  change: FileChange;
  path: string;
}

export function savedFileChange(call: ToolCallView | null): SavedFileChange | null {
  if (call?.status !== 'completed') {
    return null;
  }
  if (call.plugin != null) {
    return null;
  }
  const outcome = call.result?.outcome;
  const written = outcome?.kind === 'written';
  const edited = outcome?.kind === 'edited';
  if (!written && !edited) {
    return null;
  }
  const expectedTool = written ? 'write_file' : 'edit_file';
  if (call.tool !== expectedTool) {
    return null;
  }
  if (outcome.change == null) {
    return null;
  }
  return { change: outcome.change, path: outcome.path };
}

/** Both host path separators work when viewing a worker's conversation. */
export function changedFileName(path: string): string {
  return path.split(/[\\/]/u).at(-1) ?? path;
}
