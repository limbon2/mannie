<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Checkbox } from '@mannie/plugin-sdk/components/checkbox';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import TrashIcon from '@mannie/plugin-sdk/icons/trash';

  import type { KeyValueEntry, VariableEntry } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import type { VariableSuggestion } from '../utils/network-urls.js';

  import VariableInput from './VariableInput.svelte';

  type RowItem = KeyValueEntry | VariableEntry;
  type EditorKind = 'default' | 'path' | 'query' | 'variable';

  interface Props {
    allowAddRemove?: boolean;
    allowSecret?: boolean;
    disabled?: boolean;
    emptyMessage?: string;
    entries: RowItem[];
    keyPlaceholder?: string;
    kind?: EditorKind;
    lockKeys?: boolean;
    onChange: (next: RowItem[]) => void;
    showDescription?: boolean;
    showEnabled?: boolean;
    valuePlaceholder?: string;
    variables?: readonly VariableSuggestion[];
  }

  let {
    allowAddRemove = true,
    allowSecret = true,
    disabled = false,
    emptyMessage = '',
    entries,
    keyPlaceholder = '',
    kind = 'default',
    lockKeys = false,
    onChange,
    showDescription = true,
    showEnabled = true,
    valuePlaceholder = '',
    variables = [],
  }: Props = $props();

  const isLockedPath = $derived(lockKeys && kind === 'path');

  function addRow(): void {
    const row: KeyValueEntry = {
      description: '',
      enabled: true,
      key: '',
      secret: false,
      value: '',
    };
    onChange([...entries, row]);
  }

  function removeRow(index: number): void {
    onChange(entries.filter((_, idx) => idx !== index));
  }

  function updateField<K extends keyof KeyValueEntry>(index: number, field: K, value: KeyValueEntry[K]): void {
    const next = entries.map((entry, idx) => (idx === index ? { ...entry, [field]: value } : entry));
    onChange(next);
  }
</script>

