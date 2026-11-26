<script lang="ts">
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
  import { untrack } from 'svelte';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import { Input } from '#lib/components/ui/input/index.js';
  import { Label } from '#lib/components/ui/label/index.js';
  import type {
    Access,
    ApprovalChoice,
    ApprovalRequest,
    ConfigSecretField,
    ConversationId,
    PartId,
  } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { toolsActions } from '#lib/state/tools-actions.svelte.js';
  import { approvalReasonMessage, grantScopeMessage } from '#lib/utils/tools-messages.js';

  import ToolAccessList from './ToolAccessList.svelte';

  // A tool call that waits for the owner (docs/features/system-tools/
  // tool-activity.md, approval requests): what will happen, exactly as the
  // core prepared it, why Mannie asks, what "Allow for…" would cover, and the
  // owner's options. "Allow for…" lasts for this conversation, its project or
  // every conversation; the project is offered only in a project's
  // conversation. Configuration approvals (ADR-0081) always offer only Allow
  // once and Deny, and may include masked inputs for secrets that go straight
  // to the core without entering the transcript or model context. The buttons
  // belong to the card's own layout, outside every piece of content (T-3).
  // Nothing here takes the focus by itself.
  interface Props {
    accesses: readonly Access[];
    conversationId: ConversationId;
    partId: PartId;
    request: ApprovalRequest;
  }

  let { accesses, conversationId, partId, request }: Props = $props();

  const cardId = $props.id();

  // Masked secret inputs stay strictly in component-local memory and clear
  // immediately on submit, deny, or when the request digest changes.
  let secretValues = $state<Partial<Record<string, string>>>({});
  let secretErrors = $state<Partial<Record<string, string>>>({});

  const appConfigSecurity = $derived(request.reasons.includes('app_config_security'));
  const appConfig = $derived(request.reasons.includes('app_config') || appConfigSecurity);
  const browserSite = $derived(request.reasons.includes('browser_site'));
  const browserControl = $derived(request.reasons.includes('browser_control'));
  const browserCredential = $derived(request.reasons.includes('browser_credential'));
  const deciding = $derived(toolsActions.deciding.includes(partId));
  const offersGrants = $derived(
    !appConfig && !browserSite && !browserControl && !browserCredential && request.grants.length > 0,
  );
  const offersRule = $derived(
    !appConfig && !browserSite && !browserControl && !browserCredential && request.rule !== null,
  );
  const inProject = $derived(request.project !== null && request.project !== undefined);
  /** The computer that runs the task of the operation, by the name its card shows, or `null` for this computer. */
  const host = $derived(request.host ?? null);
  const secretFields: readonly ConfigSecretField[] = $derived.by(() => {
    for (const access of accesses) {
      if (access.kind === 'app_config') {
        return access.secretFields ?? [];
      }
    }
    return [];
  });
  const showsSecretInputs = $derived(appConfig && secretFields.length > 0);
  const requestKey = $derived(`${partId}:${request.digest}`);

  function secretInputId(index: number): string {
    return `${cardId}-secret-${index}`;
  }

  function secretErrorId(index: number): string {
    return `${cardId}-secret-error-${index}`;
  }

  function secretHintId(index: number): string {
    return `${cardId}-secret-hint-${index}`;
  }

  function secretDescribedBy(field: ConfigSecretField, index: number): string | undefined {
    const ids: string[] = [];
    if (field.providedInChat ?? false) {
      ids.push(secretHintId(index));
    }
    if (secretErrors[field.key] !== undefined) {
      ids.push(secretErrorId(index));
    }
    return ids.length > 0 ? ids.join(' ') : undefined;
  }

  function clearSecretError(key: string): void {
    const hasError = secretErrors[key] !== undefined;
    const value = secretValues[key] ?? '';
    const hasValue = value !== '';
    if (hasError && hasValue) {
      const { [key]: _removed, ...rest } = secretErrors;

      secretErrors = rest;
    }
  }

  function handleSecretKeydown(event: KeyboardEvent): void {
    const isSubmitKey = event.key === 'Enter';
    if (isSubmitKey && !deciding) {
      event.preventDefault();
      allowConfig();
    }
  }

  function decide(choice: ApprovalChoice): void {
    void toolsActions.decide(conversationId, partId, request.digest, choice);
  }

  function allowConfig(): void {
    const missing: Partial<Record<string, string>> = {};
    let firstMissingIndex = -1;
    for (const [index, field] of secretFields.entries()) {
      const value = secretValues[field.key] ?? '';
      const providedInChat = field.providedInChat ?? false;
      const isEmpty = value === '';
      if (field.required && !providedInChat && isEmpty) {
        missing[field.key] = m.tools_approval_config_secret_missing({ label: field.label });
        if (firstMissingIndex === -1) {
          firstMissingIndex = index;
        }
      }
    }

    if (firstMissingIndex !== -1) {
      secretErrors = missing;
      const element = document.getElementById(secretInputId(firstMissingIndex));
      if (element instanceof HTMLElement) {
        element.focus();
      }
      return;
    }

    const submitted: Record<string, string> = {};
    for (const field of secretFields) {
      const value = secretValues[field.key] ?? '';
      if (value !== '') {
        submitted[field.key] = value;
      }
    }

    // Clear component-local secrets immediately before awaiting IPC.
    secretValues = {};
    secretErrors = {};
    void toolsActions.decideConfig(conversationId, partId, request.digest, true, submitted);
  }

  function denyConfig(): void {
    secretValues = {};
    secretErrors = {};
    void toolsActions.decideConfig(conversationId, partId, request.digest, false, {});
  }

  $effect(() => {
    if (requestKey !== '') {
      untrack(() => {
        secretValues = {};
        secretErrors = {};
      });
    }
  });
