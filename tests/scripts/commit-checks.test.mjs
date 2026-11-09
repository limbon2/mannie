// @ts-check
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { commitChecks, failedJobs, pushRun, stateOf } from '../../scripts/ci/commit-checks.mjs';

/** @typedef {import('../../scripts/ci/commit-checks.mjs').WorkflowRun} WorkflowRun */

const RELEASE = { commit: 'abc123', isTagPush: true, repository: 'owner/mannie' };
const RUNS = '/repos/owner/mannie/actions/workflows/ci.yml/runs?head_sha=abc123&event=push&per_page=20';
const RUN = '/repos/owner/mannie/actions/runs/7';
const JOBS = `${RUN}/jobs?filter=latest&per_page=100`;
const WAITS = { appearMillis: 90_000, failedReads: 2, lookMillis: 10_000, pollMillis: 30_000 };

/**
 * A run of the CI workflow.
 * @param {Partial<WorkflowRun>} [fields]
 * @returns {WorkflowRun}
 */
function runOf(fields = {}) {
  return {
    conclusion: null,
    event: 'push',
    html_url: 'https://github.example/runs/7',
    id: 7,
    run_number: 70,
    status: 'in_progress',
    ...fields,
  };
}

/**
 * Surroundings whose API answers each path with the next of its answers, an
 * `Error` being thrown, and whose clock moves only when the script sleeps.
 * @param {Record<string, unknown[]>} answers
 */
function surroundings(answers) {
  let time = 0;
  /** @type {string[]} */
  const written = [];
  /** @type {string[]} */
  const requested = [];
  /** @type {number[]} */
  const slept = [];
  return {
    /** @param {string} path */
    get: (path) => {
      requested.push(path);
      const queue = answers[path] ?? [];
      const answer = queue.length > 1 ? queue.shift() : queue[0];
      if (answer === undefined) {
        return Promise.reject(new Error(`no answer for ${path}`));
      }
      return answer instanceof Error ? Promise.reject(answer) : Promise.resolve(answer);
    },
    now: () => time,
    requested,
    /** @param {number} milliseconds */
    sleep: (milliseconds) => {
      time += milliseconds;
      slept.push(milliseconds);
      return Promise.resolve();
    },
    slept,
    /** @param {string} text */
    write: (text) => {
      written.push(text);
    },
    written,
  };
}

describe('pushRun', () => {
  it('takes the newest run that a push started', () => {
    const runs = [
      runOf({ id: 1, run_number: 10 }),
      runOf({ event: 'workflow_dispatch', id: 3, run_number: 30 }),
      runOf({ id: 2, run_number: 20 }),
    ];
    assert.equal(pushRun(runs)?.id, 2);
  });

  it('finds none among manual and pull request runs', () => {
    assert.equal(pushRun([runOf({ event: 'workflow_dispatch' }), runOf({ event: 'pull_request' })]), undefined);
    assert.equal(pushRun([]), undefined);
  });
});

describe('stateOf', () => {
  it('waits for a run that has not ended', () => {
    for (const status of ['queued', 'in_progress', 'waiting', 'requested', 'pending']) {
      assert.equal(stateOf(runOf({ status })), 'waiting', status);
    }
  });

  it('takes the result of a run that ended with a result', () => {
    assert.equal(stateOf(runOf({ conclusion: 'success', status: 'completed' })), 'passed');
    for (const conclusion of ['failure', 'timed_out', 'startup_failure']) {
      assert.equal(stateOf(runOf({ conclusion, status: 'completed' })), 'failed', conclusion);
    }
  });

  it('counts a run that checked nothing as no run', () => {
    for (const conclusion of ['cancelled', 'skipped', 'stale', 'neutral', 'action_required', null]) {
      assert.equal(stateOf(runOf({ conclusion, status: 'completed' })), 'none', String(conclusion));
    }
    assert.equal(stateOf(undefined), 'none');
  });
});

describe('failedJobs', () => {
  it('names the jobs that did not pass', () => {
    const lines = failedJobs([
      { conclusion: 'success', html_url: 'https://github.example/jobs/1', name: 'task ci -- static (ubuntu-24.04)' },
      { conclusion: 'failure', html_url: 'https://github.example/jobs/2', name: 'task ci -- core (windows-2025)' },
      { conclusion: 'cancelled', html_url: 'https://github.example/jobs/3', name: 'End-to-end tests (windows-2025)' },
      { conclusion: 'skipped', html_url: 'https://github.example/jobs/4', name: 'Selected Rust test' },
    ]);
    assert.deepEqual(lines, [
      '  failure  task ci -- core (windows-2025)  https://github.example/jobs/2',
      '  cancelled  End-to-end tests (windows-2025)  https://github.example/jobs/3',
    ]);
  });
});

