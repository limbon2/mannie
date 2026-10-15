<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import { onMount } from 'svelte';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import * as Item from '#lib/components/ui/item/index.js';
  import { Separator } from '#lib/components/ui/separator/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { Switch } from '#lib/components/ui/switch/index.js';
  import { Textarea } from '#lib/components/ui/textarea/index.js';
  import type { CommandRule, SharedGrant } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { conversationList } from '#lib/state/conversation-list.svelte.js';
  import { commandRules, sharedGrants, toolsSettings } from '#lib/state/tools-settings.svelte.js';
  import { grantScopeMessage, toolsFailureMessage } from '#lib/utils/tools-messages.js';

  // The Tools section of the settings (docs/features/system-tools/
  // tool-activity.md, settings): whether changes inside a workspace run
  // without asking, the command rules, the grants for a project or for every
  // conversation, and the environment variables that commands receive beyond
  // the built-in ones. Every change goes to the core, which checks it and
  // returns what it stored. A conversation's workspace is its project's
  // folder, which the sidebar adds (docs/features/chat/projects.md).
  let environmentDraft = $state('');
  let environmentSaved = $state(false);

  const settings = $derived(toolsSettings.settings);
  const saveFailure = $derived(toolsSettings.saveFailure);
  const storedEnvironment = $derived(settings?.commandEnvironment.join('\n') ?? '');
  const environmentChanged = $derived(environmentNames(environmentDraft).join('\n') !== storedEnvironment);

  /** The names in the text field: one per line, without spaces around them or empty lines. */
  function environmentNames(text: string): string[] {
    return text
      .split(/\r?\n/u)
      .map((line) => line.trim())
      .filter((line) => line !== '');
  }

  function ruleWords(rule: CommandRule): string {
    return rule.words.join(' ');
  }

  /** Where a grant applies: in a project, by its name, or in every conversation. */
  function grantPlace(grant: SharedGrant): string {
    if (grant.projectId === null) {
      return m.tools_settings_grant_everywhere();
    }
    const project = conversationList.findProject(grant.projectId);
    return project === undefined
      ? m.tools_settings_grant_in_some_project()
      : m.tools_settings_grant_in_project({ project: project.name });
  }

  async function load(): Promise<void> {
    await toolsSettings.load();
    environmentDraft = toolsSettings.settings?.commandEnvironment.join('\n') ?? '';
  }

  async function saveEnvironment(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const failure = await toolsSettings.update({ commandEnvironment: environmentNames(environmentDraft) });
    environmentSaved = failure === null;
    if (failure === null) {
      environmentDraft = toolsSettings.settings?.commandEnvironment.join('\n') ?? '';
    }
  }

  function setAllowChanges(allow: boolean): void {
    void toolsSettings.update({ allowChangesInWorkspace: allow });
  }

  onMount(() => {
    void load();
    void commandRules.load();
    void sharedGrants.load();
    // The names of the grants' projects come from the conversation list.
    conversationList.start();
  });
</script>

