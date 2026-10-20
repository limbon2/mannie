// @ts-check
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { describe, it } from 'node:test';

import { failures, formatDuration, limitsFor, runSteps, STEPS, summary } from '../../scripts/ci.mjs';

/** @typedef {import('../../scripts/ci.mjs').Step} Step */

const GIBIBYTE = 1024 ** 3;
const root = resolve(import.meta.dirname, '../..');

/** The limits of a large machine with enough free memory. */
const LARGE = { otherSteps: 3, sharedJobs: { build: 12, clippy: 4 }, sharedLanes: false };

/** The limits of a small machine, such as a CI runner. */
const SMALL = { otherSteps: 1, sharedJobs: { build: 3, clippy: 1 }, sharedLanes: true };

/**
 * A runner whose steps end only when a test says so, and that records which
 * steps run at the same time.
 */
function controlledRunner() {
  /** @type {{ environment: Record<string, string>, finish: (code: number) => void, task: string }[]} */
  const started = [];
  /** @type {Set<string>} */
  const running = new Set();
  /** @type {string[][]} */
  const overlaps = [];
  /** @type {import('../../scripts/ci.mjs').Runner} */
  const run = (step, environment) =>
    new Promise((settle) => {
      running.add(step.task);
      overlaps.push([...running]);
      started.push({
        environment,
        finish: (code) => {
          running.delete(step.task);
          settle({ code, output: `output of ${step.task}` });
        },
        task: step.task,
      });
    });
  /** @param {string} task */
  const finish = (task, code = 0) => {
    const entry = started.find((candidate) => candidate.task === task);
    assert.ok(entry, `${task} has not started`);
    entry.finish(code);
  };
  return { finish, overlaps, run, started };
}

/** Lets the scheduler react to the steps that ended. */
const settle = () =>
  new Promise((done) => {
    setImmediate(done);
  });

/**
 * Starts a run with a controlled runner.
 * @param {Step[]} steps
 * @param {import('../../scripts/ci.mjs').Limits} limits
 */
function start(steps, limits = LARGE) {
  const runner = controlledRunner();
  /** @type {string[]} */
  const written = [];
  const outcomes = runSteps(steps, { limits, now: () => 0, run: runner.run, write: (text) => written.push(text) });
  return { ...runner, outcomes, written };
}

describe('limitsFor', () => {
  it('runs two cargo lanes and three other steps on a large machine', () => {
    assert.deepEqual(limitsFor({ freeMemory: 20 * GIBIBYTE, processors: 16, totalMemory: 31 * GIBIBYTE }), LARGE);
  });

  it('runs one other step at a time when memory is short', () => {
    const limits = limitsFor({ freeMemory: 5 * GIBIBYTE, processors: 16, totalMemory: 31 * GIBIBYTE });
    assert.deepEqual(limits, { ...LARGE, otherSteps: 1 });
  });

  it('shares one cargo lane on a small machine', () => {
    const fewProcessors = limitsFor({ freeMemory: 10 * GIBIBYTE, processors: 4, totalMemory: 16 * GIBIBYTE });
    assert.deepEqual(fewProcessors, SMALL);
    const littleMemory = limitsFor({ freeMemory: 6 * GIBIBYTE, processors: 16, totalMemory: 12 * GIBIBYTE });
    assert.equal(littleMemory.sharedLanes, true);
  });
});

