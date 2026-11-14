import { describe, expect, it } from 'vitest';

import { browserAddress } from '../../../ui/utils/browser-address.js';

describe('browserAddress', () => {
  it.each([
    [' example.com ', 'https://example.com'],
    ['example.com/path?q=term#section', 'https://example.com/path?q=term#section'],
    ['localhost:8443/path', 'https://localhost:8443/path'],
    ['127.0.0.1:8443', 'https://127.0.0.1:8443'],
    ['[::1]:8443', 'https://[::1]:8443'],
    ['http://localhost:3000', 'http://localhost:3000'],
    ['HTTPS://example.com', 'HTTPS://example.com'],
    ['file:///example', 'file:///example'],
    ['javascript:alert(1)', 'javascript:alert(1)'],
    ['/relative/path', '/relative/path'],
    ['//example.com/path', '//example.com/path'],
    [' ', ''],
  ])('normalizes %s to %s', (input, expected) => {
    expect(browserAddress(input)).toBe(expected);
  });
});
