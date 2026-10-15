<script lang="ts">
  import PanelRightIcon from '@lucide/svelte/icons/panel-right';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as Empty from '#lib/components/ui/empty/index.js';
  import { m } from '#lib/paraglide/messages.js';

  // A panel of a kind that no feature registered, such as one that a newer
  // version of Mannie put in the address, shows this body instead of failing
  // the side panel area (ADR-0025, decision 6).
  interface Props {
    /** Closes the panel. */
    close: () => void;
    /** The kind that the address names. */
    kind: string;
  }

  let { close, kind }: Props = $props();
</script>

<Empty.Root class="flex-1">
  <Empty.Header>
    <Empty.Media variant="icon">
      <PanelRightIcon />
    </Empty.Media>
    <Empty.Title>{m.shell_side_panel_unknown_title()}</Empty.Title>
    <Empty.Description>{m.shell_side_panel_unknown_description({ kind })}</Empty.Description>
  </Empty.Header>
  <Empty.Content>
    <Button variant="outline" onclick={close}>{m.shell_side_panel_close()}</Button>
  </Empty.Content>
</Empty.Root>
