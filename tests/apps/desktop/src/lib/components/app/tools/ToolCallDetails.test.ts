// The details of a tool call (docs/features/system-tools/tool-activity.md,
// tool calls in the reply), rendered on the server side: a result that
// carries images offers a button for each one, named by its position among
// the images that the tool returned (ADR-0044), and a result without images
// offers none.
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import ToolCallDetails from '../../../../../../../../apps/desktop/src/lib/components/app/tools/ToolCallDetails.svelte';
import type { ToolCallView, ToolImageRef } from '../../../../../../../../apps/desktop/src/lib/generated/bindings.js';

const CONVERSATION = '0199e0f2-0000-7000-8000-000000000001';
const PART = '0199e0f2-0000-7000-8000-000000000002';

function completed(images: ToolImageRef[]): ToolCallView {
  return {
    accesses: [],
    activity: null,
    arguments: '{}',
    argumentsCut: false,
    decision: null,
    endedAt: null,
    interruption: null,
    notRun: null,
    plugin: 'mcp',
    request: null,
    result: { archives: [], images, outcome: { kind: 'unknown' }, output: 'The page.' },
    startedAt: null,
    status: 'completed',
    tool: 'mcp_browser_screenshot',
  };
}

function shown(call: ToolCallView): string {
  return render(ToolCallDetails, { props: { call, conversationId: CONVERSATION, partId: PART } }).body;
}

describe('ToolCallDetails', () => {
  it('offers each image of a result by its number', () => {
    const body = shown(
      completed([
        { bytes: 90_000, format: 'png', height: 720, position: 0, width: 1280 },
        { bytes: 30_000, format: 'jpeg', height: 480, position: 2, width: 640 },
      ]),
    );

    expect(body).toContain('data-tool-images');
    expect(body).toContain('Images the model saw');
    expect(body).toContain('Show image 1');
    expect(body).toContain('Show image 3');
    expect(body).not.toContain('Show image 2');
    expect(body).toContain('The page.');
  });

  it('offers no images for a result without any', () => {
    const body = shown(completed([]));

    expect(body).not.toContain('data-tool-images');
    expect(body).not.toContain('Show image');
  });
});
