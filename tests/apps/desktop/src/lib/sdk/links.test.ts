// Links of the TypeScript plugin SDK (ADR-0043, decision 9): a plugin's UI
// opens a link through the core's own command, which checks it, and learns
// why a link did not open.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { openLink } from '../../../../../../apps/desktop/src/lib/sdk/links.js';

const core = vi.hoisted(() => ({ openLink: vi.fn() }));
vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));

beforeEach(() => {
  core.openLink.mockReset();
});

describe('openLink', () => {
  it('opens a link through the core and returns no failure', async () => {
    core.openLink.mockResolvedValue({ data: null, status: 'ok' });

    const failure = await openLink('https://auth.example.com/authorize?state=abc');

    expect(failure).toBeNull();
    expect(core.openLink).toHaveBeenCalledWith({ url: 'https://auth.example.com/authorize?state=abc' });
  });

  it('returns why the core refused a link, and a failure of the call itself', async () => {
    core.openLink.mockResolvedValueOnce({ error: { code: 'unsupported_scheme', scheme: 'file' }, status: 'error' });
    core.openLink.mockRejectedValueOnce(new Error('the window is gone'));

    expect(await openLink('file:///etc/passwd')).toEqual({
      error: { code: 'unsupported_scheme', scheme: 'file' },
      kind: 'core',
    });
    expect(await openLink('https://auth.example.com/')).toEqual({ kind: 'ipc' });
  });
});
