// What an operation touches, as an approval shows it (docs/features/
// system-tools/tool-activity.md, approval requests), rendered on the server
// side: a run of an installed program shows its words, the program that the
// core found, its folder, the arguments that its plugin fixes and what it
// does beyond the workspace, with the names of what it discards (ADR-0038),
// and a call of a tool server's tool shows the tool, its arguments, the
// server, where it runs and what the server says of the tool (ADR-0043).
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import ToolAccessList from '../../../../../../../../apps/desktop/src/lib/components/app/tools/ToolAccessList.svelte';
import type { Access } from '../../../../../../../../apps/desktop/src/lib/generated/bindings.js';

const RUN: Access = {
  arguments: ['reset', '--hard', 'HEAD~1'],
  effects: [
    { effect: 'runs_folder_code', name: 'post-checkout', source: '.git/hooks/post-checkout' },
    {
      count: 3,
      effect: 'discards_work',
      items: ['src/parser.rs', 'README.md'],
      snapshot: true,
      work: 'uncommitted_changes',
    },
  ],
  fixed: ['-c', 'gc.auto=0'],
  kind: 'program',
  name: 'git',
  plugin: 'git',
  program: 'C:\\Program Files\\Git\\cmd\\git.exe',
  state: 'HEAD 1a2b3c',
  workingDirectory: 'D:\\Projects\\app',
};

const SERVER_CALL: Access = {
  arguments: '{"title":"Crash on start"}',
  endpoint: { arguments: ['-y', '@modelcontextprotocol/server-everything'], kind: 'program', program: 'npx' },
  hints: ['destructive', 'open_world'],
  kind: 'external_tool',
  label: 'Everything',
  plugin: 'mcp',
  target: '0199e0f2-0000-7000-8000-0000000000dd',
  title: 'Create an issue',
  tool: 'create_issue',
};

function shown(accesses: readonly Access[]): string {
  return render(ToolAccessList, { props: { accesses } }).body;
}

describe('ToolAccessList', () => {
  it('shows a run of an installed program with its words, program, folder and fixed arguments', () => {
    const body = shown([RUN]);

    expect(body).toContain('git reset --hard HEAD~1');
    expect(body).toContain('C:\\Program Files\\Git\\cmd\\git.exe');
    expect(body).toContain('D:\\Projects\\app');
    expect(body).toContain('-c gc.auto=0');
    expect(body).toContain('data-program-run');
    // The state is for the core's digest, and the owner never sees it.
    expect(body).not.toContain('HEAD 1a2b3c');
  });

  it('lists what a run does beyond the workspace, with the names of what it discards', () => {
    const body = shown([RUN]);

    expect(body).toContain('data-program-effects');
    expect(body).toContain('post-checkout');
    expect(body).toContain('.git/hooks/post-checkout');
    expect(body).toContain('src/parser.rs');
    expect(body).toContain('README.md');
    expect(body).toMatch(/Discards 3 uncommitted changes/u);
    expect(body).toMatch(/and 1 more/u);
    expect(body).toMatch(/snapshot is kept first/u);
  });

  it('shows no list of effects for a run without one', () => {
    const body = shown([{ ...RUN, effects: [] }]);

    expect(body).not.toContain('data-program-effects');
  });

  it('shows a call of a tool server’s tool with its arguments, the server, where it runs and its hints', () => {
    const body = shown([SERVER_CALL]);

    expect(body).toContain('data-external-tool');
    expect(body).toContain('create_issue');
    expect(body).toContain('Create an issue');
    expect(body).toContain('Everything');
    expect(body).toContain('npx -y @modelcontextprotocol/server-everything');
    expect(body).toContain('{"title":"Crash on start"}');
    expect(body).toContain('data-tool-hints');
    // The list joins the hints in the owner's language, which the test leaves open.
    expect(body).toContain('The server says that this tool may delete or overwrite');
    expect(body).toContain('reaches beyond the server');
    // The plugin's key for the server is for the policy, and the owner never sees it.
    expect(body).not.toContain('0199e0f2');
    expect(body).not.toMatch(/longer than shown/u);
  });

  it('says when the arguments of a tool server’s call are cut, and leaves out hints that the server did not give', () => {
    const body = shown([
      {
        ...SERVER_CALL,
        argumentsCut: true,
        endpoint: { kind: 'url', url: 'https://mcp.example.com/mcp' },
        hints: [],
        title: null,
      },
    ]);

    expect(body).toMatch(/longer than shown/u);
    expect(body).toContain('https://mcp.example.com/mcp');
    expect(body).not.toContain('data-tool-hints');
    expect(body).not.toContain('Create an issue');
  });
});
