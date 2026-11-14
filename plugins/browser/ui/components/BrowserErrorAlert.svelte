<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import XIcon from '@mannie/plugin-sdk/icons/x';

  import { m } from '../generated/messages.js';

  interface Props {
    certificate?: boolean;
    message: string;
    onDismiss: () => void;
  }

  let { certificate = false, message, onDismiss }: Props = $props();
  let dismissed = $state(false);

  function dismiss(): void {
    dismissed = true;
    onDismiss();
  }
</script>

{#if !dismissed}
  <Alert.Root data-browser-certificate-error={certificate ? '' : undefined} variant="destructive">
    <CircleAlertIcon />
    <Alert.Title>{m.browser_error_title()}</Alert.Title>
    <Alert.Description>{message}</Alert.Description>
    <Alert.Action>
      <Button
        aria-label={m.browser_dismiss_error()}
        data-browser-dismiss-certificate-error={certificate ? '' : undefined}
        data-browser-dismiss-error={certificate ? undefined : ''}
        size="icon-xs"
        title={m.browser_dismiss_error()}
        variant="ghost"
        onclick={dismiss}
      >
        <XIcon />
      </Button>
    </Alert.Action>
  </Alert.Root>
{/if}