describe('commitChecks', () => {
  it('takes the result of a run that already passed, without waiting', async () => {
    const around = surroundings({
      [RUNS]: [{ workflow_runs: [runOf({ conclusion: 'success', status: 'completed' })] }],
    });
    assert.equal(await commitChecks(RELEASE, around, WAITS), 'passed');
    assert.deepEqual(around.slept, []);
    assert.deepEqual(around.requested, [RUNS]);
  });

  it('waits for a run in progress and takes its result', async () => {
    const around = surroundings({
      [RUN]: [runOf(), runOf({ conclusion: 'success', status: 'completed' })],
      [RUNS]: [{ workflow_runs: [runOf({ status: 'queued' })] }],
    });
    assert.equal(await commitChecks(RELEASE, around, WAITS), 'passed');
    assert.deepEqual(around.slept, [30_000, 30_000]);
  });

  it('waits for the run of a tag that was pushed with its commit to appear', async () => {
    const around = surroundings({
      [RUNS]: [
        { workflow_runs: [] },
        { workflow_runs: [] },
        { workflow_runs: [runOf({ conclusion: 'success', status: 'completed' })] },
      ],
    });
    assert.equal(await commitChecks(RELEASE, around, WAITS), 'passed');
    assert.deepEqual(around.slept, [10_000, 10_000]);
  });

  it('lets the release run the checks when no push started CI for the commit', async () => {
    const around = surroundings({ [RUNS]: [{ workflow_runs: [runOf({ event: 'workflow_dispatch' })] }] });
    assert.equal(await commitChecks(RELEASE, around, WAITS), 'none');
    // Nine looks of ten seconds fill the time in which a run may appear.
    assert.equal(around.slept.length, 9);
    assert.match(around.written.join('\n'), /no push started CI for abc123/u);
  });

  it('does not wait for a run to appear in a manual release', async () => {
    const around = surroundings({ [RUNS]: [{ workflow_runs: [] }] });
    assert.equal(await commitChecks({ ...RELEASE, isTagPush: false }, around, WAITS), 'none');
    assert.deepEqual(around.slept, []);
  });

  it('lets the release run the checks when the run was cancelled, before or while it waited', async () => {
    const cancelled = runOf({ conclusion: 'cancelled', status: 'completed' });
    const before = surroundings({ [RUNS]: [{ workflow_runs: [cancelled] }] });
    assert.equal(await commitChecks(RELEASE, before, WAITS), 'none');

    const during = surroundings({ [RUN]: [cancelled], [RUNS]: [{ workflow_runs: [runOf()] }] });
    assert.equal(await commitChecks(RELEASE, during, WAITS), 'none');
    assert.match(during.written.join('\n'), /ended as cancelled and checked nothing/u);
  });

  it('fails with the jobs that failed', async () => {
    const around = surroundings({
      [JOBS]: [
        {
          jobs: [
            {
              conclusion: 'failure',
              html_url: 'https://github.example/jobs/2',
              name: 'task ci -- static (ubuntu-24.04)',
            },
            {
              conclusion: 'success',
              html_url: 'https://github.example/jobs/3',
              name: 'task ci -- core (ubuntu-24.04)',
            },
          ],
        },
      ],
      [RUNS]: [{ workflow_runs: [runOf({ conclusion: 'failure', status: 'completed' })] }],
    });
    assert.equal(await commitChecks(RELEASE, around, WAITS), 'failed');
    const written = around.written.join('\n');
    assert.match(written, /failure {2}task ci -- static \(ubuntu-24\.04\) {2}https:\/\/github\.example\/jobs\/2/u);
    assert.doesNotMatch(written, /task ci -- core/u);
    assert.match(written, /re-run the failed jobs of that run, or push a fix, then re-run this job/u);
  });

  it('takes the new result after the failed jobs of the run ran again', async () => {
    // A run whose failed jobs run again keeps its number and is in progress again.
    const around = surroundings({
      [RUN]: [runOf({ conclusion: 'success', status: 'completed' })],
      [RUNS]: [{ workflow_runs: [runOf()] }],
    });
    assert.equal(await commitChecks(RELEASE, around, WAITS), 'passed');
  });

  it('lets the release run the checks when the API fails before a run is found', async () => {
    const around = surroundings({ [RUNS]: [new Error('GitHub answered 502')] });
    assert.equal(await commitChecks(RELEASE, around, WAITS), 'none');
    assert.match(around.written.join('\n'), /could not look for the commit's CI run: GitHub answered 502/u);
  });

  it('keeps waiting through a failed read, and fails when the result stays unknown', async () => {
    const recovered = surroundings({
      [RUN]: [new Error('GitHub answered 502'), runOf({ conclusion: 'success', status: 'completed' })],
      [RUNS]: [{ workflow_runs: [runOf()] }],
    });
    assert.equal(await commitChecks(RELEASE, recovered, WAITS), 'passed');

    const unknown = surroundings({
      [RUN]: [new Error('GitHub answered 502')],
      [RUNS]: [{ workflow_runs: [runOf()] }],
    });
    assert.equal(await commitChecks(RELEASE, unknown, WAITS), 'failed');
    assert.equal(unknown.slept.length, 2);
    assert.match(unknown.written.join('\n'), /the result of the run is unknown/u);
  });
});
