<script lang="ts">
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Item from '#lib/components/ui/item/index.js';
  import type { GrantId, GrantScope } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { grantScopeMessage } from '#lib/utils/tools-messages.js';

  // One group of grants in the permissions of a conversation
  // (docs/features/system-tools/tool-activity.md, permissions of a
  // conversation): its heading, and each grant with "Revoke".
  interface Grant {
    readonly id: GrantId;
    readonly scope: GrantScope;
  }

  interface Props {
    grants: readonly Grant[];
    onRevoke: (id: GrantId) => void;
    /** The grants being revoked. */
    revoking: readonly GrantId[];
    title: string;
  }

  let { grants, onRevoke, revoking, title }: Props = $props();
</script>

<section class="flex flex-col gap-2">
  <h3 class="text-sm font-medium">{title}</h3>
  {#if grants.length === 0}
    <p class="text-sm text-muted-foreground">{m.tools_permissions_none()}</p>
  {:else}
    <Item.Group>
      {#each grants as grant (grant.id)}
        <Item.Root role="listitem" size="sm" variant="outline">
          <Item.Content class="min-w-0">
            <Item.Title class="line-clamp-none w-full font-normal break-all"
              >{grantScopeMessage(grant.scope)}</Item.Title
            >
          </Item.Content>
          <Item.Actions>
            <Button
              aria-label={m.tools_permissions_revoke_label({ grant: grantScopeMessage(grant.scope) })}
              disabled={revoking.includes(grant.id)}
              size="sm"
              variant="outline"
              onclick={() => onRevoke(grant.id)}
            >
              {m.tools_permissions_revoke()}
            </Button>
          </Item.Actions>
        </Item.Root>
      {/each}
    </Item.Group>
  {/if}
</section>
