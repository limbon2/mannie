import { hostRefusalMessage, ipcFailureMessage } from '@mannie/plugin-sdk/messages';
// Messages for the SSH plugin's codes (ADR-0019): the core words a call that
// got no answer and the host's refusals, and the plugin its own errors, the
// statuses of hosts, the fields that a check refused and its mentions.
import type { MentionSummary } from '@mannie/plugin-sdk/slots';

import type { HostStatus, SshError } from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import type { SshFailure } from '../types/ssh.js';

/** The plugin's own errors by their codes, which the type checks hold to the plugin's error type. */
const ERRORS: Record<SshError['code'], () => string> = {
  auth_failed: () => m.ssh_error_auth_failed(),
  auth_method_unsupported: () => m.ssh_error_auth_method_unsupported(),
  credential_store_unavailable: () => m.ssh_error_credential_store_unavailable(),
  handshake_failed: () => m.ssh_error_handshake_failed(),
  host_key_changed: () => m.ssh_error_host_key_changed(),
  host_key_not_trusted: () => m.ssh_error_host_key_not_trusted(),
  host_key_unsupported: () => m.ssh_error_host_key_unsupported(),
  host_not_found: () => m.ssh_error_host_not_found(),
  name_taken: () => m.ssh_error_name_taken(),
  password_missing: () => m.ssh_error_password_missing(),
  scan_outdated: () => m.ssh_error_scan_outdated(),
  storage_failed: () => m.ssh_error_storage_failed(),
  timed_out: () => m.ssh_error_timed_out(),
  unreachable: () => m.ssh_error_unreachable(),
};

/** What a field that the host's check refused needs, by the field's name in the input. */
const FIELDS: Readonly<Record<string, () => string>> = {
  address: () => m.ssh_invalid_address(),
  name: () => m.ssh_invalid_name(),
  note: () => m.ssh_invalid_note(),
  password: () => m.ssh_invalid_password(),
  port: () => m.ssh_invalid_port(),
  username: () => m.ssh_invalid_username(),
};

/** What to tell the owner about a call of the plugin that failed, and what they can do. */
export function sshFailureMessage(failure: SshFailure): string {
  if (failure.kind === 'ipc') {
    return ipcFailureMessage();
  }
  const { error } = failure;
  if (error.origin === 'plugin') {
    return ERRORS[error.code]();
  }
  const field = error.code === 'invalid_input' ? error.field : null;
  const fieldMessage = field === null ? undefined : FIELDS[field];
  if (fieldMessage !== undefined) {
    return fieldMessage();
  }
  return hostRefusalMessage(error, m.ssh_plugin_name());
}

/** The field that the host's check refused, which the form marks, or `null`. */
export function refusedField(failure: SshFailure | null): string | null {
  if (failure?.kind !== 'core') {
    return null;
  }
  const { error } = failure;
  const isInvalidInput = error.origin === 'host' && error.code === 'invalid_input';
  return isInvalidInput ? error.field : null;
}

/** Whether a failure says that the host is gone, which removing it counts as done. */
export function isHostGone(failure: SshFailure): boolean {
  const isGone =
    failure.kind === 'core' && failure.error.origin === 'plugin' && failure.error.code === 'host_not_found';
  return isGone;
}

/** The words of a host's status. */
export function hostStatusLabel(status: HostStatus): string {
  switch (status) {
    case 'key_changed': {
      return m.ssh_status_key_changed();
    }
    case 'key_not_checked': {
      return m.ssh_status_key_not_checked();
    }
    case 'password_missing': {
      return m.ssh_status_password_missing();
    }
    case 'ready': {
      return m.ssh_status_ready();
    }
  }
}

/** Why the plugin did not start, by the code of its start. */
export function sshStartFailureMessage(code: string): string | null {
  return code === 'storage_failed' ? m.ssh_start_failed() : null;
}

/** The sentence of a failure code of the plugin's tools, or `null` for a code that it does not know. */
export function sshToolFailure(code: string): string | null {
  return code === 'not_started' ? m.ssh_tool_not_started() : null;
}

/** The sentence of a refusal of a mention of a host. */
export function hostMentionRefusal(code: string): string | null {
  return code === 'host_not_found' ? m.ssh_mention_host_not_found() : null;
}

/** What the model got of a mentioned host. */
export function hostMentionSummary(summary: MentionSummary): string | null {
  return summary.code === 'host' ? m.ssh_mention_summary_host() : null;
}

/** What an entry of the list is, for screen readers. */
export function hostMentionVariantName(variant: string): string | null {
  return variant === 'host' ? m.ssh_mention_host() : null;
}

/** The words of a warning on an entry of the list. */
export function hostMentionWarning(code: string): string | null {
  return code === 'not_ready' ? m.ssh_mention_not_ready() : null;
}
