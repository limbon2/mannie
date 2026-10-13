<script lang="ts">
  import DatabaseIcon from '@lucide/svelte/icons/database';

  import * as Empty from '#lib/components/ui/empty/index.js';
  import type { StartupProblem } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { startupProblemMessage } from '#lib/utils/error-messages.js';

  let { problem }: { problem: StartupProblem } = $props();
</script>

<svelte:head>
  <title>{m.shell_startup_title()}</title>
</svelte:head>

<main class="flex min-h-svh items-center justify-center p-6">
  <Empty.Root class="max-w-xl">
    <Empty.Header>
      <Empty.Media variant="icon">
        <DatabaseIcon />
      </Empty.Media>
      <Empty.Title>
        <h1 class="text-base font-semibold">{m.shell_startup_title()}</h1>
      </Empty.Title>
      <Empty.Description class="wrap-break-word">{startupProblemMessage(problem)}</Empty.Description>
    </Empty.Header>
  </Empty.Root>
</main>
