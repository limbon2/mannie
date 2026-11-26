// Localized message formatting for Proxmox plugin error codes, statuses,
// mentions, work items, and tool cards (ADR-0019).
import type { WorkItem } from '@mannie/plugin-sdk';
import { hostRefusalMessage, ipcFailureMessage } from '@mannie/plugin-sdk/messages';
import type { MentionSummary } from '@mannie/plugin-sdk/slots';

import type { ProxmoxError, ServerStatus } from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import type { ProxmoxFailure } from '../types/proxmox.js';

const ERRORS: Record<ProxmoxError['code'], () => string> = {
  agent_in_control: () => m.proxmox_error_agent_in_control(),
  api_error: () => m.proxmox_error_api_error(),
  authentication_failed: () => m.proxmox_error_authentication_failed(),
  certificate_expired: () => m.proxmox_error_certificate_expired(),
  certificate_inspect_failed: () => m.proxmox_error_certificate_inspect_failed(),
  certificate_mismatch: () => m.proxmox_error_certificate_mismatch(),
  certificate_untrusted: () => m.proxmox_error_certificate_untrusted(),
  connection_failed: () => m.proxmox_error_connection_failed(),
  credentials_failed: () => m.proxmox_error_credentials_failed(),
  invalid_request: () => m.proxmox_error_invalid_request(),
  name_taken: () => m.proxmox_error_name_taken(),
  owner_in_control: () => m.proxmox_error_owner_in_control(),
  password_missing: () => m.proxmox_error_password_missing(),
  permission_denied: () => m.proxmox_error_permission_denied(),
  pin_mismatch: () => m.proxmox_error_pin_mismatch(),
  server_not_found: () => m.proxmox_error_server_not_found(),
  session_not_found: () => m.proxmox_error_session_not_found(),
  storage_failed: () => m.proxmox_error_storage_failed(),
  tfa_unsupported: () => m.proxmox_error_tfa_unsupported(),
  timeout: () => m.proxmox_error_timeout(),
  too_many_sessions: () => m.proxmox_error_too_many_sessions(),
  vm_not_allowed: () => m.proxmox_error_vm_not_allowed(),
  vm_not_found: () => m.proxmox_error_vm_not_found(),
  vnc_failed: () => m.proxmox_error_vnc_failed(),
};

const TOOL_FAILURES: Readonly<Partial<Record<string, () => string>>> = {
  ...ERRORS,
  target_changed: () => m.proxmox_error_target_changed(),
};

const FIELDS: Readonly<Partial<Record<string, () => string>>> = {
  host: () => m.proxmox_invalid_host(),
  name: () => m.proxmox_invalid_name(),
  note: () => m.proxmox_invalid_note(),
  password: () => m.proxmox_invalid_password(),
  port: () => m.proxmox_invalid_port(),
  username: () => m.proxmox_invalid_username(),
};

const CERTIFICATE_ERROR_CODES: ReadonlySet<ProxmoxError['code']> = new Set<ProxmoxError['code']>([
  'certificate_expired',
  'certificate_mismatch',
  'certificate_untrusted',
]);

/** Human-readable message for a Proxmox IPC failure. */
export function proxmoxFailureMessage(failure: ProxmoxFailure): string {
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

  return hostRefusalMessage(error, m.proxmox_plugin_name());
}

/** Whether an IPC failure indicates an untrusted, mismatched, or expired TLS certificate. */
export function isCertificateFailure(failure: ProxmoxFailure): boolean {
  if (failure.kind !== 'core') {
    return false;
  }

  const { error } = failure;
  if (error.origin !== 'plugin') {
    return false;
  }

  return CERTIFICATE_ERROR_CODES.has(error.code);
}

/** Input field refused by validation, if any. */
export function refusedField(failure: ProxmoxFailure | null): string | null {
  if (failure?.kind !== 'core') {
    return null;
  }

  const { error } = failure;
  if (error.origin !== 'host') {
    return null;
  }

  return error.code === 'invalid_input' ? error.field : null;
}

/** Whether a failure means the server is already deleted. */
export function isServerGone(failure: ProxmoxFailure): boolean {
  if (failure.kind !== 'core') {
    return false;
  }

  const { error } = failure;
  if (error.origin !== 'plugin') {
    return false;
  }

  return error.code === 'server_not_found';
}

/** Localized badge text for a server's readiness status. */
export function serverStatusLabel(status: ServerStatus): string {
  switch (status) {
    case 'no_vms_allowed': {
      return m.proxmox_status_no_vms_allowed();
    }
    case 'password_missing': {
      return m.proxmox_status_password_missing();
    }
    case 'ready': {
      return m.proxmox_status_ready();
    }
  }
}

/** Start failure message shown on the Plugins page. */
export function proxmoxStartFailureMessage(code: string): string | null {
  return code === 'storage_failed' ? m.proxmox_start_failed() : null;
}

/** Confirmation message shown when turning off the plugin with active VNC sessions. */
export function proxmoxOffQuestion(items: readonly WorkItem[]): string | null {
  const vnc = items.find((item) => item.code === 'vnc_sessions');
  if (vnc === undefined) {
    return null;
  }

  if (vnc.count <= 0) {
    return null;
  }

  return m.proxmox_off_question({ count: vnc.count });
}

/** Tool card failure message for a Proxmox tool failure code. */
export function proxmoxToolFailure(code: string): string | null {
  const lookup = TOOL_FAILURES[code];
  return lookup === undefined ? null : lookup();
}

/** Mention refusal message. */
export function proxmoxMentionRefusal(code: string): string | null {
  if (code === 'server_not_found') {
    return m.proxmox_mention_server_not_found();
  }
  if (code === 'vm_not_allowed') {
    return m.proxmox_mention_vm_not_allowed();
  }
  return null;
}

/** Mention summary text for sent mentions. */
export function proxmoxMentionSummary(summary: MentionSummary): string | null {
  if (summary.code === 'server') {
    return m.proxmox_mention_summary_server();
  }
  if (summary.code === 'vm') {
    const vmid = summary.values?.vmid;
    return m.proxmox_mention_summary_vm({ vmid: vmid === undefined ? '' : String(vmid) });
  }
  return null;
}

/** Screen-reader variant label for Proxmox mentions. */
export function proxmoxMentionVariantName(variant: string): string | null {
  if (variant === 'server') {
    return m.proxmox_mention_server();
  }
  if (variant === 'vm') {
    return m.proxmox_mention_vm();
  }
  return null;
}

/** Warning badge text for mentions of servers not yet ready. */
export function proxmoxMentionWarning(code: string): string | null {
  return code === 'not_ready' ? m.proxmox_mention_not_ready() : null;
}
