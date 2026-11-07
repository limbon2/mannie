<script lang="ts">
  import BotIcon from '@lucide/svelte/icons/bot';
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import { onMount } from 'svelte';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Empty from '#lib/components/ui/empty/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { agentActions } from '#lib/state/agent-actions.svelte.js';
  import { agents } from '#lib/state/agents.svelte.js';
  import { providerProfiles } from '#lib/state/provider-profiles.svelte.js';
  import { agentFailureMessage } from '#lib/utils/agents-messages.js';
  import { shownAgentKey } from '#lib/utils/agents-urls.js';

  import AgentEditor from './AgentEditor.svelte';

  // The main area of the Agents page (docs/features/custom-agents.md): the
  // editor of the agent that the address names, or of a new one, which a form
  // holds until it is saved. Each agent opens with a form of its own.
  const key = $derived(shownAgentKey());
  const shown = $derived(agents.find(key));
  const newForm = $derived(key === null ? agentActions.newForm : null);

  onMount(() => {
    // The plugins that are on may have changed since the page last showed.
    void agents.reload();
    void providerProfiles.ensureLoaded();
  });
</script>

<div class="flex min-h-0 flex-1 flex-col overflow-y-auto" data-agents-view="">
  {#if agents.status === 'loading'}
    <div class="mx-auto flex w-full max-w-3xl flex-col gap-4 p-6" aria-busy="true">
      <Skeleton class="h-10 w-1/2" />
      <Skeleton class="h-24 w-full" />
      <Skeleton class="h-48 w-full" />
    </div>
  {:else if agents.status === 'error'}
    <div class="mx-auto w-full max-w-3xl p-6">
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.agents_failed_title()}</Alert.Title>
        <Alert.Description>{agents.failure === null ? '' : agentFailureMessage(agents.failure)}</Alert.Description>
        <Alert.Action>
          <Button size="sm" variant="outline" onclick={() => void agents.reload()}>{m.chat_try_again()}</Button>
        </Alert.Action>
      </Alert.Root>
    </div>
  {:else if newForm !== null}
    {#key newForm}
      <AgentEditor source={newForm} />
    {/key}
  {:else if shown !== undefined}
    {#key shown.key}
      <AgentEditor source={shown} />
    {/key}
  {:else if key !== null}
    <Empty.Root class="flex-1">
      <Empty.Header>
        <Empty.Media variant="icon">
          <BotIcon />
        </Empty.Media>
        <Empty.Title>{m.agents_gone_title()}</Empty.Title>
        <Empty.Description>{m.agents_gone_description()}</Empty.Description>
      </Empty.Header>
    </Empty.Root>
  {:else}
    <Empty.Root class="flex-1">
      <Empty.Header>
        <Empty.Media variant="icon">
          <BotIcon />
        </Empty.Media>
        <Empty.Title>{m.agents_empty_title()}</Empty.Title>
        <Empty.Description>{m.agents_empty_description()}</Empty.Description>
      </Empty.Header>
    </Empty.Root>
  {/if}
</div>
