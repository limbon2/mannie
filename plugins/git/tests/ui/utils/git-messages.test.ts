// The words of the git plugin's codes: every code that the mentions, the
// tools and the panel send has its sentence, a code that the plugin does not
// know leaves the core's own words, and git's states read as words.
import { describe, expect, it } from 'vitest';

import type { GitError } from '../../../ui/generated/bindings.js';
import {
  changeWords,
  conflictWords,
  gitFailureMessage,
  gitMentionRefusal,
  gitMentionSummary,
  gitMentionVariantName,
  gitToolFailure,
} from '../../../ui/utils/git-messages.js';

/** The codes that the plugin's git runs fail with, in its mentions and its panel. */
const SHARED_CODES = [
  'bare_repository',
  'dubious_ownership',
  'git_failed',
  'git_missing',
  'git_too_old',
  'locked',
  'not_a_repository',
  'read_not_allowed',
  'timed_out',
];

/** The codes of the panel's commands, as the plugin's error type holds them. */
const PANEL_CODES: readonly GitError['code'][] = [
  'bare_repository',
  'dubious_ownership',
  'git_failed',
  'git_missing',
  'git_too_old',
  'locked',
  'not_a_repository',
  'project_not_found',
  'read_not_allowed',
  'timed_out',
  'unknown_commit',
];

describe('the git mentions', () => {
  it('word every refusal and failure that the provider sends', () => {
    for (const code of [...SHARED_CODES, 'invalid_key', 'unknown_revision']) {
      expect(gitMentionRefusal(code), code).toEqual(expect.any(String));
    }
    expect(gitMentionRefusal('something_else')).toBeNull();
  });

  it('count what the model got of each kind of mention', () => {
    expect(gitMentionSummary({ code: 'changes', values: { files: 1 } })).toBe('status and diff of 1 changed file');
    expect(gitMentionSummary({ code: 'commit', values: { files: 3 } })).toBe('message and patch of 3 files');
    expect(gitMentionSummary({ code: 'branch', values: { commits: 2, files: 5 } })).toBe(
      '2 commits and the files that they change',
    );
    expect(gitMentionSummary({ code: 'not_read', values: {} })).toBe(
      'not read: reading this repository here needs your permission',
    );
    expect(gitMentionSummary({ code: 'other', values: {} })).toBeNull();
  });

  it('name each variant for screen readers', () => {
    expect(gitMentionVariantName('changes')).toBe('Uncommitted changes');
    expect(gitMentionVariantName('branch')).toBe('Branch');
    expect(gitMentionVariantName('folder')).toBeNull();
  });
});

describe('the git tools’ failures', () => {
  it('word every code that a tool’s call fails with', () => {
    const codes = [
      ...SHARED_CODES,
      'identity_missing',
      'invalid_argument',
      'operation_refused',
      'repository_changed',
      'unknown_revision',
    ];

    for (const code of codes) {
      expect(gitToolFailure(code), code).toEqual(expect.any(String));
    }
    expect(gitToolFailure('repository_changed')).toBe(
      'The repository changed after the call was allowed, so nothing ran.',
    );
    expect(gitToolFailure('something_else')).toBeNull();
  });
});

describe('the panel’s failures', () => {
  it('word every error of the plugin, the host’s refusals and a lost answer', () => {
    for (const code of PANEL_CODES) {
      const message = gitFailureMessage({
        error: { code, description: 'x', origin: 'plugin' },
        kind: 'core',
      });
      expect(message, code).not.toBe('');
    }
    expect(gitFailureMessage({ kind: 'ipc' })).toEqual(expect.any(String));
    expect(
      gitFailureMessage({
        error: { code: 'plugin_off', description: 'off', origin: 'host', plugin: 'git' },
        kind: 'core',
      }),
    ).toMatch(/Git/u);
  });
});

describe('the words of git’s states', () => {
  it('word each change letter and each kind of conflict', () => {
    expect(changeWords('M')).toBe('modified');
    expect(changeWords('R')).toBe('renamed');
    expect(changeWords('X')).toBe('changed');
    expect(conflictWords('both_changed')).toBe('both changed');
    expect(conflictWords('deleted_incoming')).toBe('deleted on the incoming side');
  });
});
