<script lang="ts">
  import { onDestroy } from 'svelte';

  import { networkDialogs } from '../state/network-dialogs.svelte.js';

  import CollectionDialog from './CollectionDialog.svelte';
  import CookieJarDialog from './CookieJarDialog.svelte';
  import EnvironmentsDialog from './EnvironmentsDialog.svelte';
  import FolderAndDeleteDialogs from './FolderAndDeleteDialogs.svelte';
  import GlobalVariablesDialog from './GlobalVariablesDialog.svelte';
  import ImportOpenApiDialog from './ImportOpenApiDialog.svelte';

  onDestroy(() => {
    networkDialogs.closeAll();
  });
</script>

<CollectionDialog />
<ImportOpenApiDialog />
<EnvironmentsDialog />
<GlobalVariablesDialog />
<CookieJarDialog />
<FolderAndDeleteDialogs />
