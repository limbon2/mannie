// Messages for the Network Client plugin's codes (ADR-0019; ADR-0075).
import type { WorkItem } from '@mannie/plugin-sdk';
import { hostRefusalMessage, ipcFailureMessage } from '@mannie/plugin-sdk/messages';
import type { MentionSummary } from '@mannie/plugin-sdk/slots';

import type { AuthConfig, BodyConfig, NetworkError, OpenApiWarning } from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import type { NetworkFailure } from '../types/network.js';

const START_STORAGE_FAILED = 'storage_failed';
const START_CREDENTIAL_STORE_FAILED = 'credential_store_failed';
const START_HTTP_FAILED = 'http_client_failed';
const ACTIVE_REQUESTS_WORK = 'active_requests';

/** Explains a start failure code on the Plugins page. */
export function networkStartFailure(code: string): string | null {
  switch (code) {
    case START_STORAGE_FAILED: {
      return m.network_start_failed_storage();
    }
    case START_CREDENTIAL_STORE_FAILED: {
      return m.network_start_failed_credentials();
    }
    case START_HTTP_FAILED: {
      return m.network_start_failed_http();
    }
    default: {
      return null;
    }
  }
}

/** Explains what turning the plugin off does to running manual requests. */
export function networkOffQuestion(items: readonly WorkItem[]): string | null {
  const active = items.find((item) => item.code === ACTIVE_REQUESTS_WORK);
  if (active === undefined) {
    return null;
  }
  if (active.count <= 0) {
    return null;
  }
  return m.network_off_question({ count: active.count });
}

/** Translates a typed `NetworkError` into an owner-facing sentence. */
export function networkErrorMessage(error: NetworkError): string {
  switch (error.code) {
    case 'collection_not_found': {
      return m.network_error_collection_not_found();
    }
    case 'cookie_not_found': {
      return m.network_error_cookie_not_found();
    }
    case 'credential_store_unavailable': {
      return m.network_error_credential_store_unavailable();
    }
    case 'environment_name_taken': {
      return m.network_error_environment_name_taken();
    }
    case 'environment_not_found': {
      return m.network_error_environment_not_found();
    }
    case 'file_dialog_failed': {
      return m.network_error_file_dialog_failed();
    }
    case 'file_unreadable': {
      return m.network_error_file_unreadable();
    }
    case 'folder_not_found': {
      return m.network_error_folder_not_found();
    }
    case 'insecure_credential_transport': {
      return m.network_error_insecure_credential_transport();
    }
    case 'invalid_spec': {
      return m.network_error_invalid_spec({ detail: error.detail });
    }
    case 'invalid_url': {
      return m.network_error_invalid_url();
    }
    case 'name_taken': {
      return m.network_error_name_taken();
    }
    case 'preview_not_found': {
      return m.network_error_preview_not_found();
    }
    case 'request_canceled': {
      return m.network_error_request_canceled();
    }
    case 'request_failed': {
      return m.network_error_request_failed({ detail: error.detail });
    }
    case 'request_not_found': {
      return m.network_error_request_not_found();
    }
    case 'request_timed_out': {
      return m.network_error_request_timed_out();
    }
    case 'response_lost': {
      return m.network_error_response_lost({ detail: error.detail });
    }
    case 'spec_too_complex': {
      return m.network_error_spec_too_complex();
    }
    case 'spec_too_large': {
      return m.network_error_spec_too_large();
    }
    case 'storage_failed': {
      return m.network_error_storage_failed();
    }
    case 'unsupported_spec_version': {
      return m.network_error_unsupported_spec_version({ version: error.version });
    }
  }
}

/** Translates any plugin call failure (IPC, host refusal, or plugin error). */
export function networkFailureMessage(failure: NetworkFailure): string {
  if (failure.kind === 'ipc') {
    return ipcFailureMessage();
  }
  const { error } = failure;
  if (error.origin === 'host') {
    return hostRefusalMessage(error, m.network_plugin_name());
  }
  return networkErrorMessage(error);
}

