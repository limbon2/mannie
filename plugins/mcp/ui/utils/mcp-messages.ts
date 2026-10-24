// Messages for the MCP plugin's codes (ADR-0019): the core words a call that
// got no answer and the host's refusals, and the plugin its own errors, the
// statuses and failures of servers, their warnings, the events of their
// logs, the hints and problems of their tools, the refusals of a pasted
// configuration, its mentions and the failure codes of its tools.
import type { WorkItem } from '@mannie/plugin-sdk';
import { hostRefusalMessage, ipcFailureMessage } from '@mannie/plugin-sdk/messages';
import type { MentionSummary } from '@mannie/plugin-sdk/slots';

import type {
  EntryRefusalCode,
  LogEvent,
  McpError,
  ServerFailure,
  ServerStatus,
  ServerWarning,
  ToolHint,
  ToolProblem,
} from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import type { McpFailure } from '../types/mcp.js';

/** The code of the work of the calls in progress, as the plugin reports it. */
const ACTIVE_CALLS = 'active_calls';

/** The plugin's own errors by their codes, which the type checks hold to the plugin's error type. */
const ERRORS: Record<McpError['code'], (error: McpError) => string> = {
  authorization_failed: () => m.mcp_error_authorization_failed(),
  authorization_unsupported: () => m.mcp_error_authorization_unsupported(),
  client_id_needed: () => m.mcp_error_client_id_needed(),
  configuration_empty: () => m.mcp_error_configuration_empty(),
  configuration_not_json: () => m.mcp_error_configuration_not_json(),
  configuration_too_large: () => m.mcp_error_configuration_too_large(),
  configuration_too_many_servers: () => m.mcp_error_configuration_too_many_servers(),
  credential_store_unavailable: () => m.mcp_error_credential_store_unavailable(),
  name_taken: () => m.mcp_error_name_taken(),
  no_sign_in: () => m.mcp_error_no_sign_in(),
  prefix_taken: () => m.mcp_error_prefix_taken(),
  program_not_found: () => m.mcp_error_program_not_found(),
  program_refused: () => m.mcp_error_program_refused(),
  server_not_found: () => m.mcp_error_server_not_found(),
  storage_failed: () => m.mcp_error_storage_failed(),
  too_many_tools: (error) => m.mcp_error_too_many_tools({ limit: error.code === 'too_many_tools' ? error.limit : 0 }),
  tool_not_found: () => m.mcp_error_tool_not_found(),
  transport_changed: () => m.mcp_error_transport_changed(),
};

/** What a field that the host's check refused needs, by the field's name in the input. */
const FIELDS: Readonly<Record<string, () => string>> = {
  arguments: () => m.mcp_invalid_arguments(),
  clientId: () => m.mcp_invalid_client_id(),
  clientSecret: () => m.mcp_invalid_client_secret(),
  folder: () => m.mcp_invalid_folder(),
  name: () => m.mcp_invalid_name(),
  oauth: () => m.mcp_invalid_oauth(),
  plain: () => m.mcp_invalid_plain(),
  prefix: () => m.mcp_invalid_prefix(),
  program: () => m.mcp_invalid_program(),
  secrets: () => m.mcp_invalid_secrets(),
  timeoutSeconds: () => m.mcp_invalid_timeout(),
  url: () => m.mcp_invalid_url(),
};

/** What to tell the owner about a call of the plugin that failed, and what they can do. */
export function mcpFailureMessage(failure: McpFailure): string {
  if (failure.kind === 'ipc') {
    return ipcFailureMessage();
  }
  const { error } = failure;
  if (error.origin === 'plugin') {
    return ERRORS[error.code](error);
  }
  const field = error.code === 'invalid_input' ? error.field : null;
  const fieldMessage = field === null ? undefined : FIELDS[field];
  if (fieldMessage !== undefined) {
    return fieldMessage();
  }
  return hostRefusalMessage(error, m.mcp_plugin_name());
}

