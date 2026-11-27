// The context meter in the composer and toolbar (docs/features/chat/
// conversation-view.md, layout): shows the budget share and marks when older
// tool results are trimmed from the active turn (ADR-0083).
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import ContextMeter from '../../../../../../../../apps/desktop/src/lib/components/app/chat/ContextMeter.svelte';
import type { ContextUsage } from '../../../../../../../../apps/desktop/src/lib/generated/bindings.js';

const CONVERSATION = '0199e0f2-0000-7000-8000-000000000001';

describe('ContextMeter', () => {
  it('shows the context percentage without a trimmed badge by default', () => {
    const context: ContextUsage = { budgetTokens: 100_000, estimated: false, usedTokens: 50_000 };
    const { body } = render(ContextMeter, {
      props: { checkpoint: null, context, conversationId: CONVERSATION },
    });

    expect(body).toContain('50% of the context');
    expect(body).not.toContain('Trimmed');
  });

  it('shows the trimmed badge and accessible label when older tool results are pruned', () => {
    const context: ContextUsage = {
      budgetTokens: 100_000,
      estimated: false,
      resultsPruned: true,
      unprunedTokens: 88_000,
      usedTokens: 62_000,
    };
    const { body } = render(ContextMeter, {
      props: { checkpoint: null, context, conversationId: CONVERSATION },
    });

    expect(body).toContain('62% of the context');
    expect(body).toContain('Trimmed');
    expect(body).toContain('62% of the context, older tool results trimmed');
  });
});
