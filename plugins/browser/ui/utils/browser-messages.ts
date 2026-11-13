import type { PluginCallFailure } from '@mannie/plugin-sdk/ipc';

import type { BrowserError } from '../generated/bindings.js';
import { m } from '../generated/messages.js';

/** Error text is catalogued. Browser-provided descriptions can contain page data. */
export function browserErrorMessage(code: string): string | null {
  switch (code) {
    case 'browser_disconnected': {
      return m.browser_error_disconnected();
    }
    case 'browser_incompatible': {
      return m.browser_error_incompatible();
    }
    case 'browser_missing': {
      return m.browser_error_missing();
    }
    case 'agent_control': {
      return m.browser_error_agent_control();
    }
    case 'certificate_rejected': {
      return m.browser_error_certificate_rejected();
    }
    case 'certificate_expired': {
      return m.browser_error_certificate_expired();
    }
    case 'certificate_conflict': {
      return m.browser_error_certificate_conflict();
    }
    case 'certificate_journal_failed': {
      return m.browser_error_certificate_journal_failed();
    }
    case 'certificate_store_failed': {
      return m.browser_error_certificate_store_failed();
    }
    case 'certificate_saved_conflict': {
      return m.browser_error_certificate_saved_conflict();
    }
    case 'certificate_revocation_failed': {
      return m.browser_error_certificate_revocation_failed();
    }
    case 'certificate_revoked': {
      return m.browser_error_certificate_revoked();
    }
    case 'credential_missing': {
      return m.browser_error_credential_missing();
    }
    case 'invalid_browser_input':
    case 'invalid_input':
    case 'input_too_large': {
      return m.browser_error_invalid_input();
    }
    case 'limit_reached': {
      return m.browser_error_limit();
    }
    case 'navigation_denied': {
      return m.browser_error_navigation_denied();
    }
    case 'not_found': {
      return m.browser_error_not_found();
    }
    case 'operation_uncertain': {
      return m.browser_error_uncertain();
    }
    case 'owner_control': {
      return m.browser_error_owner_control();
    }
    case 'plugin_off': {
      return m.browser_error_off();
    }
    case 'session_missing': {
      return m.browser_error_session_missing();
    }
    case 'stale_target': {
      return m.browser_error_stale_target();
    }
    case 'unsupported_prompt': {
      return m.browser_error_unsupported_prompt();
    }
    default: {
      return null;
    }
  }
}

/** The sentence of one of the plugin's start failure codes, for the Plugins page. */
export function browserStartFailureMessage(code: string): string | null {
  switch (code) {
    case 'unsupported_platform': {
      return m.browser_failure_unsupported_platform();
    }
    default: {
      return m.browser_start_failed();
    }
  }
}

/** The pane's line about the agent's latest gesture. */
export function browserActivityMessage(kind: string): string {
  switch (kind) {
    case 'click': {
      return m.browser_activity_click();
    }
    case 'fill': {
      return m.browser_activity_fill();
    }
    case 'press': {
      return m.browser_activity_press();
    }
    case 'scroll': {
      return m.browser_activity_scroll();
    }
    case 'select': {
      return m.browser_activity_select();
    }
    default: {
      return '';
    }
  }
}

export function browserFailureMessage(failure: PluginCallFailure<BrowserError>): string {
  const code = failure.kind === 'core' ? failure.error.code : null;
  return (code === null ? null : browserErrorMessage(code)) ?? m.browser_error_ipc();
}
