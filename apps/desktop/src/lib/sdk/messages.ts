// Messages of the TypeScript plugin SDK, `@mannie/plugin-sdk/messages`
// (ADR-0034, decisions 9 and 11): the host refuses a plugin's calls the same
// way for every plugin, and a call can get no answer at all, so the core
// words those once (ADR-0019), and a plugin's UI words only its own codes.
import type { HostRefusal } from '#lib/generated/plugin-sdk.js';
import { m } from '#lib/paraglide/messages.js';
import { formatCount } from '#lib/utils/chat-errors.js';

/** What to tell the owner when a call got no answer, because the window could not reach the core. */
export function ipcFailureMessage(): string {
  return m.shell_error_ipc_failed();
}

/** What to tell the owner when the host refused a call of the plugin whose name is `name`. */
export function hostRefusalMessage(refusal: HostRefusal, name: string): string {
  switch (refusal.code) {
    case 'host_service_failed': {
      return m.plugins_host_host_service_failed();
    }
    case 'input_too_large': {
      return m.plugins_host_input_too_large({ limit: formatCount(refusal.maxBytes), name });
    }
    case 'invalid_input': {
      return m.plugins_host_invalid_input({ name });
    }
    case 'not_allowed': {
      return m.plugins_host_not_allowed({ name });
    }
    case 'plugin_off': {
      return m.plugins_host_plugin_off({ name });
    }
    case 'shutting_down': {
      return m.plugins_host_shutting_down();
    }
    case 'store_unavailable': {
      return m.plugins_host_store_unavailable();
    }
    case 'unknown_command': {
      return m.plugins_host_unknown_command({ name });
    }
  }
}