describe('runSteps', () => {
  it('starts a step once the steps it needs have passed', async () => {
    const run = start([{ task: 'gen' }, { needs: ['gen'], task: 'lint' }]);
    await settle();
    assert.deepEqual(
      run.started.map((entry) => entry.task),
      ['gen'],
    );
    run.finish('gen');
    await settle();
    assert.deepEqual(
      run.started.map((entry) => entry.task),
      ['gen', 'lint'],
    );
    run.finish('lint');
    const outcomes = await run.outcomes;
    assert.deepEqual(
      outcomes.map((outcome) => outcome.status),
      ['passed', 'passed'],
    );
  });

  it('skips the steps that need a failed step and still runs the others', async () => {
    const run = start([
      { task: 'gen' },
      { needs: ['gen'], task: 'lint' },
      { needs: ['lint'], task: 'report' },
      { task: 'docs' },
    ]);
    await settle();
    run.finish('docs');
    run.finish('gen', 1);
    const outcomes = await run.outcomes;
    assert.deepEqual(outcomes, [
      { milliseconds: 0, output: 'output of gen', reason: 'exit code 1', status: 'failed', task: 'gen' },
      { milliseconds: 0, reason: 'needs gen', status: 'skipped', task: 'lint' },
      { milliseconds: 0, reason: 'needs lint', status: 'skipped', task: 'report' },
      { milliseconds: 0, status: 'passed', task: 'docs' },
    ]);
  });

  it('starts a step that comes after a failed step', async () => {
    const run = start([{ task: 'build' }, { after: ['build'], task: 'tests' }]);
    await settle();
    run.finish('build', 101);
    await settle();
    run.finish('tests');
    const outcomes = await run.outcomes;
    assert.deepEqual(
      outcomes.map((outcome) => outcome.status),
      ['failed', 'passed'],
    );
  });

  it('runs one step of a lane at a time, and the two lanes side by side with a share of the processors each', async () => {
    const run = start([
      { lanes: ['build'], task: 'build-a' },
      { lanes: ['build'], task: 'build-b' },
      { lanes: ['clippy'], task: 'clippy' },
    ]);
    await settle();
    assert.deepEqual(
      run.started.map((entry) => [entry.task, entry.environment]),
      [
        ['build-a', { CARGO_BUILD_JOBS: '12' }],
        ['clippy', { CARGO_BUILD_JOBS: '4' }],
      ],
    );
    run.finish('clippy');
    run.finish('build-a');
    await settle();
    // Alone in the lanes, it gets every processor.
    assert.deepEqual(run.started[2], { ...run.started[2], environment: {}, task: 'build-b' });
    run.finish('build-b');
    await run.outcomes;
  });

  it('runs a step of both lanes with no other cargo step', async () => {
    const run = start([
      { lanes: ['clippy'], task: 'clippy' },
      { lanes: ['build', 'clippy'], task: 'tests' },
      { lanes: ['build'], task: 'build' },
    ]);
    await settle();
    run.finish('clippy');
    await settle();
    assert.deepEqual(
      run.started.map((entry) => entry.task),
      ['clippy', 'build'],
    );
    run.finish('build');
    await settle();
    assert.deepEqual(run.started[2], { ...run.started[2], environment: {}, task: 'tests' });
    run.finish('tests');
    await run.outcomes;
    const isTestsAlone = run.overlaps.every((tasks) => {
      const isAlone = tasks.length === 1;
      return isAlone || !tasks.includes('tests');
    });
    assert.equal(isTestsAlone, true);
  });

  it('runs the cargo steps one at a time on a small machine', async () => {
    const run = start(
      [
        { lanes: ['build'], task: 'build' },
        { lanes: ['clippy'], task: 'clippy' },
      ],
      SMALL,
    );
    await settle();
    assert.deepEqual(run.started, [{ ...run.started[0], environment: {}, task: 'build' }]);
    run.finish('build');
    await settle();
    run.finish('clippy');
    await run.outcomes;
    const overlapping = run.overlaps.filter((tasks) => tasks.length > 1);
    assert.deepEqual(overlapping, []);
  });

  it('limits the steps without a lane', async () => {
    const run = start([{ task: 'a' }, { task: 'b' }, { task: 'c' }, { task: 'd' }], { ...LARGE, otherSteps: 2 });
    await settle();
    assert.equal(run.started.length, 2);
    run.finish('a');
    await settle();
    assert.equal(run.started.length, 3);
    run.finish('b');
    run.finish('c');
    await settle();
    run.finish('d');
    await run.outcomes;
    const mostAtOnce = Math.max(...run.overlaps.map((tasks) => tasks.length));
    assert.equal(mostAtOnce, 2);
  });

  it('starts nothing new after an interruption', async () => {
    let isInterrupted = false;
    const runner = controlledRunner();
    const outcomes = runSteps([{ task: 'gen' }, { needs: ['gen'], task: 'lint' }], {
      isInterrupted: () => isInterrupted,
      limits: LARGE,
      now: () => 0,
      run: runner.run,
      write: () => undefined,
    });
    await settle();
    isInterrupted = true;
    runner.finish('gen');
    const ended = await outcomes;
    assert.deepEqual(
      ended.map((outcome) => [outcome.status, outcome.reason]),
      [
        ['passed', undefined],
        ['skipped', 'interrupted'],
      ],
    );
  });

  it('keeps a failed step out of the stream and returns its output', async () => {
    const run = start([{ task: 'pass' }, { task: 'fail' }]);
    await settle();
    run.finish('pass');
    run.finish('fail', 2);
    const outcomes = await run.outcomes;
    const stream = run.written.join('');
    assert.match(stream, /^ci: pass passed in 0 s\noutput of pass\n/mu);
    assert.match(stream, /^ci: fail failed in 0 s$/mu);
    assert.doesNotMatch(stream, /output of fail/u);
    assert.equal(outcomes[1].output, 'output of fail');
  });
});

