<script lang="ts">
  import { m } from '#lib/paraglide/messages.js';
  import { formatCount } from '#lib/utils/chat-errors.js';
  import { changedFileName, type SavedFileChange } from '#lib/utils/tools-file-changes.js';

  let { file }: { file: SavedFileChange } = $props();
</script>

<span class="truncate" data-content dir="auto" title={m.tools_file_change_path({ path: file.path })}
  >{changedFileName(file.path)}</span
>
<span class="shrink-0 text-diff-added-foreground" data-file-added>
  <span aria-hidden="true">{m.tools_file_change_added_count({ count: formatCount(file.change.added) })}</span>
  <span class="sr-only">{m.tools_file_change_added({ count: formatCount(file.change.added) })}</span>
</span>
<span class="shrink-0 text-diff-removed-foreground" data-file-removed>
  <span aria-hidden="true">{m.tools_file_change_removed_count({ count: formatCount(file.change.removed) })}</span>
  <span class="sr-only">{m.tools_file_change_removed({ count: formatCount(file.change.removed) })}</span>
</span>
