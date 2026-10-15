<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Dialog from '#lib/components/ui/dialog/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import type { GrantId } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { toolsActions } from '#lib/state/tools-actions.svelte.js';
  import { toolsFailureMessage } from '#lib/utils/tools-messages.js';

  import GrantList from './GrantList.svelte';

  // What the owner allowed that applies to one conversation (docs/features/
  // system-tools/tool-activity.md, permissions of a conversation): its own
  // grants, its project's and those for every conversation, each with
  // "Revoke". `toolsActions.showPermissions` opens it.
  const request = $derived(toolsActions.permissions);
</script>

<Dialog.Root bind:open={toolsActions.permissionsOpen}>
  <Dialog.Content class="sm:max-w-xl" showCloseButton={false}>
    <Dialog.Header>
      <Dialog.Title>{m.tools_permissions_title()}</Dialog.Title>
      <Dialog.Description>{m.tools_permissions_description()}</Dialog.Description>
    </Dialog.Header>
    {#if request !== null}
      {#if request.status === 'loading'}
        <div class="flex flex-col gap-2" aria-busy="true">
          <span class="sr-only" role="status">{m.tools_loading()}</span>
          <Skeleton class="h-12 w-full" />
          <Skeleton class="h-12 w-full" />
        </div>
      {:else if request.readFailure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{m.tools_permissions_failed_title()}</Alert.Title>
          <Alert.Description>{toolsFailureMessage(request.readFailure)}</Alert.Description>
          <Alert.Action>
            <Button size="sm" variant="outline" onclick={() => void request.load()}>{m.tools_try_again()}</Button>
          </Alert.Action>
        </Alert.Root>
      {:else}
        <div class="flex max-h-[60vh] flex-col gap-4 overflow-y-auto">
          <GrantList
            grants={request.grants}
            revoking={request.revoking}
            title={m.tools_permissions_conversation()}
            onRevoke={(id: GrantId) => void request.revoke(id)}
          />
          {#if request.projectId !== null}
            <GrantList
              grants={request.projectGrants}
              revoking={request.revoking}
              title={m.tools_permissions_project()}
              onRevoke={(id: GrantId) => void request.revokeShared(id)}
            />
          {/if}
          <GrantList
            grants={request.alwaysGrants}
            revoking={request.revoking}
            title={m.tools_permissions_always()}
            onRevoke={(id: GrantId) => void request.revokeShared(id)}
          />
        </div>
      {/if}
      {#if request.revokeFailure !== null}
        <p class="text-sm text-destructive" role="alert">{toolsFailureMessage(request.revokeFailure)}</p>
      {/if}
    {/if}
    <Dialog.Footer>
      <Button variant="outline" onclick={() => (toolsActions.permissionsOpen = false)}>{m.tools_close()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
