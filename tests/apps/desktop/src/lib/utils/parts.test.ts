import { describe, expect, it } from 'vitest';

import type { PartContent, PartView } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { partKind } from '../../../../../../apps/desktop/src/lib/utils/parts.js';

function part(content: PartContent): PartView {
  return {
    content,
    createdAt: 0,
    id: '0199e0f2-0000-7000-8000-000000000001',
    position: 1,
    runId: '0199e0f2-0000-7000-8000-000000000002',
    state: 'complete',
    turnId: '0199e0f2-0000-7000-8000-000000000003',
  };
}

describe('partKind', () => {
  it('is the kind of a part the core knows', () => {
    expect(partKind(part({ kind: 'text', text: 'Hello' }))).toBe('text');
    expect(partKind(part({ failure: { code: 'internal' }, kind: 'error' }))).toBe('error');
  });

  it('is the name of a kind that the core does not know', () => {
    expect(partKind(part({ kind: 'unknown', name: 'tool_call' }))).toBe('tool_call');
  });
});
