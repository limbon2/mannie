// Tests error, tool failure, mention and label mappers of the Network Client UI.
import { describe, expect, it } from 'vitest';

import type { NetworkError, OpenApiWarning } from '../../../ui/generated/bindings.js';
import {
  authKindLabel,
  bodyKindLabel,
  formatImportWarning,
  networkErrorMessage,
  networkFailureMessage,
  networkToolFailure,
} from '../../../ui/utils/network-messages.js';
import { readSendCardDetails } from '../../../ui/utils/network-tools.js';

describe('network message mappers', () => {
  it('words every typed NetworkError variant', () => {
    const errors: readonly NetworkError[] = [
      { code: 'name_taken' },
      { code: 'environment_name_taken' },
      { code: 'collection_not_found' },
      { code: 'folder_not_found' },
      { code: 'request_not_found' },
      { code: 'environment_not_found' },
      { code: 'cookie_not_found' },
      { code: 'preview_not_found' },
      { code: 'invalid_url' },
      { code: 'insecure_credential_transport' },
      { code: 'invalid_spec', detail: 'bad yaml' },
      { code: 'spec_too_large' },
      { code: 'spec_too_complex' },
      { code: 'unsupported_spec_version', version: '1.2' },
      { code: 'file_dialog_failed' },
      { code: 'file_unreadable' },
      { code: 'request_timed_out' },
      { code: 'request_canceled' },
      { code: 'response_lost', detail: 'connection reset' },
      { code: 'request_failed', detail: 'connection refused' },
      { code: 'credential_store_unavailable' },
      { code: 'storage_failed' },
    ];

    for (const error of errors) {
      const text = networkErrorMessage(error);
      expect(text.length).toBeGreaterThan(0);
    }

    expect(networkFailureMessage({ kind: 'ipc' }).length).toBeGreaterThan(0);
  });

  it('words tool failure codes and auth/body kinds', () => {
    for (const code of [
      'request_not_found',
      'ambiguous_request',
      'environment_not_found',
      'secret_in_arguments',
      'origin_override_refused',
      'invalid_url',
      'request_changed',
      'insecure_transport',
      'timed_out',
      'response_lost',
      'request_failed',
      'invalid_arguments',
    ]) {
      expect(networkToolFailure(code)).not.toBeNull();
    }
    expect(networkToolFailure('unknown')).toBeNull();

    expect(authKindLabel('bearer')).toBe('Bearer token');
    expect(bodyKindLabel('form_url_encoded')).toBe('Form URL-encoded');
  });

  it('words every structured OpenApiWarning variant', () => {
    const warnings: readonly OpenApiWarning[] = [
      { kind: 'missing_base_url', source: 'v2_host' },
      { kind: 'missing_base_url', source: 'v3_servers' },
      { cookieName: 'sid', endpoint: 'GET /me', kind: 'cookie_parameter' },
      { cookieName: 'auth', kind: 'cookie_security_scheme', scheme: 'cookieAuth' },
      { kind: 'skipped_external_ref', reference: 'https://evil.example/a.yaml' },
      { kind: 'cyclic_ref', reference: '#/components/schemas/Node' },
      { kind: 'unresolved_local_ref', reference: '#/components/schemas/Missing' },
      { kind: 'deep_ref_chain' },
      { kind: 'endpoint_limit_reached', maxEndpoints: 500 },
      { context: 'Document security', kind: 'combined_security_schemes' },
      { kind: 'unsupported_operation_security', operation: 'GET /admin' },
    ];

    for (const warning of warnings) {
      expect(formatImportWarning(warning).length).toBeGreaterThan(0);
    }
    expect(formatImportWarning({ kind: 'missing_base_url', source: 'v3_servers' })).toContain(
      'Specification declared no servers',
    );
  });

  it('parses sanitized send card details', () => {
    expect(readSendCardDetails(null)).toBeNull();
    expect(
      readSendCardDetails({
        bytes: 512,
        collectionName: 'Payments',
        durationMs: 42,
        method: 'POST',
        protocol: 'http',
        requestName: 'Charge',
        status: 200,
        statusText: 'OK',
        truncated: false,
        url: 'https://api.example.com/charges?api_key=[redacted]',
      }),
    ).toEqual({
      bytes: 512,
      collectionName: 'Payments',
      durationMs: 42,
      method: 'POST',
      protocol: 'http',
      requestName: 'Charge',
      status: 200,
      statusText: 'OK',
      truncated: false,
      url: 'https://api.example.com/charges?api_key=[redacted]',
    });
  });
});