/** The field that the host's check refused, which the form marks, or `null`. */
export function refusedField(failure: McpFailure | null): string | null {
  if (failure?.kind !== 'core') {
    return null;
  }
  const { error } = failure;
  const isInvalidInput = error.origin === 'host' && error.code === 'invalid_input';
  return isInvalidInput ? error.field : null;
}

/** Whether a failure says that the server is gone, which removing it counts as done. */
export function isServerGone(failure: McpFailure): boolean {
  const isGone =
    failure.kind === 'core' && failure.error.origin === 'plugin' && failure.error.code === 'server_not_found';
  return isGone;
}

/** The words of a server's status. */
export function serverStatusLabel(status: ServerStatus): string {
  switch (status) {
    case 'access_needed': {
      return m.mcp_status_access_needed();
    }
    case 'connected': {
      return m.mcp_status_connected();
    }
    case 'failed': {
      return m.mcp_status_failed();
    }
    case 'off': {
      return m.mcp_status_off();
    }
    case 'sign_in_needed': {
      return m.mcp_status_sign_in_needed();
    }
    case 'starting': {
      return m.mcp_status_starting();
    }
  }
}

/** Why a server failed, and what the owner can do. */
export function serverFailureMessage(failure: ServerFailure): string {
  switch (failure.code) {
    case 'argument_refused': {
      return m.mcp_failure_argument_refused();
    }
    case 'authorization_unsupported': {
      return m.mcp_failure_authorization_unsupported();
    }
    case 'crashed': {
      return m.mcp_failure_crashed({ restarts: failure.restarts });
    }
    case 'credential_store_unavailable': {
      return m.mcp_failure_credential_store_unavailable();
    }
    case 'exited': {
      return failure.exitCode === null
        ? m.mcp_failure_exited_without_code()
        : m.mcp_failure_exited({ code: failure.exitCode });
    }
    case 'http_status': {
      return m.mcp_failure_http_status({ status: failure.status });
    }
    case 'program_not_found': {
      return m.mcp_failure_program_not_found();
    }
    case 'program_refused': {
      return m.mcp_failure_program_refused();
    }
    case 'protocol': {
      return m.mcp_failure_protocol();
    }
    case 'secret_missing': {
      return m.mcp_failure_secret_missing();
    }
    case 'sign_in_failed': {
      return m.mcp_failure_sign_in_failed();
    }
    case 'spawn_failed': {
      return m.mcp_failure_spawn_failed();
    }
    case 'sse_only': {
      return m.mcp_failure_sse_only();
    }
    case 'timed_out': {
      return m.mcp_failure_timed_out({ seconds: failure.seconds });
    }
    case 'too_many_servers': {
      return m.mcp_failure_too_many_servers({ limit: failure.limit });
    }
    case 'unreachable': {
      return m.mcp_failure_unreachable();
    }
    case 'variable_refused': {
      return m.mcp_failure_variable_refused({ name: failure.name });
    }
    case 'working_directory_invalid': {
      return m.mcp_failure_working_directory_invalid();
    }
  }
}

/** The words of a warning about a server. */
export function serverWarningMessage(warning: ServerWarning): string {
  switch (warning.code) {
    case 'new_tools': {
      return m.mcp_warning_new_tools({ count: warning.count });
    }
    case 'token_in_argument': {
      return m.mcp_warning_token_in_argument({ number: warning.index + 1 });
    }
    case 'unusable_tools': {
      return m.mcp_warning_unusable_tools({ count: warning.count });
    }
  }
}

/** The words of an event of a server's log. */
export function logEventMessage(event: LogEvent): string {
  switch (event.kind) {
    case 'connected': {
      return m.mcp_event_connected({ version: event.version });
    }
    case 'connecting': {
      return m.mcp_event_connecting();
    }
    case 'disconnected': {
      return m.mcp_event_disconnected();
    }
    case 'exited': {
      return event.exitCode === null ? m.mcp_event_exited_without_code() : m.mcp_event_exited({ code: event.exitCode });
    }
    case 'failed': {
      return m.mcp_event_failed({ reason: serverFailureMessage(event.failure) });
    }
    case 'listed': {
      return m.mcp_event_listed({ tools: event.tools });
    }
    case 'restarting': {
      return m.mcp_event_restarting({ seconds: event.seconds });
    }
    case 'signed_in': {
      return m.mcp_event_signed_in();
    }
    case 'signed_out': {
      return m.mcp_event_signed_out();
    }
    case 'stopped': {
      return m.mcp_event_stopped();
    }
  }
}

