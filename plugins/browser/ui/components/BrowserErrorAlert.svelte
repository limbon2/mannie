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
    onInspect?: () => void;
  }

  let { certificate = false, message, onDismiss, onInspect }: Props = $props();

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
    <Alert.Description>
      <p>{message}</p>

      {#if onInspect !== undefined}
        <div class="mt-2 flex flex-wrap gap-2">
          <Button data-browser-inspect-certificate="" size="sm" type="button" variant="outline" onclick={onInspect}>
            {m.browser_certificate_inspect()}
          </Button>
        </div>
      {/if}
    </Alert.Description>
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
