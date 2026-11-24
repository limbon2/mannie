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

function completed(images: ToolImageRef[], extra: Partial<ToolCallView> = {}): ToolCallView {
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
    ...extra,
  };
}

function shown(call: ToolCallView, extra: { hasFileChange?: boolean; outcomeNote?: string | null } = {}): string {
  return render(ToolCallDetails, { props: { call, conversationId: CONVERSATION, partId: PART, ...extra } }).body;
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

  it('places the result above metadata and collapses raw arguments when accesses describe the call', () => {
    const body = shown(
      completed([], {
        accesses: [
          { background: false, command: 'echo hello', kind: 'command', shell: 'pwsh', workingDirectory: 'C:\\' },
        ],
        arguments: '{"command":"echo hello"}',
        decision: 'full_access',
        tool: 'run_command',
      }),
      { outcomeNote: 'Exit code 0, 1.1 sec' },
    );

    expect(body).toContain('echo hello');
    expect(body).toContain('Copy');
    expect(body).toContain('data-tool-metadata');
    expect(body).toContain('Exit code 0, 1.1 sec');
    expect(body).toContain('Full access allowed this.');
    expect(body).toContain('data-tool-arguments-toggle');
    expect(body).not.toContain('What it touches');
    expect(body.indexOf('What the model read')).toBeLessThan(body.indexOf('data-tool-metadata'));
    expect(body.indexOf('data-tool-metadata')).toBeLessThan(body.indexOf('data-tool-arguments-toggle'));
  });

  it('renders raw arguments directly when no accesses, plugin details or saved diff are present', () => {
    const body = shown(completed([], { arguments: '{"query":"status"}' }));

    expect(body).not.toContain('data-tool-arguments-toggle');
    expect(body).toContain('"query": "status"');
  });
});
