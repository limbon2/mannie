// The git plugin's entries in the shell (plugins/git/docs/README.md;
// ADR-0030, decision 11; ADR-0039): the cards of its tools, the Git side
// panel, its button in a conversation's toolbar and its kind of mentions show
// only while the plugin is on, and a card describes only the calls of the
// plugin's own tools.
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { describe, expect, it } from 'vitest';

import plugin from '../../ui/index.js';

/** The plugin's tools, as its Rust code names them. */
const TOOLS = [
  'git_blame',
  'git_branch',
  'git_cherry_pick',
  'git_commit',
  'git_conflicts',
  'git_diff',
  'git_edit_history',
  'git_fetch',
  'git_grep',
  'git_log',
  'git_merge',
  'git_operation',
  'git_push',
  'git_rebase',
  'git_refs',
  'git_reset',
  'git_resolve',
  'git_restore',
  'git_revert',
  'git_show',
  'git_stage',
  'git_stash',
  'git_status',
  'git_switch',
];

describe('the git plugin', () => {
  it('registers its entries, which show only while it is on', () => {
    const context = new TestPluginContext('git');

    plugin.register(context);
    const { shell } = context;
    const whileOff = [
      shell.toolKinds.entries,
      shell.sidePanels.entries,
      shell.conversationControls.entries,
      shell.mentionKinds.entries,
    ];
    context.turnOn();

    expect(whileOff).toEqual([[], [], [], []]);
    expect(shell.toolKinds.entries.map((kind) => kind.id).sort()).toEqual(TOOLS);
    expect(shell.sidePanels.entries.map((kind) => kind.id)).toEqual(['git.repository']);
    expect(shell.conversationControls.entries.map((control) => control.id)).toEqual(['git.control']);
    expect(shell.mentionKinds.entries.map((kind) => kind.id)).toEqual(['git']);
  });

  it('describes the calls of its own tools only', () => {
    const context = new TestPluginContext('git', true);
    plugin.register(context);

    expect(context.shell.toolKinds.findOf('git_commit', 'git')).toBeDefined();
    expect(context.shell.toolKinds.findOf('git_commit', 'ssh')).toBeUndefined();
  });

  it('gives each tool a line, and words its failure codes', () => {
    const context = new TestPluginContext('git', true);
    plugin.register(context);
    const commit = context.shell.toolKinds.findOf('git_commit', 'git');
    const status = context.shell.toolKinds.findOf('git_status', 'git');

    expect(commit?.failure('repository_changed')).toMatch(/changed/u);
    expect(commit?.failure('something_else')).toBeNull();
    expect(
      status?.summary({ arguments: {}, data: { branch: { head: 'main' } }, failure: null, status: 'completed' }),
    ).toBe('Read the status of main');
    expect(commit?.details).toBeDefined();
    expect(context.shell.toolKinds.findOf('git_reset', 'git')?.details).toBeUndefined();
  });

  it('words its mentions', () => {
    const context = new TestPluginContext('git', true);
    plugin.register(context);
    const [kind] = context.shell.mentionKinds.entries;

    expect(kind.group()).toBe('Git');
    expect(kind.variantName('commit')).toBe('Commit');
    expect(kind.warning('anything')).toBeNull();
    expect(kind.icon('branch')).not.toBe(kind.icon('commit'));
    expect(kind.icon('changes')).toBe(kind.icon('unknown'));
    expect(kind.refusal('not_a_repository')).not.toBeNull();
  });

  it('has no start failure or work in progress of its own', () => {
    expect(plugin.failure).toBeUndefined();
    expect(plugin.offQuestion).toBeUndefined();
  });
});