<div class="flex flex-col gap-2">
  {#if entries.length === 0}
    <p class="text-xs text-muted-foreground">{emptyMessage || m.network_kv_empty()}</p>
  {:else}
    <div class="overflow-visible rounded-md border border-border">
      <table class="w-full border-collapse text-xs">
        <thead>
          <tr class="border-b border-border bg-muted/40 text-muted-foreground">
            {#if showEnabled}
              <th class="w-8 p-2 text-center font-medium">
                <span class="sr-only">{m.network_kv_enabled()}</span>
              </th>
            {/if}

            <th class="p-2 text-start font-medium">{m.network_kv_key()}</th>
            <th class="p-2 text-start font-medium">{m.network_kv_value()}</th>

            {#if showDescription}
              <th class="p-2 text-start font-medium">{m.network_kv_description()}</th>
            {/if}

            {#if allowSecret}
              <th class="w-16 p-2 text-center font-medium">{m.network_kv_secret()}</th>
            {/if}

            {#if allowAddRemove}
              <th class="w-10 p-2 text-end font-medium">
                <span class="sr-only">{m.network_remove()}</span>
              </th>
            {/if}
          </tr>
        </thead>

        <tbody>
          {#each entries as entry, index (lockKeys ? entry.key : index)}
            <tr class="border-b border-border last:border-b-0">
              {#if showEnabled}
                <td class="p-1.5 text-center">
                  <Checkbox
                    aria-label={m.network_kv_enabled()}
                    checked={entry.enabled}
                    {disabled}
                    onCheckedChange={(checked: boolean) => updateField(index, 'enabled', checked)}
                  />
                </td>
              {/if}

              <td class="p-1.5">
                {#if isLockedPath}
                  <span
                    class="inline-flex h-8 w-full items-center rounded-md border border-token-path-border/55 bg-token-path px-2.5 font-mono text-xs font-medium text-token-path-foreground"
                    data-content=""
                    data-network-token-kind="path"
                    dir="ltr"
                  >
                    {`{${entry.key}}`}
                  </span>
                {:else if lockKeys}
                  <span
                    class="inline-flex h-8 w-full items-center rounded-md border border-transparent bg-muted/40 px-2.5 font-mono text-xs text-foreground"
                    data-content=""
                    dir="ltr"
                  >
                    {entry.key}
                  </span>
                {:else if kind === 'query'}
                  <div class="relative flex items-center" data-network-token-kind="query">
                    <span
                      class="pointer-events-none absolute inset-s-2.5 font-mono text-xs font-semibold text-token-query-foreground"
                      aria-hidden="true"
                    >
                      ?
                    </span>
                    <Input
                      class="h-8 ps-6 font-mono text-xs"
                      aria-label={m.network_kv_key()}
                      {disabled}
                      placeholder={keyPlaceholder || m.network_kv_key()}
                      value={entry.key}
                      oninput={(event: Event) =>
                        updateField(index, 'key', (event.currentTarget as HTMLInputElement).value)}
                    />
                  </div>
                {:else if kind === 'variable'}
                  <div class="relative flex items-center" data-network-token-kind="variable">
                    <span
                      class="pointer-events-none absolute inset-s-2 font-mono text-[11px] font-medium text-token-variable-foreground"
                      aria-hidden="true"
                    >
                      {'{{'}
                    </span>
                    <Input
                      class="h-8 ps-6 pe-6 font-mono text-xs"
                      aria-label={m.network_kv_key()}
                      {disabled}
                      placeholder={keyPlaceholder || m.network_kv_key()}
                      value={entry.key}
                      oninput={(event: Event) =>
                        updateField(index, 'key', (event.currentTarget as HTMLInputElement).value)}
                    />
                    <span
                      class="pointer-events-none absolute inset-e-2 font-mono text-[11px] font-medium text-token-variable-foreground"
                      aria-hidden="true"
                    >
                      }}
                    </span>
                  </div>
                {:else}
                  <Input
                    class="h-8 font-mono text-xs"
                    aria-label={m.network_kv_key()}
                    {disabled}
                    placeholder={keyPlaceholder || m.network_kv_key()}
                    value={entry.key}
                    oninput={(event: Event) =>
                      updateField(index, 'key', (event.currentTarget as HTMLInputElement).value)}
                  />
                {/if}
              </td>

              <td class="p-1.5">
                <VariableInput
                  class="h-8 font-mono text-xs"
                  ariaLabel={m.network_kv_value()}
                  {disabled}
                  placeholder={valuePlaceholder || m.network_kv_value()}
                  value={entry.value}
                  {variables}
                  onValueChange={(next: string) => updateField(index, 'value', next)}
                />
              </td>

              {#if showDescription}
                <td class="p-1.5">
                  <Input
                    class="h-8 text-xs"
                    aria-label={m.network_kv_description()}
                    {disabled}
                    placeholder={m.network_kv_description()}
                    value={entry.description}
                    oninput={(event: Event) =>
                      updateField(index, 'description', (event.currentTarget as HTMLInputElement).value)}
                  />
                </td>
              {/if}

              {#if allowSecret}
                <td class="p-1.5 text-center">
                  <Checkbox
                    aria-label={m.network_kv_secret()}
                    checked={entry.secret}
                    {disabled}
                    onCheckedChange={(checked: boolean) => updateField(index, 'secret', checked)}
                  />
                </td>
              {/if}

              {#if allowAddRemove}
                <td class="p-1.5 text-end">
                  <Button
                    aria-label={m.network_kv_remove_row({ name: entry.key.trim() || m.network_kv_unnamed() })}
                    {disabled}
                    size="icon-xs"
                    type="button"
                    variant="ghost"
                    onclick={() => removeRow(index)}
                  >
                    <TrashIcon class="size-3.5" />
                  </Button>
                </td>
              {/if}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}

  {#if allowAddRemove}
    <div>
      <Button {disabled} size="sm" type="button" variant="outline" onclick={addRow}>
        <PlusIcon data-icon="inline-start" />
        {m.network_kv_add_row()}
      </Button>
    </div>
  {/if}
</div>
