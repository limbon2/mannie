<script lang="ts">
  import { FieldHelp } from '@mannie/plugin-sdk/components';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import XIcon from '@mannie/plugin-sdk/icons/x';

  import { m } from '../generated/messages.js';
  import { MAX_NAME_OF_VALUE_CHARS } from '../generated/plugin.js';

  // The variables or headers of a server, plain or secret, as rows of a
  // name and a value (plugins/mcp/docs/README.md, registering a server). A
  // secret's value never comes back from the core: a stored one shows as
  // stored, and an empty field keeps it.
  interface Row {
    name: string;
    readonly stored?: boolean;
    value: string;
  }

  let {
    disabled,
    help,
    id,
    invalid,
    legend,
    namePlaceholder,
    onAdd,
    onRemove,
    rows,
    secret,
    valuePlaceholder,
  }: {
    disabled: boolean;
    help: string;
    id: string;
    invalid: boolean;
    legend: string;
    namePlaceholder: string;
    onAdd: () => void;
    onRemove: (index: number) => void;
    rows: Row[];
    secret: boolean;
    valuePlaceholder: string;
  } = $props();

  /** Whether a row of secrets names a secret that the core keeps no value of, and the owner typed none. */
  function isUnstored(row: Row): boolean {
    const isNotStored = row.stored === false;
    const isEmpty = row.value === '';
    return secret && isNotStored && isEmpty;
  }
</script>

<Field.Set data-invalid={invalid || undefined}>
  <div class="flex items-center gap-1">
    <Field.Legend class="mb-0" variant="label">{legend}</Field.Legend>
    <FieldHelp id="{id}-help" field={legend} text={help} />
  </div>
  {#each rows as row, index (index)}
    <div class="grid grid-cols-[minmax(0,2fr)_minmax(0,3fr)_auto] items-start gap-2" data-mcp-row={id}>
      <Input
        class="font-mono"
        aria-describedby="{id}-help"
        aria-invalid={invalid}
        aria-label={m.mcp_value_name()}
        autocomplete="off"
        dir="ltr"
        {disabled}
        maxlength={MAX_NAME_OF_VALUE_CHARS}
        placeholder={namePlaceholder}
        spellcheck="false"
        bind:value={row.name}
      />
      <Input
        class="font-mono"
        aria-invalid={invalid}
        aria-label={m.mcp_value_value()}
        autocomplete="off"
        dir="ltr"
        {disabled}
        placeholder={row.stored === true ? m.mcp_secret_stored() : valuePlaceholder}
        spellcheck="false"
        type={secret ? 'password' : 'text'}
        bind:value={row.value}
      />
      <Button
        aria-label={m.mcp_value_remove({ name: row.name === '' ? m.mcp_value_unnamed() : row.name })}
        {disabled}
        size="icon"
        type="button"
        variant="ghost"
        onclick={() => onRemove(index)}
      >
        <XIcon />
      </Button>
    </div>
    {#if isUnstored(row)}
      <p class="text-xs text-muted-foreground">{m.mcp_secret_not_stored()}</p>
    {/if}
  {/each}
  <div>
    <Button data-mcp-add-row={id} {disabled} size="sm" type="button" variant="outline" onclick={onAdd}>
      <PlusIcon />
      {m.mcp_value_add()}
    </Button>
  </div>
</Field.Set>
