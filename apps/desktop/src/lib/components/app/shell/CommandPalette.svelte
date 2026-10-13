<script lang="ts">
  import * as Command from '#lib/components/ui/command/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import type { ShellCommand } from '#lib/types/shell-slots.js';
  import { sortedByOrder } from '#lib/utils/order.js';

  interface CommandGroup {
    readonly commands: ShellCommand[];
    readonly heading: string;
  }

  let { open = $bindable(false) }: { open?: boolean } = $props();

  const slots = getShellSlots();

  // bits-ui points the input's aria-controls at Command.Viewport, which this
  // palette does not use, so the palette links the input to its list itself.
  const uid = $props.id();
  const listId = `${uid}-list`;

  const groups = $derived(groupByHeading(slots.commands.entries));

  /** Groups the commands under their group's heading, in the order the groups first appear. */
  function groupByHeading(commands: readonly ShellCommand[]): CommandGroup[] {
    const headings = [...new Set(commands.map((command) => command.group()))];
    return headings.map((heading) => ({
      commands: sortedByOrder(commands.filter((command) => command.group() === heading)),
      heading,
    }));
  }

  function run(command: ShellCommand): void {
    open = false;
    command.run();
  }
</script>

<Command.Dialog description={m.shell_command_palette_description()} title={m.shell_command_palette_title()} bind:open>
  <Command.Input aria-controls={listId} placeholder={m.shell_command_palette_placeholder()} />
  <Command.List id={listId} aria-label={m.shell_command_palette_results()}>
    <Command.Empty>{m.shell_command_palette_empty()}</Command.Empty>
    {#each groups as group (group.heading)}
      <Command.Group heading={group.heading}>
        {#each group.commands as command (command.id)}
          <Command.Item value={command.label()} onSelect={() => run(command)}>
            {command.label()}
          </Command.Item>
        {/each}
      </Command.Group>
    {/each}
  </Command.List>
</Command.Dialog>
