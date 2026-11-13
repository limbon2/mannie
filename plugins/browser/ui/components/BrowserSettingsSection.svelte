<script lang="ts">
  import { browserSettings } from '../state/browser-settings.svelte.js';

  import BrowserCertificateExceptions from './BrowserCertificateExceptions.svelte';
  import BrowserCredentials from './BrowserCredentials.svelte';

  let certificates = $state<HTMLElement | null>(null);
  let credentials = $state<HTMLElement | null>(null);

  $effect(() => {
    const target = browserSettings.focus;
    if (target !== null) {
      const heading = target === 'certificates' ? certificates : credentials;
      if (heading !== null) {
        heading.focus();
        browserSettings.focus = null;
      }
    }
  });
</script>

<div class="flex flex-col gap-8" data-browser-settings="">
  <BrowserCertificateExceptions bind:heading={certificates} />
  <BrowserCredentials bind:heading={credentials} />
</div>
