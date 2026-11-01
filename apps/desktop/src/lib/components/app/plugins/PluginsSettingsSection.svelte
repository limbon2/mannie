<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import PlugIcon from '@lucide/svelte/icons/plug';
  import { onMount } from 'svelte';

  import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Empty from '#lib/components/ui/empty/index.js';
  import * as Item from '#lib/components/ui/item/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { Switch } from '#lib/components/ui/switch/index.js';
  import type { FeatureView, PluginState, PluginView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { ownerName, pluginCatalog } from '#lib/state/plugin-catalog.js';
  import { pluginStates } from '#lib/state/plugin-states.svelte.js';
  import { featureKey, pluginsActions } from '#lib/state/plugins-actions.svelte.js';
  import { featureStateMessage, pluginsFailureMessage, pluginStateMessage } from '#lib/utils/plugins-messages.js';

  import PluginUses from './PluginUses.svelte';

  // The Plugins section of the settings (docs/features/plugins.md): every
  // plugin of this version of Mannie with its name, what it adds, what its
  // manifest lets it use and a switch, which takes effect at once, and under
  // it each of its optional features the same way, whose switch works while
  // the plugin is on (ADR-0055, decision 10). A plugin or feature that is
  // starting or stopping says so, a failed one says what failed and offers
  // "Try again", and turning off one with work in progress asks first.
  const plugins = $derived(
    pluginCatalog().map((descriptor) => ({ descriptor, view: pluginStates.view(descriptor.id) })),
  );

  function switchId(id: string): string {
    return `plugin-switch-${id}`;
  }

  function stateId(id: string): string {
    return `plugin-state-${id}`;
  }

  function featureSwitchId(plugin: string, feature: string): string {
    return `plugin-feature-switch-${plugin}-${feature}`;
  }

  function featureStateId(plugin: string, feature: string): string {
    return `plugin-feature-state-${plugin}-${feature}`;
  }

  /** The line under a feature, once the core reported its plugin and the feature. */
  function featureLineOf(
    view: PluginView | undefined,
    featureView: FeatureView | undefined,
    pluginName: () => string,
  ): string | null {
    if (view === undefined) {
      return null;
    }
    return featureView === undefined ? null : featureStateMessage(view, featureView, pluginName());
  }

  /** Whether a feature's last start failed while its plugin is on, which only then offers "Try again". */
  function failedWhilePluginIsOn(pluginState: PluginState, featureState: PluginState): boolean {
    const pluginIsOn = pluginState === 'on';
    const featureFailed = featureState === 'failed';
    return pluginIsOn && featureFailed;
  }

  onMount(() => {
    pluginStates.start();
  });
</script>

{#if pluginStates.status === 'error'}
  <Alert.Root variant="destructive">
    <CircleAlertIcon />
    <Alert.Title>{m.plugins_list_failed_title()}</Alert.Title>
    <Alert.Description>
      {pluginStates.failure === null ? '' : pluginsFailureMessage(pluginStates.failure)}
    </Alert.Description>
    <Alert.Action>
      <Button size="sm" variant="outline" onclick={() => pluginStates.retry()}>{m.plugins_try_again()}</Button>
    </Alert.Action>
  </Alert.Root>
{:else if pluginStates.status === 'loading'}
  <div class="flex flex-col gap-3" aria-busy="true">
    <span class="sr-only" role="status">{m.plugins_loading()}</span>
    <Skeleton class="h-20 w-full" />
  </div>
{:else if plugins.length === 0}
  <Empty.Root class="border">
    <Empty.Header>
      <Empty.Media variant="icon">
        <PlugIcon />
      </Empty.Media>
      <Empty.Description>{m.plugins_empty()}</Empty.Description>
    </Empty.Header>
  </Empty.Root>
{:else}
  <Item.Group>
    {#each plugins as { descriptor, view } (descriptor.id)}
      {const state = $derived(view?.state ?? 'off')}
      {const moving = $derived(state === 'starting' || state === 'stopping')}
      {const changing = $derived(pluginsActions.changing.has(descriptor.id) || moving)}
      {const stateLine = $derived(view === undefined ? null : pluginStateMessage(view))}
      {const failure = $derived(pluginsActions.failures.get(descriptor.id) ?? null)}
      <div class="flex flex-col gap-2" role="listitem">
        <Item.Root data-plugin={descriptor.id} data-plugin-state={state} variant="outline">
          <Item.Content class="min-w-0">
            <Item.Title>
              <label for={switchId(descriptor.id)}>{descriptor.name()}</label>
            </Item.Title>
            <Item.Description class="line-clamp-none">{descriptor.description()}</Item.Description>
            {#if view !== undefined}
              <PluginUses uses={view.uses} />
            {/if}
            {#if stateLine !== null}
              <p
                id={stateId(descriptor.id)}
                class={[
                  'flex items-center gap-2 text-sm',
                  state === 'failed' ? 'text-destructive' : 'text-muted-foreground',
                ]}
                role="status"
              >
                {#if moving}
                  <Spinner />
                {/if}
                <span>{stateLine}</span>
              </p>
            {/if}
            {#if failure !== null}
              <Alert.Root variant="destructive">
                <CircleAlertIcon />
                <Alert.Title>{m.plugins_change_failed_title()}</Alert.Title>
                <Alert.Description>{pluginsFailureMessage(failure)}</Alert.Description>
              </Alert.Root>
            {/if}
          </Item.Content>
          <Item.Actions>
            {#if state === 'failed'}
              <Button
                data-plugin-retry=""
                disabled={changing}
                size="sm"
                variant="outline"
                onclick={() => void pluginsActions.retry(descriptor.id)}
              >
                {m.plugins_try_again()}
              </Button>
            {/if}
            <Switch
              id={switchId(descriptor.id)}
              aria-describedby={stateLine === null ? undefined : stateId(descriptor.id)}
              data-plugin-switch=""
              disabled={changing}
              bind:checked={
                () => view?.enabled ?? false, (checked: boolean) => void pluginsActions.toggle(descriptor.id, checked)
              }
            />
          </Item.Actions>
        </Item.Root>
        {#if descriptor.features.length > 0}
          <Item.Group
            class="gap-2 ps-6"
            aria-label={m.plugins_features_label({ name: descriptor.name() })}
            data-plugin-features={descriptor.id}
          >
            {#each descriptor.features as feature (feature.id)}
              {const featureView = $derived(view?.features.find((candidate) => candidate.id === feature.id))}
              {const key = $derived(featureKey(descriptor.id, feature.id))}
              {const featureState = $derived(featureView?.state ?? 'off')}
              {const featureMoving = $derived(featureState === 'starting' || featureState === 'stopping')}
              {const featureChanging = $derived(changing || pluginsActions.changing.has(key) || featureMoving)}
              {const featureLine = $derived(featureLineOf(view, featureView, descriptor.name))}
              {const featureFailed = $derived(failedWhilePluginIsOn(state, featureState))}
              {const featureFailure = $derived(pluginsActions.failures.get(key) ?? null)}
              <Item.Root
                data-plugin-feature={feature.id}
                data-plugin-feature-state={featureState}
                role="listitem"
                variant="muted"
              >
                <Item.Content class="min-w-0">
                  <Item.Title>
                    <label for={featureSwitchId(descriptor.id, feature.id)}>{feature.name()}</label>
                  </Item.Title>
                  <Item.Description class="line-clamp-none">{feature.description()}</Item.Description>
                  {#if featureView !== undefined}
                    <PluginUses uses={featureView.uses} />
                  {/if}
                  {#if featureLine !== null}
                    <p
                      id={featureStateId(descriptor.id, feature.id)}
                      class={[
                        'flex items-center gap-2 text-sm',
                        featureFailed ? 'text-destructive' : 'text-muted-foreground',
                      ]}
                      role="status"
                    >
                      {#if featureMoving}
                        <Spinner />
                      {/if}
                      <span>{featureLine}</span>
                    </p>
                  {/if}
                  {#if featureFailure !== null}
                    <Alert.Root variant="destructive">
                      <CircleAlertIcon />
                      <Alert.Title>{m.plugins_change_failed_title()}</Alert.Title>
                      <Alert.Description>{pluginsFailureMessage(featureFailure)}</Alert.Description>
                    </Alert.Root>
                  {/if}
                </Item.Content>
                <Item.Actions>
                  {#if featureFailed}
                    <Button
                      data-plugin-feature-retry=""
                      disabled={featureChanging}
                      size="sm"
                      variant="outline"
                      onclick={() => void pluginsActions.retryFeature(descriptor.id, feature.id)}
                    >
                      {m.plugins_try_again()}
                    </Button>
                  {/if}
                  <Switch
                    id={featureSwitchId(descriptor.id, feature.id)}
                    aria-describedby={featureLine === null ? undefined : featureStateId(descriptor.id, feature.id)}
                    data-plugin-feature-switch=""
                    disabled={featureChanging || state !== 'on'}
                    bind:checked={
                      () => featureView?.enabled ?? false,
                      (checked: boolean) => void pluginsActions.toggleFeature(descriptor.id, feature.id, checked)
                    }
                  />
                </Item.Actions>
              </Item.Root>
            {/each}
          </Item.Group>
        {/if}
      </div>
    {/each}
  </Item.Group>
{/if}

<AlertDialog.Root bind:open={pluginsActions.offOpen}>
  <AlertDialog.Content class="data-[size=default]:sm:max-w-md">
    {#if pluginsActions.offQuestion !== null}
      <AlertDialog.Header>
        <AlertDialog.Title
          >{m.plugins_off_title({
            name: ownerName({ feature: pluginsActions.offQuestion.feature, plugin: pluginsActions.offQuestion.id }),
          })}</AlertDialog.Title
        >
        <AlertDialog.Description>{pluginsActions.offQuestion.question}</AlertDialog.Description>
      </AlertDialog.Header>
      <AlertDialog.Footer>
        <AlertDialog.Cancel>{m.plugins_cancel()}</AlertDialog.Cancel>
        <Button data-plugin-off-confirm="" variant="destructive" onclick={() => void pluginsActions.confirmOff()}>
          {m.plugins_off_confirm()}
        </Button>
      </AlertDialog.Footer>
    {/if}
  </AlertDialog.Content>
</AlertDialog.Root>
