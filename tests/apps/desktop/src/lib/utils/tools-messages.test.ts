// Every code of the system tools feature has a message of its own (ADR-0019),
// and the messages name the paths and commands they are about.
import { describe, expect, it } from 'vitest';

import type {
  AccessMode,
  ApprovalReason,
  Decision,
  GrantScope,
  Interruption,
  NotRunReason,
  PathRefusal,
  ProgramEffect,
  ProgramEffectKey,
  Refusal,
  ToolFailure,
  ToolsError,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  accessModeLabel,
  approvalReasonMessage,
  decisionMessage,
  effectKeyPhrase,
  effectKeysText,
  entryCountMessage,
  formatBytes,
  grantScopeMessage,
  notRunMessage,
  pathRefusalMessage,
  programEffectItems,
  programEffectMessage,
  programRunText,
  refusalMessage,
  toolFailureMessage,
  toolsFailureMessage,
  uncertainMessage,
} from '../../../../../../apps/desktop/src/lib/utils/tools-messages.js';

const TOOLS_ERRORS: ToolsError[] = [
  { code: 'conversation_not_found' },
  { code: 'approval_not_pending' },
  { code: 'approval_changed' },
  { code: 'rule_not_possible' },
  { code: 'rule_prefix_invalid' },
  { code: 'not_in_a_project' },
  { code: 'grant_not_possible' },
  { code: 'rule_not_found' },
  { code: 'grant_not_found' },
  { code: 'output_not_found' },
  { code: 'environment_name_invalid', name: 'BAD=NAME' },
  { code: 'environment_name_secret', name: 'GITHUB_TOKEN' },
  { code: 'too_many_environment_names', max: 50 },
  { code: 'written_by_newer_version' },
  { code: 'storage_failed' },
  { code: 'store_unavailable' },
  { code: 'shutting_down' },
  { code: 'worker_unreachable' },
  { code: 'task_target_unreachable', kind: 'workers', target: 'build-01' },
  { code: 'plugin_off', plugin: 'workers' },
  { code: 'full_access_follows_parent' },
  { code: 'full_access_refused' },
];

const TOOL_FAILURES: ToolFailure[] = [
  { code: 'no_workspace' },
  { code: 'path_refused', reason: 'reserved_name' },
  { code: 'path_through_link', resolved: 'D:\\Real\\notes.md' },
  { code: 'not_found', path: 'D:\\Projects\\app\\missing.txt' },
  { code: 'not_a_file', path: 'D:\\Projects\\app' },
  { code: 'not_a_folder', path: 'D:\\Projects\\app\\main.rs' },
  { code: 'already_exists', path: 'D:\\Projects\\app\\main.rs' },
  { code: 'project_not_found', project: 'elsewhere' },
  { code: 'already_in_project', name: 'mannie' },
  { code: 'permission_denied', path: 'C:\\Windows\\System32' },
  { code: 'in_use', path: 'D:\\Projects\\app\\locked.db' },
  { code: 'revision_mismatch', current: 'sha256:00' },
  { code: 'edit_not_found', edit: 2 },
  { code: 'edit_ambiguous', edit: 1, matches: 3 },
  { code: 'edits_overlap', first: 1, second: 2 },
  { code: 'too_large', limitBytes: 1_048_576 },
  { code: 'binary_file', path: 'D:\\Projects\\app\\logo.png' },
  { code: 'unsupported_encoding', path: 'D:\\Projects\\app\\latin1.txt' },
  { code: 'link_in_source', path: 'D:\\Projects\\app\\link' },
  { code: 'different_volume' },
  { code: 'folder_not_empty', path: 'D:\\Projects\\app\\build' },
  { code: 'too_many_entries', limit: 10_000 },
  { code: 'invalid_pattern', reason: 'unclosed character class' },
  { code: 'output_not_found' },
  { code: 'path_changed' },
  { code: 'shell_missing', path: 'C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe' },
  { code: 'spawn_failed', reason: 'The system cannot find the file specified.' },
  { code: 'io_error', kind: 'disk full' },
  { code: 'plugin_invalid', plugin: 'ssh' },
];

const PATH_REFUSALS: PathRefusal[] = [
  'empty',
  'network_path',
  'device_path',
  'alternate_stream',
  'reserved_name',
  'trailing_dot_or_space',
  'drive_relative',
  'too_long',
  'not_absolute',
];

const NOT_RUN_REASONS: NotRunReason[] = [
  { reason: 'unknown_tool' },
  { detail: 'missing field `path`', reason: 'invalid_arguments' },
  { reason: 'cut_off' },
  { reason: 'earlier_call_denied' },
  { reason: 'over_budget' },
  { reason: 'tools_off' },
  { plugin: 'workers', reason: 'plugin_off' },
];

