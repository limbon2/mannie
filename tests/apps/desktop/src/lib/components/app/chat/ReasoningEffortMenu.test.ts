import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import ReasoningEffortMenu from '../../../../../../../../apps/desktop/src/lib/components/app/chat/ReasoningEffortMenu.svelte';
import type { ReasoningEffort } from '../../../../../../../../apps/desktop/src/lib/generated/bindings.js';

function shown(effort: ReasoningEffort | null, supportedEfforts?: ReasoningEffort[]): string {
  return render(ReasoningEffortMenu, {
    props: {
      effort,
      onChoose: () => Promise.resolve(null),
      profile: { reasoningEffort: 'medium', supportedEfforts },
    },
  }).body;
}

describe('the reasoning effort in the chat header', () => {
  it('shows the configured effort before the owner chooses one', () => {
    expect(shown(null, ['medium', 'high'])).toContain('Reasoning effort: Medium');
  });

  it('shows the remembered level for a matching model or an endpoint with unknown capabilities', () => {
    expect(shown('high', ['high'])).toContain('Reasoning effort: High');
    expect(shown('high')).toContain('Reasoning effort: High');
  });

  it('shows the server default when the new model cannot accept the remembered effort', () => {
    const body = shown('high', ['medium']);
    expect(body).toMatch(/Reasoning effort: The server.{1,8}s default/u);
    expect(body).not.toContain('Reasoning effort: Medium');
  });
});
