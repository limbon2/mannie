<script lang="ts">
  import { call } from '@mannie/plugin-sdk/ipc';

  import { type BrowserFrame, commands } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { followSurface } from '../utils/browser-surface.js';

  interface Props {
    conversationId: string;
    tabId: string;
  }

  let { conversationId, tabId }: Props = $props();
  let still = $state.raw<BrowserFrame | null>(null);
  let hidden = $state(false);

  // Browser status changes can replace the tab record without changing the native surface's identity.
  const surfaceConversationId = $derived(conversationId);
  const surfaceTabId = $derived(tabId);
  const shownStill = $derived(hidden ? still : null);

  async function showStill(): Promise<void> {
    const outcome = await call(() => commands.browserStill({ conversationId, tabId }));
    if (hidden) {
      still = outcome.ok ? outcome.data : null;
    }
  }

  function surface(element: HTMLElement): () => void {
    return followSurface(element, {
      conversationId: surfaceConversationId,
      onHidden(covered) {
        if (covered === hidden) {
          return;
        }
        hidden = covered;
        if (covered) {
          void showStill();
        } else {
          still = null;
        }
      },
      tabId: surfaceTabId,
    });
  }
</script>

<!-- The tab's native page sits over this placeholder; WebView2 paints it. -->
<div
  class="relative min-h-0 flex-1 overflow-hidden rounded-md border bg-muted/30"
  {@attach surface}
  aria-label={m.browser_page_label()}
  data-browser-viewport=""
  role="region"
>
  {#if shownStill !== null}
    <img
      class="pointer-events-none size-full object-contain object-top-left select-none"
      alt={m.browser_still_label()}
      draggable="false"
      height={shownStill.height}
      src={`data:image/jpeg;base64,${shownStill.jpeg}`}
      width={shownStill.width}
    />
  {/if}
</div>
