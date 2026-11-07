<script lang="ts">
  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';

  // The two commands of the skills CLI that install where Mannie reads
  // (ADR-0061, decision 2), each with a button that copies it on the owner's
  // click; the webview never reads the clipboard.
  interface SkillCommand {
    readonly id: 'computer' | 'project';
    readonly label: string;
    readonly text: string;
  }

  interface Copied {
    readonly id: SkillCommand['id'];
    readonly result: 'failed' | 'succeeded';
  }

  let copied = $state<Copied | null>(null);

  const skillCommands: readonly SkillCommand[] = $derived([
    { id: 'computer', label: m.skills_command_computer_label(), text: m.skills_command_computer() },
    { id: 'project', label: m.skills_command_project_label(), text: m.skills_command_project() },
  ]);

  async function copy(command: SkillCommand): Promise<void> {
    try {
      await navigator.clipboard.writeText(command.text);
      copied = { id: command.id, result: 'succeeded' };
    } catch {
      // WebKitGTK can refuse the write; the owner learns that it failed.
      copied = { id: command.id, result: 'failed' };
    }
  }

  function status(id: SkillCommand['id']): string {
    if (copied?.id !== id) {
      return '';
    }
    return copied.result === 'succeeded' ? m.skills_copy_succeeded() : m.skills_copy_failed();
  }
</script>

<dl class="flex flex-col gap-2">
  {#each skillCommands as command (command.id)}
    <div class="flex flex-col gap-1">
      <dt class="text-sm font-medium">{command.label}</dt>
      <dd class="flex items-center gap-2 rounded-md border bg-muted px-3 py-1">
        <code class="min-w-0 flex-1 font-mono text-sm wrap-break-word" data-content>{command.text}</code>
        <span class="text-xs text-muted-foreground" role="status">{status(command.id)}</span>
        <Button
          aria-label={m.skills_copy_label({ label: command.label })}
          data-skills-copy={command.id}
          size="sm"
          variant="ghost"
          onclick={() => void copy(command)}
        >
          {m.skills_copy()}
        </Button>
      </dd>
    </div>
  {/each}
</dl>
