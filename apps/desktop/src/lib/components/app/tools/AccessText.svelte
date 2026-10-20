<script lang="ts">
  // A command or a run of an installed program as an operation's accesses
  // show it: monospaced, wrapped and scrollable. It is content, shown as
  // plain text (T-3).
  let { label, text }: { label: string; text: string } = $props();
</script>

<!-- Focusable, so that the owner can scroll a long text from the keyboard (WCAG 2.1.1). -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<pre
  class="max-h-48 overflow-auto rounded-md bg-muted px-2 py-1.5 font-mono text-xs break-all whitespace-pre-wrap outline-none focus-visible:ring-2 focus-visible:ring-ring"
  aria-label={label}
  data-content
  role="group"
  tabindex="0">{text}</pre>
