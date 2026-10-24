// What the server dialog sends (plugins/mcp/docs/README.md, registering a
// server): the fields of its kind only, one argument on each line, a stored
// secret kept by an empty field, and the names that must be secret, as the
// core decides them.
import { describe, expect, it } from 'vitest';

import { argumentsOf, inputOf, mustBeSecret, rowsToMove, type ServerFields } from '../../../ui/utils/server-form.js';

function fields(overrides: Partial<ServerFields> = {}): ServerFields {
  return {
    argumentsText: '-y\n@modelcontextprotocol/server-github\n',
    clientId: 'ignored',
    clientSecret: '',
    folder: ' D:\\work ',
    forgetClientSecret: false,
    images: true,
    instructions: false,
    name: ' GitHub ',
    oauth: true,
    plain: [
      { name: 'GITHUB_HOST', value: 'github.com' },
      { name: ' ', value: 'dropped' },
    ],
    prefix: '',
    program: ' npx ',
    secrets: [
      { name: 'GITHUB_TOKEN', stored: true, value: '' },
      { name: 'NEW_TOKEN', stored: false, value: 'planted' },
    ],
    timeoutSeconds: 120,
    transport: 'stdio',
    url: 'https://ignored.example.com',
    ...overrides,
  };
}

describe('the input of the server dialog', () => {
  it('sends the fields of a local server only, and keeps a stored secret with an empty field', () => {
    const input = inputOf(fields());

    expect(input).toEqual({
      arguments: ['-y', '@modelcontextprotocol/server-github'],
      clientId: '',
      clientSecret: null,
      folder: 'D:\\work',
      forgetClientSecret: false,
      images: true,
      instructions: false,
      name: 'GitHub',
      oauth: false,
      plain: [{ name: 'GITHUB_HOST', value: 'github.com' }],
      prefix: '',
      program: 'npx',
      secrets: [
        { name: 'GITHUB_TOKEN', value: null },
        { name: 'NEW_TOKEN', value: 'planted' },
      ],
      timeoutSeconds: 120,
      transport: 'stdio',
      url: '',
    });
  });

  it('sends the fields of a remote server only, with its sign-in', () => {
    const input = inputOf(fields({ clientId: ' mannie ', clientSecret: 'secret', transport: 'http' }));

    expect(input.program).toBe('');
    expect(input.arguments).toEqual([]);
    expect(input.folder).toBe('');
    expect(input.url).toBe('https://ignored.example.com');
    expect(input.oauth).toBe(true);
    expect(input.clientId).toBe('mannie');
    expect(input.clientSecret).toBe('secret');
  });

  it('reads one argument on each line and leaves empty lines out', () => {
    expect(argumentsOf('a b\r\n\nc\n')).toEqual(['a b', 'c']);
    expect(argumentsOf('')).toEqual([]);
  });
});

describe('the names that must be secret', () => {
  it('are those whose words name a secret, and the headers that carry credentials', () => {
    expect(mustBeSecret('stdio', 'GITHUB_TOKEN')).toBe(true);
    expect(mustBeSecret('stdio', 'apiKey')).toBe(true);
    expect(mustBeSecret('stdio', 'GITHUB_HOST')).toBe(false);
    expect(mustBeSecret('http', 'Authorization')).toBe(true);
    expect(mustBeSecret('http', 'X-Api-Key')).toBe(true);
    expect(mustBeSecret('stdio', 'Authorization')).toBe(false);
    expect(rowsToMove({ plain: fields().plain, transport: 'stdio' })).toEqual([]);
    expect(rowsToMove({ plain: [{ name: 'MY_SECRET', value: 'x' }], transport: 'stdio' })).toEqual(['MY_SECRET']);
  });
});
