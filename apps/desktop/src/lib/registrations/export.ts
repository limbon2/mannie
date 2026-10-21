import ExportControl from '#lib/components/app/export/ExportControl.svelte';
import ExportDialog from '#lib/components/app/export/ExportDialog.svelte';
import { m } from '#lib/paraglide/messages.js';
import { exportActions } from '#lib/state/export-actions.svelte.js';
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';
import { openConversationId } from '#lib/utils/chat-urls.js';

/** Export adds controls through the shell, including in conversation side panels. */
export function registerExport(slots: ShellSlots): void {
  slots.conversationControls.register({ component: ExportControl, id: 'export.conversation', order: 30 });
  slots.overlays.register({ component: ExportDialog, id: 'export.result' });
  slots.commands.register({
    available: () => {
      const hasConversation = openConversationId() !== null;
      return hasConversation && !exportActions.busy;
    },
    group: m.export_command_group,
    id: 'export.conversation',
    label: m.export_action,
    order: 0,
    run: () => {
      const id = openConversationId();
      if (id !== null) {
        void exportActions.start(id);
      }
    },
  });
}