describe('formatDuration', () => {
  it('writes seconds, then minutes and seconds', () => {
    assert.equal(formatDuration(400), '0 s');
    assert.equal(formatDuration(45_200), '45 s');
    assert.equal(formatDuration(185_000), '3 min 05 s');
  });
});

describe('failures', () => {
  it('prints the output of each failed step under its name', () => {
    const text = failures([
      { milliseconds: 1000, status: 'passed', task: 'gen' },
      { milliseconds: 1000, output: 'error: bad', reason: 'exit code 1', status: 'failed', task: 'lint:ts' },
      { milliseconds: 0, reason: 'needs lint:ts', status: 'skipped', task: 'later' },
    ]);
    assert.equal(text, 'ci: output of lint:ts, which failed\nerror: bad\n');
  });
});

describe('summary', () => {
  it('lists every step with its status and duration, then the counts', () => {
    const text = summary(
      [
        { milliseconds: 52_000, status: 'passed', task: 'gen' },
        { milliseconds: 72_000, output: '', reason: 'exit code 1', status: 'failed', task: 'lint:ts' },
        { milliseconds: 0, reason: 'needs lint:ts', status: 'skipped', task: 'later' },
      ],
      390_000,
    );
    assert.equal(
      text,
      [
        'ci: summary',
        '  passed   gen            52 s',
        '  failed   lint:ts  1 min 12 s  exit code 1',
        '  skipped  later                needs lint:ts',
        'ci: 1 passed, 1 failed, 1 skipped in 6 min 30 s',
        '',
      ].join('\n'),
    );
  });
});

describe('STEPS', () => {
  it('finishes desktop tests before Rust tests in the actual graph, including a desktop failure', async () => {
    for (const desktopCode of [0, 1]) {
      const run = start(STEPS);
      const finished = new Set();
      await settle();
      // Finish every prerequisite while holding the desktop suite open.
      while (true) {
        const pending = run.started.filter((entry) => {
          const held = entry.task === 'test:desktop' || entry.task === 'test:rust';
          return !held && !finished.has(entry.task);
        });
        if (pending.length === 0) {
          break;
        }
        for (const entry of pending) {
          run.finish(entry.task);
          finished.add(entry.task);
        }
        await settle();
      }
      assert.ok(run.started.some((entry) => entry.task === 'test:desktop'));
      assert.equal(
        run.started.some((entry) => entry.task === 'test:rust'),
        false,
      );
      run.finish('test:desktop', desktopCode);
      finished.add('test:desktop');
      await settle();
      while (true) {
        const pending = run.started.filter((entry) => !finished.has(entry.task));
        if (pending.length === 0) {
          break;
        }
        for (const entry of pending) {
          run.finish(entry.task);
          finished.add(entry.task);
        }
        await settle();
      }
      const outcomes = await run.outcomes;
      assert.equal(outcomes.find((outcome) => outcome.task === 'test:rust')?.status, 'passed');
      const overlap = run.overlaps.some((tasks) => tasks.includes('test:desktop') && tasks.includes('test:rust'));
      assert.equal(overlap, false);
    }
  });

  it('names only steps of the list in needs and after', () => {
    const names = new Set(STEPS.map((step) => step.task));
    const unknown = STEPS.flatMap((step) => [...(step.needs ?? []), ...(step.after ?? [])]).filter(
      (name) => !names.has(name),
    );
    assert.deepEqual(unknown, []);
  });

  it('lists each step once, in an order where every prerequisite comes first', () => {
    /** @type {Set<string>} */
    const seen = new Set();
    for (const step of STEPS) {
      assert.equal(seen.has(step.task), false, `${step.task} is listed twice`);
      const late = [...(step.needs ?? []), ...(step.after ?? [])].filter((name) => !seen.has(name));
      assert.deepEqual(late, [], `${step.task} comes before what it waits for`);
      seen.add(step.task);
    }
  });

  it('runs tasks that Taskfile.yml defines', () => {
    const listing = spawnSync('task', ['--list-all', '--json'], { cwd: root, encoding: 'utf8' });
    assert.equal(listing.status, 0, listing.stderr);
    /** @type {{ tasks: { name: string }[] }} */
    const parsed = JSON.parse(listing.stdout);
    const defined = new Set(parsed.tasks.map((task) => task.name));
    const missing = STEPS.map((step) => step.task).filter((task) => !defined.has(task));
    assert.deepEqual(missing, []);
  });
});
