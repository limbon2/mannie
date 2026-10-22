// The line and the status of a tool call's row (docs/features/system-tools/
// tool-activity.md, tool calls in the reply): what the call did once it has a
// result, what it does before, how long a command has run, and which statuses
// the row names.
import { describe, expect, it } from 'vitest';

import type { ToolCallView, ToolResult } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  callArguments,
  callStatus,
  callSummary,
  firstLine,
  folderName,
  toolIcon,
} from '../../../../../../apps/desktop/src/lib/utils/tools-summaries.js';

function call(tool: string, args: unknown, changes: Partial<ToolCallView> = {}): ToolCallView {
  return {
    accesses: [],
    activity: null,
    arguments: JSON.stringify(args),
    argumentsCut: false,
    decision: null,
    endedAt: null,
    notRun: null,
    request: null,
    result: null,
    startedAt: null,
    status: 'pending',
    tool,
    ...changes,
  };
}

function result(outcome: ToolResult['outcome']): ToolResult {
  return { archives: [], outcome, output: '' };
}

describe('callSummary', () => {
  it('names what a call does from the arguments the model wrote', () => {
    expect(callSummary(call('read_file', { path: 'src/main.rs' }))).toContain('src/main.rs');
    expect(callSummary(call('run_command', { command: 'cargo test\ncargo build' }))).toContain('cargo test …');
    expect(callSummary(call('copy_path', { destination: 'b.txt', source: 'a.txt' }))).toMatch(/a\.txt.*b\.txt/u);
  });

  it('falls back to the prepared accesses when the arguments are cut', () => {
    const cut = call(
      'write_file',
      {},
      {
        accesses: [
          {
            entries: null,
            exists: false,
            folder: false,
            identity: '1:2',
            kind: 'path',
            mode: 'create',
            path: 'D:\\Projects\\app\\notes.md',
          },
        ],
        arguments: '{"path": "notes.md", "content": "a very long',
        argumentsCut: true,
      },
    );

    expect(callSummary(cut)).toContain('D:\\Projects\\app\\notes.md');
  });

  it('names the tool while the model still writes the call', () => {
    const receiving = call('read_file', {}, { arguments: '', status: 'receiving' });

    expect(callSummary(receiving)).not.toBe('');
    expect(callSummary(receiving)).not.toContain('read_file');
    expect(callSummary(call('translate', {}))).toContain('translate');
  });

  it('says what a finished call did', () => {
    const edited = call(
      'edit_file',
      { path: 'main.rs' },
      {
        result: result({ kind: 'edited', path: 'D:\\Projects\\app\\main.rs', replacements: 2 }),
        status: 'completed',
      },
    );
    const ran = call(
      'run_command',
      { command: 'cargo test' },
      {
        result: result({
          command: 'cargo test',
          durationMs: 3_200,
          ending: 'exited',
          exitCode: 0,
          kind: 'command_ran',
        }),
        status: 'completed',
      },
    );

    expect(callSummary(edited)).toContain('D:\\Projects\\app\\main.rs');
    expect(callSummary(edited)).toMatch(/2/u);
    expect(callSummary(ran)).toContain('cargo test');
    expect(callSummary(ran)).toMatch(/0/u);
  });

  it('names a command in the background by its number once it started', () => {
    const starting = call('start_command', { command: 'npm run dev\n--port 5173' });
    const started = call(
      'start_command',
      { command: 'npm run dev' },
      { result: result({ command: 'npm run dev', kind: 'command_started', number: 3 }), status: 'completed' },
    );
    const read = call(
      'read_command_output',
      { command: 3 },
      {
        result: result({
          ending: null,
          exitCode: null,
          firstLine: 120,
          kind: 'command_output_read',
          lastLine: 180,
          number: 3,
          totalLines: 180,
        }),
        status: 'completed',
      },
    );
    const nothing = call(
      'read_command_output',
      { command: 3 },
      {
        result: result({
          ending: null,
          exitCode: null,
          firstLine: 0,
          kind: 'command_output_read',
          lastLine: 0,
          number: 3,
          totalLines: 180,
        }),
        status: 'completed',
      },
    );
    const stopped = call(
      'stop_command',
      { command: 3 },
      {
        result: result({ ending: 'stopped_by_agent', exitCode: null, kind: 'command_stopped', number: 3 }),
        status: 'completed',
      },
    );

    expect(callSummary(starting)).toContain('npm run dev …');
    expect(callSummary(started)).toContain('3');
    expect(callSummary(read)).toMatch(/120.*180.*3|3.*120.*180/u);
    expect(callSummary(nothing)).not.toBe(callSummary(read));
    expect(callSummary(stopped)).toContain('3');
    expect(callSummary(call('stop_command', { command: 4 }))).toContain('4');
    expect(callSummary(call('read_command_output', {}))).not.toBe('');
    expect(toolIcon('start_command')).not.toBe(toolIcon('stop_command'));
  });

  it('names the project of a move, by its name rather than its folder', () => {
    const access = {
      folder: 'D:\\Projects\\atlas',
      id: '0199e0f2-0000-7000-a000-000000000001',
      kind: 'project' as const,
      name: 'atlas',
    };
    const written = call('move_to_project', { project: 'atlas' });
    const prepared = call('move_to_project', { project: 'D:\\Projects\\atlas' }, { accesses: [access] });
    const moved = call(
      'move_to_project',
      { project: 'atlas' },
      {
        accesses: [access],
        result: result({ folder: 'D:\\Projects\\atlas', kind: 'moved_to_project', name: 'atlas' }),
        status: 'completed',
      },
    );

    expect(callSummary(written)).toContain('atlas');
    expect(callSummary(prepared)).toContain('atlas');
    expect(callSummary(prepared)).not.toContain('D:\\Projects');
    expect(callSummary(moved)).toContain('atlas');
    expect(callSummary(moved)).not.toBe(callSummary(prepared));
    expect(toolIcon('move_to_project')).not.toBe(toolIcon('translate'));
  });

  it('keeps an exit code without grouping', () => {
    const crashed = call(
      'run_command',
      { command: 'app.exe' },
      {
        result: result({
          command: 'app.exe',
          durationMs: 100,
          ending: 'exited',
          exitCode: -1_073_741_819,
          kind: 'command_ran',
        }),
        status: 'completed',
      },
    );

    expect(callSummary(crashed)).toContain('-1073741819');
  });

  it('names what a failed call tried, and what a call of a newer version does', () => {
    const failed = call(
      'read_file',
      { path: 'missing.txt' },
      {
        result: result({ failure: { code: 'not_found', path: 'D:\\missing.txt' }, kind: 'failed' }),
        status: 'completed',
      },
    );
    const unknown = call(
      'read_file',
      { path: 'notes.md' },
      { result: result({ kind: 'unknown' }), status: 'completed' },
    );

    expect(callSummary(failed)).toContain('missing.txt');
    expect(callSummary(unknown)).toContain('notes.md');
  });

  it("names a finished call of a plugin's tool by its tool, which reports no outcome of its own", () => {
    const finished = call(
      'notes_find',
      { query: 'groceries' },
      { result: result({ kind: 'plugin', plugin: 'notes' }), status: 'completed' },
    );
    const running = call('notes_find', { query: 'groceries' }, { status: 'running' });

    expect(callSummary(finished)).toBe(callSummary(running));
    expect(callSummary(finished)).toContain('notes_find');
    expect(callStatus(finished, 0).tone).toBe('done');
  });

  it("names a plugin's command on a remote host by its command and host, once prepared", () => {
    const command = 'journalctl -u nginx\n--since today';
    const access = {
      background: false,
      command,
      endpoint: 'deploy@build-box.lan:22',
      identity: 'SHA256:uX0dFmYbVKUH4bZmL1nOb4TaZ5WPMbgUpkOvR3EJPr0',
      kind: 'remote_command' as const,
      label: 'build-box',
      plugin: 'ssh',
      target: '0199e0f2-0000-7000-8000-0000000000aa',
    };
    const written = call('run_ssh_command', { command, host: 'build-box' });
    const prepared = call('run_ssh_command', { command, host: 'build-box' }, { accesses: [access] });
    const finished = call(
      'run_ssh_command',
      { command, host: 'build-box' },
      { accesses: [access], result: result({ kind: 'plugin', plugin: 'ssh' }), status: 'completed' },
    );

    expect(callSummary(written)).toContain('run_ssh_command');
    expect(callSummary(prepared)).toContain('journalctl -u nginx …');
    expect(callSummary(prepared)).toContain('build-box');
    expect(callSummary(prepared)).not.toContain('--since');
    expect(callSummary(prepared)).not.toContain('run_ssh_command');
    expect(callSummary(finished)).toBe(callSummary(prepared));
  });

  it("says that a plugin's command on a remote host starts in the background", () => {
    const access = {
      background: true,
      command: 'journalctl -f',
      endpoint: 'deploy@build-box.lan:22',
      identity: 'SHA256:uX0dFmYbVKUH4bZmL1nOb4TaZ5WPMbgUpkOvR3EJPr0',
      kind: 'remote_command' as const,
      label: 'build-box',
      plugin: 'ssh',
      target: '0199e0f2-0000-7000-8000-0000000000aa',
    };
    const prepared = call('start_ssh_command', { command: 'journalctl -f', host: 'build-box' }, { accesses: [access] });

    expect(callSummary(prepared)).toBe('Start journalctl -f on build-box in the background');
  });
});

