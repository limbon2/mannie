// What the owner reads about background commands (docs/features/
// system-tools/background-commands.md, interface): a command's state, where
// it runs, the address of its output's panel, and the row of a wake-up.
import { describe, expect, it } from 'vitest';

import type { BackgroundCommandView } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  commandLabel,
  commandPanelArgument,
  commandPlace,
  commandState,
  endingLabel,
  noteTitle,
  parseCommandPanelArgument,
  runningFirst,
  startedCommand,
} from '../../../../../../apps/desktop/src/lib/utils/tools-background.js';

const CONVERSATION = '0199e0f2-0000-7000-8000-000000000001';

function command(number: number, changes: Partial<BackgroundCommandView> = {}): BackgroundCommandView {
  return {
    activity: null,
    archiveId: null,
    bytes: 0,
    command: 'npm run dev\n--port 5173',
    endedAt: null,
    ending: null,
    exitCode: null,
    lines: 0,
    number,
    onWorker: false,
    partId: `0199e0f2-0000-7000-8000-00000000000${String(number)}`,
    signal: null,
    source: { kind: 'local', shell: '/bin/sh', workingDirectory: '/home/owner/app' },
    startedAt: 1_000,
    ...changes,
  };
}

describe('commandState', () => {
  it('says how long a command has run, in whole seconds', () => {
    const state = commandState(command(1), 1_000 + 75_900);

    expect(state).toContain('1');
    expect(state).toContain('15');
    expect(commandState(command(1), 1_000 + 75_000)).toBe(state);
  });

  it('says how a command ended, with its exit code', () => {
    const exited = commandState(command(1, { ending: 'exited', exitCode: 3 }), 0);
    const finished = commandState(command(1, { ending: 'exited', exitCode: null }), 0);

    expect(exited).toContain('3');
    expect(finished).not.toContain('3');
  });

  it('names every ending apart', () => {
    const endings = [
      'exited',
      'ended_by_host',
      'stopped_by_agent',
      'stopped_by_owner',
      'task_ended',
      'plugin_off',
      'session_closed',
      'interrupted',
      'let_go',
    ] as const;

    const labels = endings.map((ending) => endingLabel(ending, null, null));

    expect(new Set(labels).size).toBe(endings.length);
  });

  it("says the exit code or the signal that a command's host reported", () => {
    expect(endingLabel('ended_by_host', 0, null)).toBe('Ended on its host with exit code 0');
    expect(endingLabel('ended_by_host', null, 'TERM')).toBe('Ended on its host by the signal TERM');
    expect(endingLabel('ended_by_host', null, null)).toBe('Ended on its host');
  });
});

describe('places and labels', () => {
  it('names the folder of a local command and the host of a remote one', () => {
    expect(commandPlace(command(1))).toContain('/home/owner/app');
    const remote = commandPlace(
      command(2, {
        source: { endpoint: 'deploy@build-box.lan:22', kind: 'remote', label: 'build-box', plugin: 'ssh' },
      }),
    );
    expect(remote).toContain('build-box');
    expect(remote).toContain('deploy@build-box.lan:22');
  });

  it('labels a command by its number and what its start call said it does', () => {
    const label = commandLabel(command(4, { activity: 'Running the dev server' }));

    expect(label).toContain('4');
    expect(label).toContain('Running the dev server');
    expect(label).not.toContain('npm run dev');
  });

  it('labels a command whose start call said nothing by its first line', () => {
    const label = commandLabel(command(4));

    expect(label).toContain('4');
    expect(label).toContain('npm run dev …');
  });
});

describe('the panel argument', () => {
  it('names the conversation and the number, and reads them back', () => {
    const argument = commandPanelArgument(CONVERSATION, 12);

    expect(parseCommandPanelArgument(argument)).toEqual({ conversationId: CONVERSATION, number: 12 });
  });

  it('refuses an argument that names no command', () => {
    for (const argument of ['', CONVERSATION, `${CONVERSATION}.`, `${CONVERSATION}.0`, `${CONVERSATION}.x`, '.3']) {
      expect(parseCommandPanelArgument(argument)).toBeNull();
    }
  });
});

describe('lists', () => {
  it('puts the commands that run before the ones that ended', () => {
    const split = runningFirst([
      command(1, { ending: 'exited' }),
      command(2),
      command(3, { ending: 'stopped_by_owner' }),
    ]);

    expect(split.running.map((entry) => entry.number)).toEqual([2]);
    expect(split.ended.map((entry) => entry.number)).toEqual([1, 3]);
  });

  it('finds the command that a call started by the call’s part', () => {
    const commands = [command(1), command(2)];

    expect(startedCommand(commands, commands[1]?.partId ?? '')?.number).toBe(2);
    expect(startedCommand(commands, 'another part')).toBeUndefined();
  });
});

describe('noteTitle', () => {
  it('names one command, or several in a list', () => {
    const end = { command: 'make', ending: 'exited' as const, exitCode: 0, number: 3, signal: null, unread: 0 };

    const one = noteTitle({ ends: [end] });
    const several = noteTitle({ ends: [end, { ...end, number: 5 }] });

    expect(one).toContain('3');
    expect(several).toContain('3');
    expect(several).toContain('5');
    expect(several).not.toBe(one);
  });
});
