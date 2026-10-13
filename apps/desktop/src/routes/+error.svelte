<script lang="ts">
  import UnplugIcon from '@lucide/svelte/icons/unplug';

  import * as Empty from '#lib/components/ui/empty/index.js';
  import { m } from '#lib/paraglide/messages.js';
</script>

<!--
  Shown when the window could not reach the core while starting, the only
  error the root layout lets through.
-->
<svelte:head>
  <title>{m.shell_startup_title()}</title>
</svelte:head>

<main class="flex min-h-svh items-center justify-center p-6">
  <Empty.Root class="max-w-xl">
    <Empty.Header>
      <Empty.Media variant="icon">
        <UnplugIcon />
      </Empty.Media>
      <Empty.Title>
        <h1 class="text-base font-semibold">{m.shell_startup_title()}</h1>
      </Empty.Title>
      <Empty.Description>{m.shell_error_ipc_failed()}</Empty.Description>
    </Empty.Header>
  </Empty.Root>
</main>
