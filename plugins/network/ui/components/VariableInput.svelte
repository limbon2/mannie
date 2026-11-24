<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Input } from '@mannie/plugin-sdk/components/input';

  import { m } from '../generated/messages.js';
  import { extractVariableNames, type VariableScope, type VariableSuggestion } from '../utils/network-urls.js';

  interface Props {
    ariaLabel?: string;
    class?: string;
    disabled?: boolean;
    id?: string;
    onkeydown?: (event: KeyboardEvent) => void;
    onValueChange: (next: string) => void;
    placeholder?: string;
    readonly?: boolean;
    required?: boolean;
    value: string;
    variables?: readonly VariableSuggestion[];
    wrapperClass?: string;
  }

  let {
    ariaLabel,
    class: className = '',
    disabled = false,
    id,
    onkeydown,
    onValueChange,
    placeholder = '',
    readonly = false,
    required = false,
    value,
    variables = [],
    wrapperClass = '',
  }: Props = $props();

  const listId = $props.id();

  let inputEl = $state<HTMLInputElement | null>(null);
  let triggerStart = $state<number | null>(null);
  let triggerEnd = $state(0);
  let filterQuery = $state('');
  let activeIndex = $state(0);
  let explicitOpen = $state(false);

  const suggestions = $derived.by(() => {
    const hasTrigger = triggerStart !== null;

    if (!hasTrigger && !explicitOpen) {
      return [];
    }
    const needle = filterQuery.trim().toLowerCase();
    if (needle === '') {
      return variables;
    }
    return variables.filter((item) => item.key.toLowerCase().includes(needle));
  });

  const open = $derived.by(() => {
    const isInteractive = !disabled && !readonly;
    const isActive = triggerStart !== null || explicitOpen;
    const hasMatches = suggestions.length > 0;

    return isInteractive && isActive && hasMatches;
  });

  const referencedVariables = $derived(extractVariableNames(value));
  const hasReferencedVariables = $derived(referencedVariables.length > 0);
  const referencedTooltip = $derived(
    hasReferencedVariables ? m.network_variable_references_label({ names: referencedVariables.join(', ') }) : '',
  );

  function scopeLabel(scope: VariableScope): string {
    switch (scope) {
      case 'environment': {
        return m.network_variable_scope_environment();
      }
      case 'collection': {
        return m.network_variable_scope_collection();
      }
      case 'global': {
        return m.network_variable_scope_global();
      }
    }
  }

  function closeMenu(): void {
    triggerStart = null;
    explicitOpen = false;
    filterQuery = '';
    activeIndex = 0;
  }

  function inspectCaret(text: string, caret: number | null): void {
    const noCaret = caret === null;
    const noVariables = variables.length === 0;

    if (noCaret || noVariables || disabled || readonly) {
      closeMenu();
      return;
    }

    const before = text.slice(0, caret);
    const match = /\{\{([\w.-]*)$/u.exec(before);

    if (match === null) {
      if (!explicitOpen) {
        closeMenu();
      }
      return;
    }

    const start = caret - match[0].length;
    triggerStart = start;
    triggerEnd = caret;
    filterQuery = match[1];
    activeIndex = 0;
  }

  function handleInput(event: Event): void {
    const target = event.currentTarget as HTMLInputElement;
    const next = target.value;

    explicitOpen = false;
    onValueChange(next);
    inspectCaret(next, target.selectionStart);
  }

  function applySuggestion(item: VariableSuggestion): void {
    const start = triggerStart ?? inputEl?.selectionStart ?? value.length;
    const end = triggerStart !== null ? triggerEnd : (inputEl?.selectionEnd ?? value.length);
    const trailingCloses = value.slice(end, end + 2) === '}}';
    const replaceEnd = trailingCloses ? end + 2 : end;
    const inserted = `{{${item.key}}}`;
    const next = `${value.slice(0, start)}${inserted}${value.slice(replaceEnd)}`;
    const nextCaret = start + inserted.length;

    closeMenu();
    onValueChange(next);

    queueMicrotask(() => {
      if (inputEl !== null) {
        inputEl.focus();
        inputEl.setSelectionRange(nextCaret, nextCaret);
      }
    });
  }

  function handleKeydown(event: KeyboardEvent): void {
    const isModPressed = event.ctrlKey || event.metaKey;
    const isSpace = event.key === ' ';
    const canOpenExplicitly = !disabled && !readonly && variables.length > 0;

    if (isModPressed && isSpace && canOpenExplicitly) {
      event.preventDefault();
      const caret = inputEl?.selectionStart ?? value.length;
      explicitOpen = true;
      inspectCaret(value, caret);
      if (triggerStart === null) {
        triggerStart = caret;
        triggerEnd = caret;
        filterQuery = '';
        activeIndex = 0;
      }
      return;
    }

    if (open) {
      if (event.key === 'ArrowDown') {
        event.preventDefault();
        activeIndex = (activeIndex + 1) % suggestions.length;
        return;
      }
      if (event.key === 'ArrowUp') {
        event.preventDefault();
        activeIndex = (activeIndex - 1 + suggestions.length) % suggestions.length;
        return;
      }
      const isConfirmKey = event.key === 'Enter' || event.key === 'Tab';

      if (isConfirmKey) {
        const chosen = suggestions.at(activeIndex);
        if (chosen !== undefined) {
          event.preventDefault();
          applySuggestion(chosen);
          return;
        }
      }
      if (event.key === 'Escape') {
        event.preventDefault();
        closeMenu();
        return;
      }
    }

    onkeydown?.(event);
  }

  function handleClick(): void {
    if (inputEl !== null) {
      inspectCaret(inputEl.value, inputEl.selectionStart);
    }
  }

  function handleOptionMouseDown(event: MouseEvent, item: VariableSuggestion): void {
    event.preventDefault();
    applySuggestion(item);
  }

  function previewValue(item: VariableSuggestion): string {
    if (item.secret) {
      return m.network_variable_redacted_preview();
    }
    return item.value;
  }

  function subtitleFor(item: VariableSuggestion): string {
    return previewValue(item) || item.description.trim();
  }
</script>

<div class="relative min-w-0 {wrapperClass}">
  <Input
    {id}
    class="{hasReferencedVariables ? 'pe-11' : ''} {className}"
    aria-activedescendant={open ? `${listId}-option-${activeIndex}` : undefined}
    aria-autocomplete={variables.length > 0 ? 'list' : undefined}
    aria-controls={open ? listId : undefined}
    aria-label={ariaLabel}
    autocomplete="off"
    {disabled}
    {placeholder}
    {readonly}
    {required}
    {value}
    bind:ref={inputEl}
    onblur={closeMenu}
    onclick={handleClick}
    oninput={handleInput}
    onkeydown={handleKeydown}
  />

  {#if hasReferencedVariables}
    <span
      class="pointer-events-none absolute inset-e-2 top-1/2 -translate-y-1/2 rounded-[3px] bg-token-variable px-1 py-0.5 font-mono text-[10px] leading-none font-medium text-token-variable-foreground ring-1 ring-token-variable-border/55 ring-inset"
      data-content=""
      data-network-variable-indicator=""
      title={referencedTooltip}
    >
      {referencedVariables.length > 1 ? `{{${referencedVariables.length}}}` : '{{}}'}
    </span>
  {/if}

  {#if open}
    <div
      id={listId}
      class="absolute inset-s-0 top-full z-50 mt-1 max-h-56 w-full max-w-md min-w-64 overflow-y-auto rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-md"
      aria-label={m.network_variable_suggestions_label()}
      data-mention-list=""
      data-network-variable-list=""
      role="listbox"
    >
      {#each suggestions as item, index (item.key)}
        {const subtitle = subtitleFor(item)}
        <button
          id={`${listId}-option-${index}`}
          class="flex w-full flex-col gap-0.5 rounded-sm px-2 py-1.5 text-start text-xs outline-none hover:bg-accent"
          aria-selected={index === activeIndex}
          data-mention-option=""
          data-network-variable-option={item.key}
          role="option"
          type="button"
          onmousedown={(event: MouseEvent) => handleOptionMouseDown(event, item)}
        >
          <div class="flex items-center justify-between gap-2">
            <span
              class="truncate rounded-[3px] bg-token-variable px-1 font-mono font-medium text-token-variable-foreground ring-1 ring-token-variable-border/55 ring-inset"
              data-content=""
              dir="ltr"
            >
              {`{{${item.key}}}`}
            </span>
            <Badge class="h-4 shrink-0 rounded-sm px-1 text-[10px]" variant="secondary">
              {scopeLabel(item.scope)}
            </Badge>
          </div>

          {#if subtitle !== ''}
            <span class="truncate font-mono text-[11px] text-muted-foreground" data-content="" dir="ltr">
              {subtitle}
            </span>
          {/if}
        </button>
      {/each}
    </div>
  {/if}
</div>