{#if toolsSettings.readFailure !== null}
  <Alert.Root variant="destructive">
    <CircleAlertIcon />
    <Alert.Title>{m.shell_settings_load_failed_title()}</Alert.Title>
    <Alert.Description>{toolsFailureMessage(toolsSettings.readFailure)}</Alert.Description>
    <Alert.Action>
      <Button size="sm" variant="outline" onclick={() => void load()}>{m.tools_try_again()}</Button>
    </Alert.Action>
  </Alert.Root>
{:else if settings === null}
  <div class="flex flex-col gap-3" aria-busy="true">
    <span class="sr-only" role="status">{m.tools_loading()}</span>
    <Skeleton class="h-5 w-56" />
    <Skeleton class="h-9 w-full" />
    <Skeleton class="h-5 w-72" />
    <Skeleton class="h-16 w-full" />
  </div>
{:else}
  <div class="flex flex-col gap-6">
    <Field.Field orientation="horizontal">
      <Field.Content>
        <Field.Label for="tools-allow-changes">{m.tools_settings_allow_changes()}</Field.Label>
        <Field.Description>{m.tools_settings_allow_changes_description()}</Field.Description>
      </Field.Content>
      <Switch
        id="tools-allow-changes"
        disabled={toolsSettings.saving}
        bind:checked={() => settings.allowChangesInWorkspace, setAllowChanges}
      />
    </Field.Field>

    <Separator />

    <Field.Set>
      <Field.Legend variant="label">{m.tools_settings_rules_title()}</Field.Legend>
      <Field.Description>{m.tools_settings_rules_description()}</Field.Description>
      {#if commandRules.status === 'loading'}
        <div class="flex flex-col gap-2" aria-busy="true">
          <Skeleton class="h-12 w-full" />
          <Skeleton class="h-12 w-full" />
        </div>
      {:else if commandRules.readFailure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{m.tools_settings_rules_failed_title()}</Alert.Title>
          <Alert.Description>{toolsFailureMessage(commandRules.readFailure)}</Alert.Description>
          <Alert.Action>
            <Button size="sm" variant="outline" onclick={() => void commandRules.load()}>{m.tools_try_again()}</Button>
          </Alert.Action>
        </Alert.Root>
      {:else if commandRules.rules.length === 0}
        <p class="text-sm text-muted-foreground">{m.tools_settings_rules_empty()}</p>
      {:else}
        <Item.Group>
          {#each commandRules.rules as rule (rule.id)}
            <Item.Root role="listitem" size="sm" variant="outline">
              <Item.Content class="min-w-0">
                <Item.Title class="line-clamp-none w-full">
                  <span class="font-mono break-all" data-content>{ruleWords(rule)}</span>
                </Item.Title>
                <Item.Description class="line-clamp-none break-all">
                  {rule.folder === null
                    ? m.tools_settings_rule_anywhere()
                    : m.tools_settings_rule_in_folder({ folder: rule.folder })}
                </Item.Description>
              </Item.Content>
              <Item.Actions>
                <Button
                  aria-label={m.tools_settings_rule_delete_label({ words: ruleWords(rule) })}
                  disabled={commandRules.deleting.includes(rule.id)}
                  size="sm"
                  variant="outline"
                  onclick={() => void commandRules.remove(rule.id)}
                >
                  {m.tools_settings_rule_delete()}
                </Button>
              </Item.Actions>
            </Item.Root>
          {/each}
        </Item.Group>
      {/if}
      {#if commandRules.deleteFailure !== null}
        <p class="text-sm text-destructive" role="alert">{toolsFailureMessage(commandRules.deleteFailure)}</p>
      {/if}
    </Field.Set>

    <Separator />

    <Field.Set>
      <Field.Legend variant="label">{m.tools_settings_grants_title()}</Field.Legend>
      <Field.Description>{m.tools_settings_grants_description()}</Field.Description>
      {#if sharedGrants.status === 'loading'}
        <div class="flex flex-col gap-2" aria-busy="true">
          <Skeleton class="h-12 w-full" />
        </div>
      {:else if sharedGrants.readFailure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{m.tools_settings_grants_failed_title()}</Alert.Title>
          <Alert.Description>{toolsFailureMessage(sharedGrants.readFailure)}</Alert.Description>
          <Alert.Action>
            <Button size="sm" variant="outline" onclick={() => void sharedGrants.load()}>{m.tools_try_again()}</Button>
          </Alert.Action>
        </Alert.Root>
      {:else if sharedGrants.grants.length === 0}
        <p class="text-sm text-muted-foreground">{m.tools_settings_grants_empty()}</p>
      {:else}
        <Item.Group>
          {#each sharedGrants.grants as grant (grant.id)}
            <Item.Root role="listitem" size="sm" variant="outline">
              <Item.Content class="min-w-0">
                <Item.Title class="line-clamp-none w-full font-normal break-all">
                  {grantScopeMessage(grant.scope)}
                </Item.Title>
                <Item.Description class="line-clamp-none break-all">{grantPlace(grant)}</Item.Description>
              </Item.Content>
              <Item.Actions>
                <Button
                  aria-label={m.tools_permissions_revoke_label({ grant: grantScopeMessage(grant.scope) })}
                  disabled={sharedGrants.revoking.includes(grant.id)}
                  size="sm"
                  variant="outline"
                  onclick={() => void sharedGrants.revoke(grant.id)}
                >
                  {m.tools_permissions_revoke()}
                </Button>
              </Item.Actions>
            </Item.Root>
          {/each}
        </Item.Group>
      {/if}
      {#if sharedGrants.revokeFailure !== null}
        <p class="text-sm text-destructive" role="alert">{toolsFailureMessage(sharedGrants.revokeFailure)}</p>
      {/if}
    </Field.Set>

    <Separator />

    <form class="flex flex-col gap-3" onsubmit={saveEnvironment}>
      <Field.Field>
        <Field.Label for="tools-environment">{m.tools_settings_environment_label()}</Field.Label>
        <Textarea
          id="tools-environment"
          class="min-h-24 font-mono"
          aria-describedby="tools-environment-description"
          spellcheck="false"
          bind:value={environmentDraft}
          oninput={() => (environmentSaved = false)}
        />
        <Field.Description id="tools-environment-description">
          {m.tools_settings_environment_description()}
        </Field.Description>
      </Field.Field>
      <div class="flex items-center gap-3">
        <Button disabled={toolsSettings.saving || !environmentChanged} type="submit">{m.tools_settings_save()}</Button>
        <span class="text-sm text-muted-foreground" role="status">
          {environmentSaved ? m.tools_settings_saved() : ''}
        </span>
      </div>
    </form>

    {#if saveFailure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.shell_settings_save_failed_title()}</Alert.Title>
        <Alert.Description>{toolsFailureMessage(saveFailure)}</Alert.Description>
      </Alert.Root>
    {/if}
  </div>
{/if}
