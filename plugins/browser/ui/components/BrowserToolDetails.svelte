<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import GlobeIcon from '@mannie/plugin-sdk/icons/globe';
  import type { ToolKindDetailsProps } from '@mannie/plugin-sdk/slots';

  import { m } from '../generated/messages.js';
  import { openBrowser } from '../utils/browser-panel.js';

  let { conversationId }: ToolKindDetailsProps = $props();
  let trigger = $state<HTMLElement | null>(null);
</script>

<Button
  class="self-start"
  size="sm"
  variant="outline"
  bind:ref={trigger}
  onclick={() => void openBrowser(conversationId, trigger)}
>
  <GlobeIcon data-icon="inline-start" />
  {m.browser_open_panel()}
</Button>
