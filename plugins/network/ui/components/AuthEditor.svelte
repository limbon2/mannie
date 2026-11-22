<script lang="ts">
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import * as Select from '@mannie/plugin-sdk/components/select';

  import type { ApiKeyLocation, AuthConfig } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import type { AuthKind } from '../types/network.js';
  import { authKindLabel } from '../utils/network-messages.js';
  import type { VariableSuggestion } from '../utils/network-urls.js';

  import VariableInput from './VariableInput.svelte';

  interface Props {
    allowInherit?: boolean;
    auth: AuthConfig;
    disabled?: boolean;
    onChange: (next: AuthConfig) => void;
    variables?: readonly VariableSuggestion[];
  }

  let { allowInherit = false, auth, disabled = false, onChange, variables = [] }: Props = $props();

  const kinds: AuthKind[] = $derived(
    allowInherit ? ['inherit', 'none', 'bearer', 'basic', 'api_key'] : ['none', 'bearer', 'basic', 'api_key'],
  );

  function changeKind(raw: string): void {
    switch (raw) {
      case 'inherit': {
        onChange({ kind: 'inherit' });
        return;
      }
      case 'none': {
        onChange({ kind: 'none' });
        return;
      }
      case 'bearer': {
        onChange({ kind: 'bearer', token: auth.kind === 'bearer' ? auth.token : '' });
        return;
      }
      case 'basic': {
        onChange({
          kind: 'basic',
          password: auth.kind === 'basic' ? auth.password : '',
          username: auth.kind === 'basic' ? auth.username : '',
        });
        return;
      }
      case 'api_key': {
        onChange({
          kind: 'api_key',
          location: auth.kind === 'api_key' ? auth.location : 'header',
          name: auth.kind === 'api_key' ? auth.name : 'X-API-Key',
          value: auth.kind === 'api_key' ? auth.value : '',
        });
      }
    }
  }

  function updateBearerToken(token: string): void {
    onChange({ kind: 'bearer', token });
  }

  function updateBasicUsername(username: string): void {
    if (auth.kind !== 'basic') {
      return;
    }
    onChange({ kind: 'basic', password: auth.password, username });
  }

  function updateBasicPassword(password: string): void {
    if (auth.kind !== 'basic') {
      return;
    }
    onChange({ kind: 'basic', password, username: auth.username });
  }

  function updateApiKeyName(event: Event): void {
    if (auth.kind !== 'api_key') {
      return;
    }
    const name = (event.currentTarget as HTMLInputElement).value;
    onChange({
      kind: 'api_key',
      location: auth.location,
      name,
      value: auth.value,
    });
  }

  function updateApiKeyValue(value: string): void {
    if (auth.kind !== 'api_key') {
      return;
    }
    onChange({
      kind: 'api_key',
      location: auth.location,
      name: auth.name,
      value,
    });
  }

  function updateApiKeyLocation(raw: string): void {
    if (auth.kind !== 'api_key') {
      return;
    }
    const location: ApiKeyLocation = raw === 'query' ? 'query' : 'header';
    onChange({
      kind: 'api_key',
      location,
      name: auth.name,
      value: auth.value,
    });
  }
</script>

<div class="flex flex-col gap-4">
  <Field.Field>
    <Field.Label>{m.network_auth_type_label()}</Field.Label>
    <Select.Root {disabled} type="single" value={auth.kind} onValueChange={changeKind}>
      <Select.Trigger class="w-full max-w-xs">
        {authKindLabel(auth.kind)}
      </Select.Trigger>
      <Select.Content>
        {#each kinds as kind (kind)}
          <Select.Item label={authKindLabel(kind)} value={kind}>
            {authKindLabel(kind)}
          </Select.Item>
        {/each}
      </Select.Content>
    </Select.Root>
  </Field.Field>

  {#if auth.kind === 'bearer'}
    <Field.Field>
      <Field.Label for="network-auth-bearer-token">{m.network_auth_bearer_token()}</Field.Label>
      <VariableInput
        id="network-auth-bearer-token"
        class="font-mono text-xs"
        {disabled}
        placeholder={m.network_auth_bearer_placeholder()}
        value={auth.token}
        {variables}
        onValueChange={updateBearerToken}
      />
    </Field.Field>
  {:else if auth.kind === 'basic'}
    <div class="grid gap-3 sm:grid-cols-2">
      <Field.Field>
        <Field.Label>{m.network_auth_basic_username()}</Field.Label>
        <VariableInput
          class="font-mono text-xs"
          {disabled}
          placeholder={m.network_auth_basic_username_placeholder()}
          value={auth.username}
          {variables}
          onValueChange={updateBasicUsername}
        />
      </Field.Field>

      <Field.Field>
        <Field.Label>{m.network_auth_basic_password()}</Field.Label>
        <VariableInput
          class="font-mono text-xs"
          {disabled}
          placeholder={m.network_auth_basic_password_placeholder()}
          value={auth.password}
          {variables}
          onValueChange={updateBasicPassword}
        />
      </Field.Field>
    </div>
  {:else if auth.kind === 'api_key'}
    <div class="grid gap-3 sm:grid-cols-3">
      <Field.Field>
        <Field.Label>{m.network_auth_api_key_name()}</Field.Label>
        <Input
          class="font-mono text-xs"
          {disabled}
          placeholder={m.network_auth_api_key_name_placeholder()}
          value={auth.name}
          oninput={updateApiKeyName}
        />
      </Field.Field>

      <Field.Field>
        <Field.Label>{m.network_auth_api_key_value()}</Field.Label>
        <VariableInput
          class="font-mono text-xs"
          {disabled}
          placeholder={m.network_auth_api_key_value_placeholder()}
          value={auth.value}
          {variables}
          onValueChange={updateApiKeyValue}
        />
      </Field.Field>

      <Field.Field>
        <Field.Label>{m.network_auth_api_key_in()}</Field.Label>
        <Select.Root {disabled} type="single" value={auth.location} onValueChange={updateApiKeyLocation}>
          <Select.Trigger class="w-full">
            {auth.location === 'query' ? m.network_auth_api_key_query() : m.network_auth_api_key_header()}
          </Select.Trigger>
          <Select.Content>
            <Select.Item label={m.network_auth_api_key_header()} value="header">
              {m.network_auth_api_key_header()}
            </Select.Item>
            <Select.Item label={m.network_auth_api_key_query()} value="query">
              {m.network_auth_api_key_query()}
            </Select.Item>
          </Select.Content>
        </Select.Root>
      </Field.Field>
    </div>
  {/if}
</div>
