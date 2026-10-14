// Messages for the codes of the chat feature (ADR-0019): run failures, the
// chat commands' errors, provider profile errors and the provider test. The UI
// shows the catalog's message for a code and never the core's description.
import type { CheckKind, CheckNote, CredentialStoreProblem, RunFailure, SkipReason } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import type { ChatFailure, ProfileFailure } from '#lib/types/chat.js';

const counts = new Intl.NumberFormat();

/** A count, formatted for the owner's locale. */
export function formatCount(value: number): string {
  return counts.format(value);
}

/** A time of day, formatted for the owner's locale. */
export function formatTime(millis: number): string {
  return new Intl.DateTimeFormat(undefined, { timeStyle: 'short' }).format(millis);
}

/** What a failed reply tells the owner: what failed and what they can do. */
export function runFailureMessage(failure: RunFailure): string {
  switch (failure.code) {
    case 'bad_request': {
      return m.chat_failure_bad_request({ status: String(failure.status) });
    }
    case 'broken_stream': {
      return m.chat_failure_broken_stream();
    }
    case 'connection_failed': {
      return m.chat_failure_connection_failed();
    }
    case 'content_filtered': {
      return m.chat_failure_content_filtered();
    }
    case 'context_overflow': {
      return m.chat_failure_context_overflow();
    }
    case 'credential_missing': {
      return m.chat_failure_credential_missing();
    }
    case 'credential_store_unavailable': {
      return m.chat_failure_credential_store_unavailable();
    }
    case 'endpoint_not_found': {
      return m.chat_failure_endpoint_not_found();
    }
    case 'internal': {
      return m.chat_failure_internal();
    }
    case 'invalid_credential': {
      return m.chat_failure_invalid_credential();
    }
    case 'invalid_response': {
      return m.chat_failure_invalid_response();
    }
    case 'model_not_found': {
      return m.chat_failure_model_not_found({ model: failure.model });
    }
    case 'no_profile': {
      return m.chat_failure_no_profile();
    }
    case 'permission_denied': {
      return m.chat_failure_permission_denied();
    }
    case 'profile_missing': {
      return m.chat_failure_profile_missing();
    }
    case 'quota_exceeded': {
      return m.chat_failure_quota_exceeded();
    }
    case 'rate_limited': {
      return failure.retryAt === null
        ? m.chat_failure_rate_limited()
        : m.chat_failure_rate_limited_until({ time: formatTime(failure.retryAt) });
    }
    case 'redirect_refused': {
      return m.chat_failure_redirect_refused();
    }
    case 'request_too_large': {
      return m.chat_failure_request_too_large();
    }
    case 'server_error': {
      return m.chat_failure_server_error({ status: String(failure.status) });
    }
    case 'timed_out': {
      return m.chat_failure_timed_out();
    }
  }
}

/** What to tell the owner about a chat command that failed. */
export function chatFailureMessage(failure: ChatFailure): string {
  if (failure.kind === 'ipc') {
    return m.shell_error_ipc_failed();
  }
  const { error } = failure;
  switch (error.code) {
    case 'checkpoint_not_found': {
      return m.chat_error_checkpoint_not_found();
    }
    case 'compacting': {
      return m.chat_error_compacting();
    }
    case 'compaction_failed': {
      return m.chat_error_compaction_failed({ cause: runFailureMessage(error.failure) });
    }
    case 'conversation_not_found': {
      return m.chat_error_conversation_not_found();
    }
    case 'instructions_too_long': {
      return m.chat_error_instructions_too_long({ maxBytes: formatCount(error.maxBytes) });
    }
    case 'message_empty': {
      return m.chat_error_message_empty();
    }
    case 'message_too_long': {
      return m.chat_error_message_too_long({ maxBytes: formatCount(error.maxBytes) });
    }
    case 'no_profile': {
      return m.chat_failure_no_profile();
    }
    case 'nothing_to_compact': {
      return m.chat_error_nothing_to_compact();
    }
    case 'nothing_to_retry': {
      return m.chat_error_nothing_to_retry();
    }
    case 'profile_not_found': {
      return m.chat_error_profile_not_found();
    }
    case 'run_active': {
      return m.chat_error_run_active();
    }
    case 'shutting_down': {
      return m.chat_error_shutting_down();
    }
    case 'storage_failed': {
      return m.chat_error_storage_failed();
    }
    case 'store_unavailable': {
      return m.shell_error_store_unavailable();
    }
    case 'title_empty': {
      return m.chat_error_title_empty();
    }
    case 'title_too_long': {
      return m.chat_error_title_too_long({ maxCharacters: formatCount(error.maxCharacters) });
    }
    case 'written_by_newer_version': {
      return m.chat_error_written_by_newer_version();
    }
  }
}

