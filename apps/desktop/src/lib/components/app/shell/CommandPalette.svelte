<script lang="ts">
  import * as Command from '#lib/components/ui/command/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { activities } from '#lib/state/activities.svelte.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import type { ShellCommand } from '#lib/types/shell-slots.js';
  import { groupCommands, navigationCommands } from '#lib/utils/commands.js';

  let { open = $bindable(false) }: { open?: boolean } = $props();

  const slots = getShellSlots();

  // bits-ui points the input's aria-controls at Command.Viewport, which this
  // palette does not use, so the palette links the input to its list itself.
  const uid = $props.id();
  const listId = `${uid}-list`;

  // Read while the palette is open, so that a command's availability follows
  // the state it depends on. The entries of the activity bar come first.
  const groups = $derived(open ? groupCommands([...viewCommands(), ...slots.commands.entries]) : []);

  function viewCommands(): ShellCommand[] {
    return navigationCommands(activities.entries, (entry) => void activities.select(entry));
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
