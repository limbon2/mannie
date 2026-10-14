<script lang="ts">
  import FileQuestionIcon from '@lucide/svelte/icons/file-question';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { PartRendererProps } from '#lib/types/shell-slots.js';
  import { partKind } from '#lib/utils/parts.js';

  // Parts of a kind that no feature renders, such as a kind from a newer
  // version of Mannie, show as a note instead of failing the transcript
  // (ADR-0020).
  let { part }: PartRendererProps = $props();
</script>

<Alert.Root>
  <FileQuestionIcon />
  <Alert.Description>{m.shell_part_unknown({ kind: partKind(part) })}</Alert.Description>
</Alert.Root>