/** The words of what a server says of a tool. */
export function toolHintLabel(hint: ToolHint): string {
  switch (hint) {
    case 'destructive': {
      return m.mcp_hint_destructive();
    }
    case 'idempotent': {
      return m.mcp_hint_idempotent();
    }
    case 'open_world': {
      return m.mcp_hint_open_world();
    }
    case 'read_only': {
      return m.mcp_hint_read_only();
    }
  }
}

/** Why a tool cannot be offered to agents. */
export function toolProblemMessage(problem: ToolProblem): string {
  switch (problem) {
    case 'activity_clash': {
      return m.mcp_problem_activity_clash();
    }
    case 'schema_invalid': {
      return m.mcp_problem_schema_invalid();
    }
    case 'too_long': {
      return m.mcp_problem_too_long();
    }
  }
}

/** Why a server of a pasted configuration cannot be added. */
export function entryRefusalMessage(refusal: EntryRefusalCode): string {
  switch (refusal) {
    case 'invalid_name': {
      return m.mcp_refusal_invalid_name();
    }
    case 'invalid_program': {
      return m.mcp_refusal_invalid_program();
    }
    case 'invalid_url': {
      return m.mcp_refusal_invalid_url();
    }
    case 'invalid_value': {
      return m.mcp_refusal_invalid_value();
    }
    case 'name_taken': {
      return m.mcp_refusal_name_taken();
    }
    case 'no_endpoint': {
      return m.mcp_refusal_no_endpoint();
    }
    case 'sse_only': {
      return m.mcp_refusal_sse_only();
    }
  }
}

/** Why the plugin did not start, by the code of its start. */
export function mcpStartFailureMessage(code: string): string | null {
  switch (code) {
    case 'http_client_failed': {
      return m.mcp_start_failed_http();
    }
    case 'storage_failed': {
      return m.mcp_start_failed_storage();
    }
    default: {
      return null;
    }
  }
}

/** What turning the plugin off does to its calls in progress, or `null` without any. */
export function mcpOffQuestion(items: readonly WorkItem[]): string | null {
  const count = items.find((item) => item.code === ACTIVE_CALLS)?.count ?? 0;
  return count === 0 ? null : m.mcp_off_question({ count });
}

/** The sentence of a failure code of the plugin's tools, or `null` for a code that it does not know. */
export function mcpToolFailure(code: string): string | null {
  switch (code) {
    case 'input_required': {
      return m.mcp_tool_failure_input_required();
    }
    case 'not_called': {
      return m.mcp_tool_failure_not_called();
    }
    case 'server_error': {
      return m.mcp_tool_failure_server_error();
    }
    case 'server_stopped': {
      return m.mcp_tool_failure_server_stopped();
    }
    case 'task_started': {
      return m.mcp_tool_failure_task_started();
    }
    case 'timed_out': {
      return m.mcp_tool_failure_timed_out();
    }
    case 'tool_error': {
      return m.mcp_tool_failure_tool_error();
    }
    default: {
      return null;
    }
  }
}

/** The sentence of a refusal of a mention of a resource. */
export function resourceMentionRefusal(code: string): string | null {
  switch (code) {
    case 'read_failed': {
      return m.mcp_mention_read_failed();
    }
    case 'resource_not_found': {
      return m.mcp_mention_resource_not_found();
    }
    default: {
      return null;
    }
  }
}

/** What the model got of a mentioned resource. */
export function resourceMentionSummary(summary: MentionSummary): string | null {
  return summary.code === 'resource' ? m.mcp_mention_summary_resource() : null;
}

/** What an entry of the list is, for screen readers. */
export function resourceMentionVariantName(variant: string): string | null {
  return variant === 'resource' ? m.mcp_mention_resource() : null;
}
