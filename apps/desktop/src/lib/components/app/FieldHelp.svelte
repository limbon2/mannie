<script lang="ts">
  import CircleQuestionMarkIcon from '@lucide/svelte/icons/circle-question-mark';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as Tooltip from '#lib/components/ui/tooltip/index.js';
  import { m } from '#lib/paraglide/messages.js';

  // A field's help text behind a "(?)" button beside the field's label
  // (docs/engineering/standards.md, UI and visible text). Hover and keyboard
  // focus show it in a tooltip. It also stays in the page, visually hidden,
  // under `id`, so the field's control names it in `aria-describedby` and
  // screen readers keep the description that the page no longer shows. The
  // tooltip brings its own provider, because dialogs that features mount as
  // overlays render outside the sidebar's provider.
  interface Props {
    /** The field's label, which names the button. */
    field: string;
    /** The ID of the hidden help text, for the control's `aria-describedby`. */
    id: string;
    /** The help text, from the message catalog. */
    text: string;
  }

  let { field, id, text }: Props = $props();
</script>

<Tooltip.Provider delayDuration={0}>
  <Tooltip.Root>
    <Tooltip.Trigger>
      {#snippet child({ props })}
        <Button
          {...props}
          class="-my-1 text-muted-foreground"
          aria-label={m.shell_field_help({ field })}
          data-field-help=""
          size="icon-xs"
          variant="ghost"
        >
          <CircleQuestionMarkIcon class="size-3.5" />
        </Button>
      {/snippet}
    </Tooltip.Trigger>
    <Tooltip.Content>{text}</Tooltip.Content>
  </Tooltip.Root>
</Tooltip.Provider>
<span {id} class="sr-only">{text}</span>