describe('callStatus', () => {
  it('asks for attention while the call waits for the owner', () => {
    expect(callStatus(call('write_file', {}, { status: 'awaiting_approval' }), 0).tone).toBe('attention');
  });

  it('tells a failed result from a successful one', () => {
    const failed = call(
      'read_file',
      {},
      {
        result: result({ failure: { code: 'no_workspace' }, kind: 'failed' }),
        status: 'completed',
      },
    );
    const done = call(
      'read_file',
      {},
      {
        result: result({ firstLine: 1, kind: 'read', lastLine: 2, path: 'a', totalLines: 2 }),
        status: 'completed',
      },
    );

    expect(callStatus(failed, 0).tone).toBe('failed');
    expect(callStatus(done, 0).tone).toBe('done');
    expect(callStatus(failed, 0).label).not.toBe(callStatus(done, 0).label);
  });

  it('marks a plugin’s call that failed with its own code as failed', () => {
    const pluginCall = (failure: string | null): ToolCallView =>
      call(
        'git_merge',
        {},
        {
          plugin: 'git',
          result: result({ data: null, failure, kind: 'plugin', plugin: 'git' }),
          status: 'completed',
        },
      );

    expect(callStatus(pluginCall('merge_conflicts'), 0).tone).toBe('failed');
    expect(callStatus(pluginCall(null), 0).tone).toBe('done');
  });

  it('says how long a command has run, in whole seconds', () => {
    const running = call('run_command', { command: 'sleep 30' }, { startedAt: 1_000, status: 'executing' });
    const reading = call('read_file', { path: 'a' }, { startedAt: 1_000, status: 'running' });

    expect(callStatus(running, 13_900).label).toMatch(/12/u);
    expect(callStatus(running, 13_900).label).not.toMatch(/12[.,]9/u);
    expect(callStatus(reading, 13_900).label).not.toMatch(/12/u);
  });

  it('marks a call that may have run', () => {
    expect(callStatus(call('delete_path', {}, { status: 'uncertain' }), 0).tone).toBe('failed');
    expect(callStatus(call('delete_path', {}, { status: 'canceled' }), 0).tone).toBe('ended');
  });

  it('leaves the status of a call that works or ended well to the icon', () => {
    const done = call(
      'read_file',
      {},
      {
        result: result({ firstLine: 1, kind: 'read', lastLine: 2, path: 'a', totalLines: 2 }),
        status: 'completed',
      },
    );
    const quiet = [
      call('read_file', {}, { status: 'receiving' }),
      call('read_file', {}, { status: 'pending' }),
      call('read_file', {}, { startedAt: 1_000, status: 'running' }),
      call('run_command', {}, { status: 'executing' }),
      done,
    ].map((current) => callStatus(current, 5_000).quiet);

    expect(quiet).toEqual([true, true, true, true, true]);
  });

  it('names every status that needs the owner, and how long a command has run', () => {
    const failed = call(
      'read_file',
      {},
      { result: result({ failure: { code: 'no_workspace' }, kind: 'failed' }), status: 'completed' },
    );
    const named = [
      call('write_file', {}, { status: 'awaiting_approval' }),
      failed,
      call('write_file', {}, { status: 'denied' }),
      call('write_file', {}, { status: 'canceled' }),
      call('delete_path', {}, { status: 'uncertain' }),
      call('write_file', {}, { status: 'not_run' }),
      call('run_command', {}, { startedAt: 1_000, status: 'executing' }),
    ].map((current) => callStatus(current, 5_000).quiet);

    expect(named).toEqual([false, false, false, false, false, false, false]);
  });
});

describe('helpers', () => {
  it('reads the arguments only when they are a complete object', () => {
    expect(callArguments(call('read_file', { path: 'a' }))).toEqual({ path: 'a' });
    expect(callArguments(call('read_file', ['a']))).toEqual({});
    expect(callArguments(call('read_file', {}, { arguments: '{"path": "a', argumentsCut: true }))).toEqual({});
    expect(callArguments(call('read_file', {}, { arguments: 'not json' }))).toEqual({});
  });

  it('names a folder by its last name', () => {
    expect(folderName('D:\\Projects\\app')).toBe('app');
    expect(folderName('/home/you/projects/app/')).toBe('app');
    expect(folderName('/')).toBe('/');
  });

  it('shortens a command to its first line', () => {
    expect(firstLine('git status')).toBe('git status');
    expect(firstLine('git status\r\ngit log')).toBe('git status …');
  });

  it('gives an unknown tool the generic icon', () => {
    expect(toolIcon('translate')).toBe(toolIcon('another_unknown_tool'));
    expect(toolIcon('read_file')).not.toBe(toolIcon('translate'));
  });
});
