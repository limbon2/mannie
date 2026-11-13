<script lang="ts">
  import CheckIcon from '@lucide/svelte/icons/check';
  import { RadioGroup as RadioGroupPrimitive } from 'bits-ui';

  import * as Field from '#lib/components/ui/field/index.js';
  import * as RadioGroup from '#lib/components/ui/radio-group/index.js';
  import * as Tooltip from '#lib/components/ui/tooltip/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { Message } from '#lib/types/shell-slots.js';

  interface PaletteChoice {
    readonly label: Message;
    readonly swatch: string;
    readonly value: string;
  }

  interface PaletteGroup {
    readonly choices: readonly PaletteChoice[];
    readonly defaultValue: string;
    readonly id: 'primary' | 'surface';
    readonly label: Message;
  }

  const primary: PaletteGroup = {
    choices: [
      { label: m.shell_settings_palette_default, swatch: 'bg-foreground', value: 'default' },
      { label: m.shell_settings_palette_blue, swatch: 'bg-palette-swatch-blue', value: 'blue' },
      { label: m.shell_settings_palette_teal, swatch: 'bg-palette-swatch-teal', value: 'teal' },
      { label: m.shell_settings_palette_green, swatch: 'bg-palette-swatch-green', value: 'green' },
      { label: m.shell_settings_palette_amber, swatch: 'bg-palette-swatch-amber', value: 'amber' },
      { label: m.shell_settings_palette_violet, swatch: 'bg-palette-swatch-violet', value: 'violet' },
      { label: m.shell_settings_palette_rose, swatch: 'bg-palette-swatch-rose', value: 'rose' },
    ],
    defaultValue: 'default',
    id: 'primary',
    label: m.shell_settings_palette_primary,
  };

  const surface: PaletteGroup = {
    choices: [
      { label: m.shell_settings_palette_neutral, swatch: 'bg-palette-swatch-neutral', value: 'neutral' },
      { label: m.shell_settings_palette_slate, swatch: 'bg-palette-swatch-slate', value: 'slate' },
      { label: m.shell_settings_palette_stone, swatch: 'bg-palette-swatch-stone', value: 'stone' },
      { label: m.shell_settings_palette_olive, swatch: 'bg-palette-swatch-olive', value: 'olive' },
    ],
    defaultValue: 'neutral',
    id: 'surface',
    label: m.shell_settings_palette_surface,
  };

  const groups = [primary, surface];
  const selected = $state({ primary: appliedChoice(primary), surface: appliedChoice(surface) });

  function appliedChoice(group: PaletteGroup): string {
    // The head script has applied the saved choices before the settings mount.
    const classes = document.documentElement.classList;
    const choice = group.choices.find(({ value }) => classes.contains(`palette-${group.id}-${value}`));
    return choice?.value ?? group.defaultValue;
  }

  function changePalette(group: PaletteGroup, value: string): void {
    const alreadySelected = value === selected[group.id];
    const isAvailable = group.choices.some((choice) => choice.value === value);
    if (alreadySelected || !isAvailable) {
      return;
    }
    const classes = document.documentElement.classList;
    classes.remove(`palette-${group.id}-${selected[group.id]}`);
    if (value !== group.defaultValue) {
      classes.add(`palette-${group.id}-${value}`);
    }
    selected[group.id] = value;
    try {
      localStorage.setItem(`mannie.theme.${group.id}`, value);
    } catch {
      // Without writable storage, the choice still applies to this window.
    }
  }
</script>

<Tooltip.Provider>
  {#each groups as group (group.id)}
    <Field.Set>
      <Field.Legend id="interface-palette-{group.id}" variant="label">{group.label()}</Field.Legend>
      <RadioGroup.Root
        class="flex flex-wrap gap-2"
        aria-labelledby="interface-palette-{group.id}"
        loop
        orientation="horizontal"
        value={selected[group.id]}
        onValueChange={(value: string) => changePalette(group, value)}
      >
        {#each group.choices as choice (choice.value)}
          {const label = $derived(choice.label())}
          <Tooltip.Root>
            <Tooltip.Trigger id="interface-{group.id}-{choice.value}">
              {#snippet child({ props })}
                <RadioGroupPrimitive.Item
                  {...props}
                  class="grid size-8 shrink-0 cursor-pointer place-items-center rounded-full focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
                  aria-label={label}
                  value={choice.value}
                >
                  {#snippet children({ checked })}
                    <span
                      class={['grid size-7 place-items-center rounded-full border border-foreground/65', choice.swatch]}
                      aria-hidden="true"
                    >
                      {#if checked}
                        <span class="grid size-4 place-items-center rounded-full bg-background text-foreground">
                          <CheckIcon class="size-3" strokeWidth={3} />
                        </span>
                      {/if}
                    </span>
                  {/snippet}
                </RadioGroupPrimitive.Item>
              {/snippet}
            </Tooltip.Trigger>
            <Tooltip.Content sideOffset={4}>{label}</Tooltip.Content>
          </Tooltip.Root>
        {/each}
      </RadioGroup.Root>
    </Field.Set>
  {/each}
</Tooltip.Provider>