</script>

<div class="flex flex-col gap-3 rounded-md border bg-muted/40 p-3" data-tool-approval>
  <p class="text-sm font-medium">{m.tools_approval_title()}</p>

  <div class="flex flex-col gap-1.5">
    <p class="text-xs font-medium text-muted-foreground">{m.tools_approval_will()}</p>
    <ToolAccessList {accesses} />
  </div>

  {#if host !== null}
    <!-- The computer's name is content, which it or the owner wrote. -->
    <div class="flex flex-col gap-1">
      <p class="text-xs font-medium text-muted-foreground">{m.tools_approval_host()}</p>
      <p class="text-sm wrap-anywhere" data-approval-host="" data-content dir="auto">{host}</p>
    </div>
  {/if}

  {#if request.reasons.length > 0}
    <div class="flex flex-col gap-1">
      <p class="text-xs font-medium text-muted-foreground">{m.tools_approval_why()}</p>
      <ul class="list-disc ps-5 text-sm">
        {#each request.reasons as reason (reason)}
          <li>{approvalReasonMessage(reason)}</li>
        {/each}
      </ul>
    </div>
  {/if}

  {#if appConfigSecurity}
    <p class="text-xs font-medium text-destructive" data-app-config-security-note="">
      {m.tools_approval_config_security_note()}
    </p>
  {/if}

  {#if showsSecretInputs}
    <div class="flex flex-col gap-2.5 rounded-md border bg-background/70 p-2.5" data-config-secrets="">
      <div class="flex flex-col gap-0.5">
        <p class="text-xs font-medium">{m.tools_approval_config_secrets_title()}</p>
        <p class="text-xs text-muted-foreground">{m.tools_approval_config_secrets_note()}</p>
      </div>

      {#each secretFields as field, index (field.key)}
        {const inputId = secretInputId(index)}
        {const errorMessage = secretErrors[field.key]}
        {const providedInChat = field.providedInChat ?? false}
        {const requiresInput = field.required && !providedInChat}
        <div class="flex flex-col gap-1" data-config-secret-field={field.key}>
          <div class="flex items-baseline justify-between gap-2">
            <Label class="text-xs font-medium" for={inputId}>
              <span data-content dir="auto">{field.label}</span>
            </Label>
            <span class="text-xs text-muted-foreground">
              {requiresInput
                ? m.tools_approval_config_secret_required_tag()
                : m.tools_approval_config_secret_optional()}
            </span>
          </div>

          <Input
            id={inputId}
            aria-describedby={secretDescribedBy(field, index)}
            aria-invalid={errorMessage !== undefined ? 'true' : undefined}
            autocomplete="off"
            disabled={deciding}
            required={requiresInput}
            spellcheck={false}
            type="password"
            bind:value={secretValues[field.key]}
            oninput={() => clearSecretError(field.key)}
            onkeydown={handleSecretKeydown}
          />

          {#if providedInChat}
            <p id={secretHintId(index)} class="text-xs text-muted-foreground" data-secret-in-chat="">
              {m.tools_approval_config_secret_in_chat()}
            </p>
          {/if}

          {#if errorMessage !== undefined}
            <p id={secretErrorId(index)} class="text-xs text-destructive" role="alert">
              {errorMessage}
            </p>
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  {#if offersGrants}
    <div class="flex flex-col gap-1">
      <p class="text-xs font-medium text-muted-foreground">{m.tools_approval_covers()}</p>
      <ul class="list-disc ps-5 text-sm break-all">
        {#each request.grants as grant, index (index)}
          <li>{grantScopeMessage(grant)}</li>
        {/each}
      </ul>
    </div>
  {/if}

  {#if browserSite}
    <p class="text-sm">{m.tools_approval_browser_scope()}</p>
  {/if}

  {#if browserCredential}
    <p class="text-sm">{m.tools_approval_browser_credential_scope()}</p>
  {/if}

  <div class="flex flex-wrap gap-2">
    {#if appConfig}
      <Button disabled={deciding} size="sm" onclick={allowConfig}>
        {m.tools_allow_once()}
      </Button>
      <Button disabled={deciding} size="sm" variant="destructive" onclick={denyConfig}>
        {m.tools_deny()}
      </Button>
    {:else}
      {#if browserCredential || browserSite}
        <Button disabled={deciding} size="sm" onclick={() => decide({ choice: 'allow_for_conversation' })}>
          {m.tools_allow_browser_site_for_conversation()}
        </Button>

        {#if !browserCredential}
          <Button disabled={deciding} size="sm" variant="outline" onclick={() => decide({ choice: 'allow_once' })}>
            {m.tools_allow_browser_site()}
          </Button>
        {/if}
      {:else}
        <Button disabled={deciding} size="sm" onclick={() => decide({ choice: 'allow_once' })}>
          {browserControl ? m.tools_allow_browser_control() : m.tools_allow_once()}
        </Button>
      {/if}

      {#if offersGrants}
        <DropdownMenu.Root>
          <DropdownMenu.Trigger>
            {#snippet child({ props })}
              <Button {...props} data-allow-for disabled={deciding} size="sm" variant="outline">
                {m.tools_allow_for()}
                <ChevronDownIcon data-icon="inline-end" />
              </Button>
            {/snippet}
          </DropdownMenu.Trigger>
          <DropdownMenu.Content align="start">
            <DropdownMenu.Group>
              <DropdownMenu.Item onSelect={() => decide({ choice: 'allow_for_conversation' })}>
                {m.tools_allow_for_conversation()}
              </DropdownMenu.Item>

              {#if inProject}
                <DropdownMenu.Item onSelect={() => decide({ choice: 'allow_for_project' })}>
                  {m.tools_allow_for_project()}
                </DropdownMenu.Item>
              {/if}

              <DropdownMenu.Item onSelect={() => decide({ choice: 'allow_always' })}>
                {m.tools_allow_always()}
              </DropdownMenu.Item>
            </DropdownMenu.Group>
          </DropdownMenu.Content>
        </DropdownMenu.Root>
      {/if}

      {#if offersRule}
        <Button
          disabled={deciding}
          size="sm"
          variant="outline"
          onclick={() => toolsActions.startSaveRule(conversationId, partId, request)}
        >
          {m.tools_save_rule_open()}
        </Button>
      {/if}

      <Button disabled={deciding} size="sm" variant="destructive" onclick={() => decide({ choice: 'deny' })}>
        {m.tools_deny()}
      </Button>
    {/if}
  </div>
</div>
