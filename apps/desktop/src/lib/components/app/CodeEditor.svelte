<script lang="ts">
  import { untrack } from 'svelte';

  import { cn } from '#lib/components/ui/utils.js';
  import type {
    CodeCompletionItem,
    CodeEditorHandle,
    CodeEditorHighlightMode,
    CodeEditorLanguage,
  } from '#lib/sdk/code-editor.js';

  interface Props {
    /** Accessible name for screen readers and keyboard focus. */
    ariaLabel: string;
    /** Extra layout classes (typically a fixed height such as `h-48` or `h-full`). */
    class?: string;
    /** Optional completion provider for `{{variable}}` expressions. */
    completions?: () => readonly CodeCompletionItem[];
    /** Extra inline token highlighting mode (`none`, `variables`, or `url`). */
    highlightTokens?: CodeEditorHighlightMode;
    /** Optional DOM `id` applied to the editable content area. */
    id?: string;
    /** Active syntax highlighting language. */
    language?: CodeEditorLanguage;
    /** Whether to display line numbers and fold markers. */
    lineNumbers?: boolean;
    /** Called when the owner edits the text. */
    onchange?: (next: string) => void;
    /** Called when the owner presses Enter in single-line mode. */
    onEnter?: () => void;
    /** Called when the owner presses Ctrl+Enter or Cmd+Enter. */
    onModEnter?: () => void;
    /** Placeholder shown when the editor is empty. */
    placeholder?: string;
    /** Whether the buffer is read-only. */
    readonly?: boolean;
    /** Restricts the editor to one horizontally scrolling line (for URL bars). */
    singleLine?: boolean;
    /** Optional `data-testid` applied to the wrapper element. */
    testId?: string;
    /** Current document text. */
    value?: string;
    /** Whether long lines wrap to fit horizontally. */
    wrapLines?: boolean;
  }

  let {
    ariaLabel,
    class: className = '',
    completions,
    highlightTokens = 'none',
    id,
    language = 'plain',
    lineNumbers = true,
    onchange,
    onEnter,
    onModEnter,
    placeholder = '',
    readonly = false,
    singleLine = false,
    testId,
    value = $bindable(''),
    wrapLines = true,
  }: Props = $props();

  let hostEl = $state<HTMLDivElement | null>(null);
  let editor = $state.raw<CodeEditorHandle | null>(null);
  let ready = $state(false);

  function handleEditorChange(next: string): void {
    value = next;
    onchange?.(next);
  }

  function handleFallbackInput(event: Event): void {
    const target = event.currentTarget as HTMLInputElement | HTMLTextAreaElement;
    value = target.value;
    onchange?.(target.value);
  }

  function handleFallbackKeydown(event: KeyboardEvent): void {
    const isModPressed = event.ctrlKey || event.metaKey;
    const isEnter = event.key === 'Enter';
    const hasModEnter = onModEnter !== undefined;

    if (isModPressed && isEnter && hasModEnter) {
      event.preventDefault();
      onModEnter();
      return;
    }

    const isPlainEnter = singleLine && isEnter && !isModPressed;
    if (isPlainEnter) {
      event.preventDefault();
      onEnter?.();
    }
  }

  function readCompletions(): readonly CodeCompletionItem[] {
    return completions === undefined ? [] : completions();
  }

  function invokeEnter(): void {
    onEnter?.();
  }

  function invokeModEnter(): void {
    onModEnter?.();
  }

  $effect(() => {
    const target = hostEl;
    const currentAriaLabel = ariaLabel;
    const currentHighlightTokens = highlightTokens;
    const currentId = id;
    const currentLineNumbers = lineNumbers;
    const currentSingleLine = singleLine;
    const currentWrapLines = wrapLines;
    const isMissingHost = target === null;
    const isServer = typeof document === 'undefined';

    if (isMissingHost || isServer) {
      return;
    }

    let canceled = false;
    let mounted: CodeEditorHandle | null = null;

    const initial = untrack(() => ({
      hasCompletions: completions !== undefined,
      hasEnter: onEnter !== undefined,
      hasModEnter: onModEnter !== undefined,
      language,
      placeholder,
      readonly,
      value,
    }));

    void import('#lib/sdk/code-editor.js')
      .then(({ createCodeEditor }) =>
        createCodeEditor(target, {
          ariaLabel: currentAriaLabel,
          completions: initial.hasCompletions ? readCompletions : undefined,
          highlightTokens: currentHighlightTokens,
          id: currentId,
          language: initial.language,
          lineNumbers: currentLineNumbers,
          onChange: handleEditorChange,
          onEnter: initial.hasEnter ? invokeEnter : undefined,
          onModEnter: initial.hasModEnter ? invokeModEnter : undefined,
          placeholder: initial.placeholder,
          readonly: initial.readonly,
          singleLine: currentSingleLine,
          value: initial.value,
          wrapLines: currentWrapLines,
        }),
      )
      .then((handle) => {
        if (canceled) {
          handle.dispose();
          return;
        }

        mounted = handle;
        editor = handle;
        ready = true;
      });

    return () => {
      canceled = true;
      ready = false;
      editor = null;
      mounted?.dispose();
    };
  });

  $effect(() => {
    if (editor !== null) {
      editor.setValue(value);
    }
  });

  $effect(() => {
    if (editor !== null) {
      void editor.setLanguage(language);
    }
  });

  $effect(() => {
    if (editor !== null) {
      editor.setReadonly(readonly);
    }
  });

  $effect(() => {
    if (editor !== null) {
      editor.setPlaceholder(placeholder);
    }
  });
</script>

<div
  class={cn(
    'relative overflow-hidden rounded-md border border-input bg-background text-xs focus-within:border-ring focus-within:ring-[3px] focus-within:ring-ring/50',
    singleLine ? 'h-9 min-h-0' : 'min-h-24',
    className,
  )}
  data-code-editor=""
  data-code-editor-ready={ready ? 'true' : 'false'}
  data-language={language}
  data-single-line={singleLine ? 'true' : undefined}
  data-testid={testId}
>
  <div bind:this={hostEl} class="size-full" class:hidden={!ready}></div>

  {#if !ready}
    {#if readonly}
      <pre
        class="size-full overflow-auto p-3 font-mono text-xs break-all whitespace-pre-wrap"
        aria-label={ariaLabel}
        data-content="">{value}</pre>
    {:else if singleLine}
      <input
        {id}
        class="size-full bg-transparent px-3 font-mono text-xs outline-none placeholder:text-muted-foreground"
        aria-label={ariaLabel}
        autocomplete="off"
        {placeholder}
        type="text"
        {value}
        oninput={handleFallbackInput}
        onkeydown={handleFallbackKeydown}
      />
    {:else}
      <textarea
        {id}
        class="size-full resize-none bg-transparent p-3 font-mono text-xs outline-none placeholder:text-muted-foreground"
        aria-label={ariaLabel}
        {placeholder}
        {value}
        oninput={handleFallbackInput}
        onkeydown={handleFallbackKeydown}></textarea>
    {/if}
  {/if}
</div>
