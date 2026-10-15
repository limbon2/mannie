// Every code of the chat feature has a message of its own (ADR-0019).
import { describe, expect, it } from 'vitest';

import type {
  ChatError,
  CheckKind,
  CheckNote,
  CredentialStoreProblem,
  PathRefusal,
  ProviderProfileError,
  RunFailure,
  SkipReason,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  chatFailureMessage,
  checkLabel,
  checkNoteMessage,
  credentialStoreMessage,
  folderRefusalMessage,
  profileFailureMessage,
  runFailureMessage,
  skipReasonMessage,
} from '../../../../../../apps/desktop/src/lib/utils/chat-errors.js';

const RUN_FAILURES: RunFailure[] = [
  { code: 'invalid_credential' },
  { code: 'permission_denied' },
  { code: 'quota_exceeded' },
  { code: 'model_not_found', model: 'gpt-x' },
  { code: 'endpoint_not_found' },
  { code: 'rate_limited', retryAt: null },
  { code: 'rate_limited', retryAt: 1_791_892_800_000 },
  { code: 'server_error', status: 503 },
  { code: 'context_overflow' },
  { code: 'request_too_large' },
  { code: 'bad_request', status: 400 },
  { code: 'content_filtered' },
  { code: 'connection_failed' },
  { code: 'timed_out' },
  { code: 'broken_stream' },
  { code: 'invalid_response' },
  { code: 'redirect_refused' },
  { code: 'credential_missing' },
  { code: 'credential_store_unavailable' },
  { code: 'profile_missing' },
  { code: 'no_profile' },
  { code: 'tools_unsupported' },
  { code: 'internal' },
];

const CHAT_ERRORS: ChatError[] = [
  { code: 'message_empty' },
  { code: 'message_too_long', maxBytes: 262_144 },
  { code: 'title_empty' },
  { code: 'title_too_long', maxCharacters: 200 },
  { code: 'instructions_too_long', maxBytes: 32_768 },
  { code: 'conversation_not_found' },
  { code: 'checkpoint_not_found' },
  { code: 'run_active' },
  { code: 'nothing_to_retry' },
  { code: 'nothing_to_compact' },
  { code: 'compacting' },
  { code: 'compaction_failed', failure: { code: 'timed_out' } },
  { code: 'profile_not_found' },
  { code: 'project_not_found' },
  { code: 'project_name_invalid', maxCharacters: 100 },
  { code: 'project_busy' },
  { code: 'project_changed', conversations: 3 },
  { code: 'path_refused', reason: 'network_path' },
  { code: 'folder_not_found' },
  { code: 'not_a_folder' },
  { code: 'folder_unreadable', kind: 'PermissionDenied' },
  { code: 'folder_dialog_failed' },
  { code: 'written_by_newer_version' },
  { code: 'storage_failed' },
  { code: 'shutting_down' },
];

const PROFILE_ERRORS: ProviderProfileError[] = [
  { code: 'invalid_name' },
  { code: 'name_taken' },
  { code: 'invalid_base_url' },
  { code: 'insecure_credential_transport' },
  { code: 'invalid_model' },
  { code: 'invalid_limits', maxContextWindow: 10_000_000, minBudget: 1_024, minContextWindow: 1_024 },
  { code: 'invalid_credential' },
  { code: 'not_found' },
  { code: 'profile_in_use' },
  { code: 'credential_store_unavailable', reason: 'access_denied' },
  { code: 'storage_failed' },
];

function expectDistinct(messages: string[]): void {
  for (const message of messages) {
    expect(message.trim()).not.toBe('');
  }
  expect(new Set(messages).size).toBe(messages.length);
}

describe('chat messages', () => {
  it('say what each run failure means', () => {
    const messages = RUN_FAILURES.map((failure) => runFailureMessage(failure));

    expectDistinct(messages);
    expect(runFailureMessage({ code: 'model_not_found', model: 'gpt-x' })).toContain('gpt-x');
    expect(runFailureMessage({ code: 'server_error', status: 503 })).toContain('503');
  });

  it('say what each chat command error means', () => {
    const messages = CHAT_ERRORS.map((error) => chatFailureMessage({ error, kind: 'core' }));

    expectDistinct(messages);
    expect(chatFailureMessage({ kind: 'ipc' })).not.toBe('');
  });

  it('say why Mannie refused a project’s folder', () => {
    const reasons: PathRefusal[] = [
      'empty',
      'network_path',
      'device_path',
      'alternate_stream',
      'reserved_name',
      'trailing_dot_or_space',
      'drive_relative',
      'too_long',
      'not_absolute',
    ];

    expectDistinct(reasons.map((reason) => folderRefusalMessage(reason)));
    expect(chatFailureMessage({ error: { code: 'path_refused', reason: 'network_path' }, kind: 'core' })).toContain(
      folderRefusalMessage('network_path'),
    );
  });

  it('say what each provider profile error and credential store problem means', () => {
    const problems: CredentialStoreProblem[] = ['no_secret_service', 'unsupported_platform', 'access_denied', 'other'];

    expectDistinct(PROFILE_ERRORS.map((error) => profileFailureMessage({ error, kind: 'core' })));
    expectDistinct(problems.map((problem) => credentialStoreMessage(problem)));
  });

  it('label every check, note and skip reason of the provider test', () => {
    const checks: CheckKind[] = ['credential', 'server', 'model', 'reply', 'usage', 'reasoning', 'tools'];
    const notes: CheckNote[] = [
      'model_not_listed',
      'model_list_unavailable',
      'no_usage_reported',
      'no_reasoning_returned',
      'no_tool_call',
    ];
    const reasons: SkipReason[] = [
      'no_credential',
      'earlier_check_failed',
      'no_reasoning_effort',
      'no_model_list',
      'tools_off',
    ];

    expectDistinct(checks.map((check) => checkLabel(check)));
    expectDistinct(notes.map((note) => checkNoteMessage(note)));
    expectDistinct(reasons.map((reason) => skipReasonMessage(reason)));
  });
});