const GRANTS: GrantScope[] = [
  { folder: 'D:\\Projects\\app', kind: 'path', mode: 'read' },
  { folder: 'D:\\Projects\\app', kind: 'path', mode: 'write' },
  { folder: 'D:\\Projects\\app', kind: 'path', mode: 'create' },
  { folder: 'D:\\Projects\\app', kind: 'path', mode: 'delete' },
  { command: 'cargo test', kind: 'command', workingDirectory: 'D:\\Projects\\app' },
  {
    command: 'uptime',
    endpoint: 'deploy@build-box.lan:22',
    kind: 'remote_command',
    label: 'build-box',
    plugin: 'ssh',
    target: '0199e0f2-0000-7000-8000-0000000000aa',
  },
  {
    arguments: ['push', 'origin', 'main'],
    effects: [{ effect: 'network', endpoint: 'https://example.com/repo.git' }],
    fixed: [],
    kind: 'program',
    name: 'git',
    plugin: 'git',
    program: '/usr/bin/git',
    workingDirectory: 'D:\\Projects\\app',
  },
];

const REFUSALS: Refusal[] = ['no_projects_on_host', 'no_remote_commands', 'no_programs_on_host', 'read_only_host'];

const EFFECTS: ProgramEffect[] = [
  { effect: 'network', endpoint: 'https://example.com/repo.git' },
  { effect: 'runs_folder_code', name: 'pre-commit', source: '.git/hooks/pre-commit' },
  { count: 3, effect: 'discards_work', items: ['a.txt'], snapshot: true, work: 'uncommitted_changes' },
  { count: 1, effect: 'discards_work', items: ['stash@{0}'], snapshot: false, work: 'stash_entries' },
  { count: 2, effect: 'discards_work', items: [], snapshot: false, work: 'commits' },
  { count: 2, effect: 'rewrites_published', places: ['origin/main'] },
];

const EFFECT_KEYS: ProgramEffectKey[] = [
  { effect: 'network', endpoint: 'https://example.com/repo.git' },
  { effect: 'runs_folder_code', name: 'pre-commit', source: '.git/hooks/pre-commit' },
  { effect: 'discards_work', work: 'uncommitted_changes' },
  { effect: 'discards_work', work: 'stash_entries' },
  { effect: 'discards_work', work: 'commits' },
  { effect: 'rewrites_published' },
];

const INTERRUPTIONS: (Interruption | null)[] = [null, { reason: 'stopped' }, { plugin: 'ssh', reason: 'plugin_off' }];

function expectDistinct(messages: string[]): void {
  for (const message of messages) {
    expect(message.trim()).not.toBe('');
  }
  expect(new Set(messages).size).toBe(messages.length);
}

