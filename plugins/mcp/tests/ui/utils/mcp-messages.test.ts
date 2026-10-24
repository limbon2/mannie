// Messages for the MCP plugin's codes (ADR-0019): the plugin's errors and
// the fields that the host's check refused, the statuses and failures of
// servers, their warnings and the events of their logs, the hints and the
// problems of tools, the refusals of a pasted configuration and the
// sentences of mentions.
import { describe, expect, it } from 'vitest';

import type { McpError } from '../../../ui/generated/bindings.js';
import type { McpFailure } from '../../../ui/types/mcp.js';
import {
  entryRefusalMessage,
  isServerGone,
  logEventMessage,
  mcpFailureMessage,
  refusedField,
  resourceMentionRefusal,
  resourceMentionSummary,
  serverFailureMessage,
  serverStatusLabel,
  serverWarningMessage,
  toolHintLabel,
  toolProblemMessage,
} from '../../../ui/utils/mcp-messages.js';

function pluginFailure(error: McpError): McpFailure {
  return { error: { ...error, description: 'refused', origin: 'plugin' }, kind: 'core' };
}

function invalidInput(field: string): McpFailure {
  return { error: { code: 'invalid_input', description: 'refused', field, origin: 'host' }, kind: 'core' };
}

describe('the messages of failed calls', () => {
  it('word the plugin errors with their values and the refused fields', () => {
    expect(mcpFailureMessage(pluginFailure({ code: 'too_many_tools', limit: 64 }))).toBe(
      'At most 64 tools of all servers can be on. Turn another tool off first.',
    );
    expect(mcpFailureMessage(pluginFailure({ code: 'name_taken' }))).toContain('Another server has this name');
    expect(mcpFailureMessage(invalidInput('url'))).toContain('Enter an https URL');
    expect(refusedField(invalidInput('url'))).toBe('url');
    expect(refusedField(pluginFailure({ code: 'name_taken' }))).toBeNull();
    expect(isServerGone(pluginFailure({ code: 'server_not_found' }))).toBe(true);
    expect(mcpFailureMessage({ kind: 'ipc' })).not.toBe('');
  });
});

describe('the words of servers', () => {
  it('name each status and say why a server failed', () => {
    expect(serverStatusLabel('sign_in_needed')).toBe('Sign-in needed');
    expect(serverFailureMessage({ code: 'exited', exitCode: 3 })).toContain('exit code 3');
    expect(serverFailureMessage({ code: 'exited', exitCode: null })).toBe(
      'The program ended before it answered. Show log may say why.',
    );
    expect(serverFailureMessage({ code: 'http_status', status: 401 })).toBe(
      'The server answered with the HTTP status 401.',
    );
    expect(serverFailureMessage({ code: 'crashed', restarts: 5 })).toContain('5 times in five minutes');
  });

  it('word warnings by count, and number arguments from 1', () => {
    expect(serverWarningMessage({ code: 'token_in_argument', index: 0 })).toContain('Argument 1 looks like a token');
    expect(serverWarningMessage({ code: 'new_tools', count: 1 })).toBe(
      'The server added a tool, which is off until you turn it on.',
    );
    expect(serverWarningMessage({ code: 'new_tools', count: 3 })).toBe(
      'The server added 3 tools, which are off until you turn them on.',
    );
  });

  it('word the events of a log', () => {
    expect(logEventMessage({ kind: 'connected', version: '2026-07-28' })).toBe('Connected with protocol 2026-07-28');
    expect(logEventMessage({ kind: 'listed', tools: 1 })).toBe('Listed 1 tool');
    expect(logEventMessage({ failure: { code: 'protocol' }, kind: 'failed' })).toBe(
      'Failed: The server does not speak MCP as Mannie does.',
    );
    expect(logEventMessage({ kind: 'restarting', seconds: 4 })).toBe('Connecting again in 4 seconds');
  });
});

describe('the words of tools, configurations and mentions', () => {
  it("present the hints as the server's claims and say why a tool is left out", () => {
    expect(toolHintLabel('read_only')).toBe('Says it changes nothing');
    expect(toolProblemMessage('activity_clash')).toContain('activity and activity_value');
    expect(entryRefusalMessage('sse_only')).toContain('server-sent events');
    expect(resourceMentionRefusal('resource_not_found')).not.toBeNull();
    expect(resourceMentionRefusal('unknown')).toBeNull();
    expect(resourceMentionSummary({ code: 'resource', values: {} })).toBe("The agent got the resource's content");
  });
});
