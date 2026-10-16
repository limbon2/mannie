// Messages for the codes and states of the plugins feature (ADR-0019): the
// commands' errors and why a plugin could not start. The UI shows the
// catalog's message for a code and never the core's description; a plugin
// names its own start failures.
import type { PluginsError, PluginView } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import { findPlugin } from '#lib/state/plugin-catalog.js';
import type { Failure } from '#lib/types/calls.js';

/** The ID of the Plugins settings section, which the states of plugins that are off link to. */
export const PLUGINS_SETTINGS_SECTION = 'plugins.settings';

/**
 * Whether a command failed because the plugin of what it acts on is off, so
 * that the owner gets the way to the Plugins settings beside the message
 * (ADR-0030, decision 12).
 */
export function failedForPluginOff(failure: Failure<{ readonly code: string }> | null): boolean {
  const error = failure?.kind === 'core' ? failure.error : null;
  return error?.code === 'plugin_off';
}

/** What to tell the owner about a plugins command that failed, and what they can do. */
export function pluginsFailureMessage(failure: Failure<PluginsError>): string {
  if (failure.kind === 'ipc') {
    return m.shell_error_ipc_failed();
  }
  const { error } = failure;
  switch (error.code) {
    case 'shutting_down': {
      return m.plugins_error_shutting_down();
    }
    case 'storage_failed': {
      return m.plugins_error_storage_failed();
    }
    case 'store_unavailable': {
      return m.plugins_error_store_unavailable();
    }
    case 'unknown_plugin': {
      return m.plugins_error_unknown_plugin();
    }
  }
}

/** Why a plugin could not start, as a sentence: the plugin's own, or the core's for the host's codes. */
export function pluginStartFailureMessage(id: string, code: string): string {
  const own = findPlugin(id)?.failure(code) ?? null;
  if (own !== null) {
    return own;
  }
  switch (code) {
    case 'contributions_refused': {
      return m.plugins_failure_contributions_refused();
    }
    case 'grant_missing': {
      return m.plugins_failure_grant_missing();
    }
    default: {
      return m.plugins_failure_other({ code });
    }
  }
}

/** What a plugin's state line says, or `null` for a plugin that is on or off. */
export function pluginStateMessage(view: PluginView): string | null {
  switch (view.state) {
    case 'failed': {
      const cause = pluginStartFailureMessage(view.id, view.failure ?? '');
      return m.plugins_state_failed({ cause });
    }
    case 'off':
    case 'on': {
      return null;
    }
    case 'starting': {
      return m.plugins_state_starting();
    }
    case 'stopping': {
      return m.plugins_state_stopping();
    }
  }
}