/** What to tell the owner about the credential store. */
export function credentialStoreMessage(problem: CredentialStoreProblem): string {
  switch (problem) {
    case 'access_denied': {
      return m.chat_providers_store_access_denied();
    }
    case 'no_secret_service': {
      return m.chat_providers_store_no_secret_service();
    }
    case 'other': {
      return m.chat_providers_store_other();
    }
    case 'unsupported_platform': {
      return m.chat_providers_store_unsupported_platform();
    }
  }
}

/** What to tell the owner about a provider profile command that failed. */
export function profileFailureMessage(failure: ProfileFailure): string {
  if (failure.kind === 'ipc') {
    return m.shell_error_ipc_failed();
  }
  const { error } = failure;
  switch (error.code) {
    case 'credential_store_unavailable': {
      return credentialStoreMessage(error.reason);
    }
    case 'insecure_credential_transport': {
      return m.chat_providers_error_insecure_credential_transport();
    }
    case 'invalid_base_url': {
      return m.chat_providers_error_invalid_base_url();
    }
    case 'invalid_credential': {
      return m.chat_providers_error_invalid_credential();
    }
    case 'invalid_limits': {
      return m.chat_providers_error_invalid_limits({
        maxContextWindow: formatCount(error.maxContextWindow),
        minBudget: formatCount(error.minBudget),
        minContextWindow: formatCount(error.minContextWindow),
      });
    }
    case 'invalid_model': {
      return m.chat_providers_error_invalid_model();
    }
    case 'invalid_name': {
      return m.chat_providers_error_invalid_name();
    }
    case 'name_taken': {
      return m.chat_providers_error_name_taken();
    }
    case 'not_found': {
      return m.chat_providers_error_not_found();
    }
    case 'profile_in_use': {
      return m.chat_providers_error_profile_in_use();
    }
    case 'storage_failed': {
      return m.chat_providers_error_storage_failed();
    }
    case 'store_unavailable': {
      return m.shell_error_store_unavailable();
    }
  }
}

/** The label of a check of the provider test. */
export function checkLabel(check: CheckKind): string {
  switch (check) {
    case 'credential': {
      return m.chat_providers_check_credential();
    }
    case 'model': {
      return m.chat_providers_check_model();
    }
    case 'reasoning': {
      return m.chat_providers_check_reasoning();
    }
    case 'reply': {
      return m.chat_providers_check_reply();
    }
    case 'server': {
      return m.chat_providers_check_server();
    }
    case 'usage': {
      return m.chat_providers_check_usage();
    }
  }
}

/** What a warning of the provider test means. */
export function checkNoteMessage(note: CheckNote): string {
  switch (note) {
    case 'model_list_unavailable': {
      return m.chat_providers_note_model_list_unavailable();
    }
    case 'model_not_listed': {
      return m.chat_providers_note_model_not_listed();
    }
    case 'no_reasoning_returned': {
      return m.chat_providers_note_no_reasoning_returned();
    }
    case 'no_usage_reported': {
      return m.chat_providers_note_no_usage_reported();
    }
  }
}

/** Why a check of the provider test was skipped. */
export function skipReasonMessage(reason: SkipReason): string {
  switch (reason) {
    case 'earlier_check_failed': {
      return m.chat_providers_skip_earlier_check_failed();
    }
    case 'no_credential': {
      return m.chat_providers_skip_no_credential();
    }
    case 'no_model_list': {
      return m.chat_providers_skip_no_model_list();
    }
    case 'no_reasoning_effort': {
      return m.chat_providers_skip_no_reasoning_effort();
    }
  }
}
