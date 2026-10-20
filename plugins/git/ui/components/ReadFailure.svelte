<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import { m } from '../generated/messages.js';
  import type { GitFailure } from '../types/git.js';
  import { gitFailureMessage } from '../utils/git-messages.js';

  // A read of the Git panel that failed (plugins/git/docs/panel.md, loading,
  // empty and error states): what failed and what the owner can do, with
  // "Try again", which reads it again.
  interface Props {
    readonly failure: GitFailure;
    /** Reads the value again. */
    readonly retry: () => void;
    /** What could not be read, when the alert stands for the whole panel. */
    readonly title?: string;
  }

  let { failure, retry, title }: Props = $props();
</script>

<Alert.Root variant="destructive">
  <CircleAlertIcon />
  {#if title !== undefined}
    <Alert.Title>{title}</Alert.Title>
  {/if}
  <Alert.Description>{gitFailureMessage(failure)}</Alert.Description>
  <Alert.Action>
    <Button size="sm" variant="outline" onclick={retry}>{m.git_panel_try_again()}</Button>
  </Alert.Action>
</Alert.Root>
