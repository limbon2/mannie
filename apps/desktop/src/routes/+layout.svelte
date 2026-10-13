<script lang="ts">
  import '../app.css';

  import { ModeWatcher } from 'mode-watcher';

  import Shell from '#lib/components/app/shell/Shell.svelte';
  import StartupProblemView from '#lib/components/app/shell/StartupProblemView.svelte';
  import { composeShell } from '#lib/registrations/compose-shell.js';
  import { setShellSlots } from '#lib/state/shell-slots.svelte.js';

  import type { LayoutProps } from './$types';

  let { children, data }: LayoutProps = $props();

  setShellSlots(composeShell());
</script>

<!--
  The head script and the transition blocker of mode-watcher inject inline
  code, which the Content Security Policy refuses (ADR-0004); static/theme.js
  sets the theme before the first paint instead.
-->
<ModeWatcher disableHeadScriptInjection disableTransitions={false} />
{#if data.problem === null}
  <Shell>
    {@render children()}
  </Shell>
{:else}
  <StartupProblemView problem={data.problem} />
{/if}