describe('tools messages', () => {
  it('say what each error of the feature’s commands means', () => {
    expectDistinct(TOOLS_ERRORS.map((error) => toolsFailureMessage({ error, kind: 'core' })));
    expect(toolsFailureMessage({ kind: 'ipc' })).not.toBe('');
    expect(
      toolsFailureMessage({ error: { code: 'environment_name_secret', name: 'API_KEY' }, kind: 'core' }),
    ).toContain('API_KEY');
    expect(
      toolsFailureMessage({
        error: { code: 'task_target_unreachable', kind: 'workers', target: 'build-01' },
        kind: 'core',
      }),
    ).toContain('build-01');
  });

  it('say what each failure of a tool call means, and name its path', () => {
    expectDistinct(TOOL_FAILURES.map((failure) => toolFailureMessage(failure)));
    expect(toolFailureMessage({ code: 'not_found', path: 'D:\\Projects\\app\\missing.txt' })).toContain(
      'D:\\Projects\\app\\missing.txt',
    );
    expect(toolFailureMessage({ code: 'path_through_link', resolved: 'D:\\Real\\notes.md' })).toContain(
      'D:\\Real\\notes.md',
    );
  });

  it('leave out the English text that the system and parsers report', () => {
    expect(toolFailureMessage({ code: 'io_error', kind: 'disk full' })).not.toContain('disk full');
    expect(toolFailureMessage({ code: 'spawn_failed', reason: 'not found' })).not.toContain('not found');
    expect(toolFailureMessage({ code: 'invalid_pattern', reason: 'unclosed' })).not.toContain('unclosed');
  });

  it('explain every refused path and every call that never ran', () => {
    expectDistinct(PATH_REFUSALS.map((reason) => pathRefusalMessage(reason)));
    expectDistinct(NOT_RUN_REASONS.map((reason) => notRunMessage(reason)));
    expect(toolFailureMessage({ code: 'path_refused', reason: 'too_long' })).toContain(pathRefusalMessage('too_long'));
  });

  it('explain everything the ceiling of a computer that runs a task refuses', () => {
    expectDistinct(REFUSALS.map((refusal) => refusalMessage(refusal)));
    expect(notRunMessage({ reason: 'refused', refusal: 'no_remote_commands' })).toBe(
      refusalMessage('no_remote_commands'),
    );
  });

  it('say why a call that was executing may have taken effect, and name the plugin that went off', () => {
    expectDistinct(INTERRUPTIONS.map((interruption) => uncertainMessage(interruption)));
    expect(uncertainMessage({ plugin: 'ssh', reason: 'plugin_off' })).toContain('ssh');
    expect(toolFailureMessage({ code: 'plugin_invalid', plugin: 'ssh' })).toContain('ssh');
  });

  it('describe every reason to ask, decision, access mode and grant', () => {
    const reasons: ApprovalReason[] = [
      'outside_workspace',
      'change_in_workspace',
      'mannie_folder',
      'command',
      'no_workspace',
      'move_to_project',
      'remote_command',
      'program',
    ];
    const decisions: Decision[] = [
      'workspace',
      'grant',
      'rule',
      'allowed_once',
      'allowed_for_conversation',
      'rule_saved',
      'denied',
      'project_grant',
      'always_grant',
      'allowed_for_project',
      'allowed_always',
      'owner_named_project',
      'allowed_folder',
      'host_rule',
      'full_access',
      'host_full_access',
    ];
    const modes: AccessMode[] = ['read', 'write', 'create', 'delete'];

    expectDistinct(reasons.map((reason) => approvalReasonMessage(reason)));
    expectDistinct(decisions.map((decision) => decisionMessage(decision)));
    expectDistinct(modes.map((mode) => accessModeLabel(mode)));
    expectDistinct(GRANTS.map((grant) => grantScopeMessage(grant)));
    expect(
      grantScopeMessage({ command: 'cargo test', kind: 'command', workingDirectory: 'D:\\Projects\\app' }),
    ).toContain('cargo test');
  });

  it('name the command, the host and its address of a grant on a remote host', () => {
    const message = grantScopeMessage({
      command: 'systemctl status nginx',
      endpoint: 'deploy@build-box.lan:22',
      kind: 'remote_command',
      label: 'build-box',
      plugin: 'ssh',
      target: '0199e0f2-0000-7000-8000-0000000000aa',
    });

    expect(message).toContain('systemctl status nginx');
    expect(message).toContain('build-box');
    expect(message).toContain('deploy@build-box.lan:22');
    expect(message).not.toContain('0199e0f2');
  });

  it('show a program run’s words with quotes only where a word needs them', () => {
    expect(programRunText(['git', 'commit', '-m', 'Fix the parser', ''])).toBe('git commit -m "Fix the parser" ""');
    expect(programRunText(['git', 'log', '--format=%H'])).toBe('git log --format=%H');
    expect(programRunText(['git', 'show', 'say "hi"'])).toBe(String.raw`git show "say \"hi\""`);
  });

  it('describe every effect of a program’s run, with what it names', () => {
    expectDistinct(EFFECTS.map((effect) => programEffectMessage(effect)));
    expectDistinct(EFFECT_KEYS.map((key) => effectKeyPhrase(key)));
    expect(programEffectMessage(EFFECTS[1])).toContain('.git/hooks/pre-commit');
    expect(programEffectMessage(EFFECTS[0])).toContain('https://example.com/repo.git');
    expect(programEffectMessage(EFFECTS[5])).toContain('origin/main');
    expect(programEffectMessage(EFFECTS[2])).toMatch(/3/u);
    // The first names of what a run discards, and how many more there are.
    const listed = programEffectItems(EFFECTS[2]);
    expect(listed).toContain('a.txt');
    expect(listed).toMatch(/2/u);
    expect(programEffectItems(EFFECTS[3])).toBe('stash@{0}');
    expect(programEffectItems(EFFECTS[4])).toBeNull();
    expect(programEffectItems(EFFECTS[0])).toBeNull();
  });

  it('name the run, its folder and the effects that a grant of a program covers', () => {
    const run: GrantScope = {
      arguments: ['commit', '-F', '-'],
      effects: [],
      fixed: ['-c', 'gc.auto=0'],
      kind: 'program',
      name: 'git',
      plugin: 'git',
      program: 'C:\\Program Files\\Git\\cmd\\git.exe',
      workingDirectory: 'D:\\Projects\\app',
    };
    const withHook: GrantScope = { ...run, effects: [EFFECT_KEYS[1]] };

    const plain = grantScopeMessage(run);
    const hooked = grantScopeMessage(withHook);

    expect(plain).toContain('git commit -F -');
    expect(plain).toContain('D:\\Projects\\app');
    expect(plain).not.toContain('gc.auto');
    expect(hooked).toContain('pre-commit');
    expect(hooked).toContain(effectKeysText([EFFECT_KEYS[1]]));
    expect(hooked).not.toBe(plain);
  });

  it('count what a recursive delete removes', () => {
    const message = entryCountMessage({ files: 3, folders: 2 });

    expect(message).toMatch(/3/u);
    expect(message).toMatch(/2/u);
  });
});

describe('formatBytes', () => {
  it('names bytes below a kilobyte and larger units above it', () => {
    expect(formatBytes(512)).toMatch(/512/u);
    expect(formatBytes(1_200)).toMatch(/1[.,]2/u);
    expect(formatBytes(3_400_000)).toMatch(/3[.,]4/u);
    expect(formatBytes(1_200)).not.toBe(formatBytes(1_200_000));
  });

  it('stays in the largest unit and never goes below zero', () => {
    expect(formatBytes(5_000_000_000_000)).toMatch(/5[\s\u00A0.,]?000/u);
    expect(formatBytes(-1)).toMatch(/0/u);
  });
});