/** Translates a failure code from one of the plugin's agent tools. */
export function networkToolFailure(code: string): string | null {
  switch (code) {
    case 'ambiguous_request': {
      return m.network_tool_failure_ambiguous_request();
    }
    case 'environment_not_found': {
      return m.network_tool_failure_environment_not_found();
    }
    case 'insecure_transport': {
      return m.network_tool_failure_insecure_transport();
    }
    case 'invalid_arguments': {
      return m.network_tool_failure_invalid_arguments();
    }
    case 'invalid_url': {
      return m.network_tool_failure_invalid_url();
    }
    case 'origin_override_refused': {
      return m.network_tool_failure_origin_override_refused();
    }
    case 'request_changed': {
      return m.network_tool_failure_request_changed();
    }
    case 'request_failed': {
      return m.network_tool_failure_request_failed();
    }
    case 'request_not_found': {
      return m.network_tool_failure_request_not_found();
    }
    case 'response_lost': {
      return m.network_tool_failure_response_lost();
    }
    case 'secret_in_arguments': {
      return m.network_tool_failure_secret_in_arguments();
    }
    case 'timed_out': {
      return m.network_tool_failure_timed_out();
    }
    default: {
      return null;
    }
  }
}

/** Translates a refusal code from the `network` mention provider. */
export function networkMentionRefusal(code: string): string | null {
  return code === 'item_not_found' ? m.network_mention_refusal_not_found() : null;
}

/** Labels a resolved mention summary chip. */
export function networkMentionSummary(summary: MentionSummary): string | null {
  switch (summary.code) {
    case 'collection': {
      return m.network_mention_collection();
    }
    case 'request': {
      return m.network_mention_request();
    }
    default: {
      return null;
    }
  }
}

/** Screen-reader variant label for a mention entry. */
export function networkMentionVariantName(variant: string): string | null {
  switch (variant) {
    case 'collection': {
      return m.network_mention_collection();
    }
    case 'request': {
      return m.network_mention_request();
    }
    default: {
      return null;
    }
  }
}

/** Display label for an authentication kind. */
export function authKindLabel(kind: AuthConfig['kind']): string {
  switch (kind) {
    case 'api_key': {
      return m.network_auth_api_key();
    }
    case 'basic': {
      return m.network_auth_basic();
    }
    case 'bearer': {
      return m.network_auth_bearer();
    }
    case 'inherit': {
      return m.network_auth_inherit();
    }
    case 'none': {
      return m.network_auth_none();
    }
  }
}

/** Display label for a request body kind. */
export function bodyKindLabel(kind: BodyConfig['kind']): string {
  switch (kind) {
    case 'form_url_encoded': {
      return m.network_body_form();
    }
    case 'json': {
      return m.network_body_json();
    }
    case 'none': {
      return m.network_body_none();
    }
    case 'text': {
      return m.network_body_text();
    }
  }
}

/** Translates a structured OpenAPI preview warning into a localized note. */
export function formatImportWarning(warning: OpenApiWarning): string {
  switch (warning.kind) {
    case 'missing_base_url': {
      return warning.source === 'v2_host'
        ? m.network_import_warning_missing_base_url_v2()
        : m.network_import_warning_missing_base_url_v3();
    }
    case 'cookie_parameter': {
      return m.network_import_warning_cookie_parameter({
        cookie: warning.cookieName,
        endpoint: warning.endpoint,
      });
    }
    case 'cookie_security_scheme': {
      return m.network_import_warning_cookie_security_scheme({
        cookie: warning.cookieName,
        scheme: warning.scheme,
      });
    }
    case 'skipped_external_ref': {
      return m.network_import_warning_skipped_external_ref({ reference: warning.reference });
    }
    case 'cyclic_ref': {
      return m.network_import_warning_cyclic_ref({ reference: warning.reference });
    }
    case 'unresolved_local_ref': {
      return m.network_import_warning_unresolved_local_ref({ reference: warning.reference });
    }
    case 'deep_ref_chain': {
      return m.network_import_warning_deep_ref_chain();
    }
    case 'endpoint_limit_reached': {
      return m.network_import_warning_endpoint_limit({ max: String(warning.maxEndpoints) });
    }
    case 'combined_security_schemes': {
      return m.network_import_warning_combined_security({ context: warning.context });
    }
    case 'unsupported_operation_security': {
      return m.network_import_warning_unsupported_security({ operation: warning.operation });
    }
  }
}
