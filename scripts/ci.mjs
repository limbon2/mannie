// @ts-check
// Runs the checks of `task ci` side by side (ADR-0037). Each step is a task of
// Taskfile.yml, so the commands stay in one place, and this script decides
// only when each one starts:
//
// - A step starts once the steps it needs have passed. A step whose
//   prerequisite failed or was skipped is skipped, and every other step still
//   runs, so one run reports every failure.
// - Cargo lets one command at a time use a target folder. The builds use
//   target/ and Clippy uses target/clippy, so a cargo step holds the lane of
//   its folder, and the two lanes run side by side. Desktop and Rust tests
//   each hold both lanes, so compilation cannot starve a test's timeout.
//   Rust tests start after the desktop tests finish.
// - The machine stays usable. The run has below-normal priority, which every
//   command inherits. Two cargo steps that run side by side share the logical
//   processors, three quarters for the builds and a quarter for Clippy, so
//   they start as many compilers as one cargo build. At most three other
//   steps run at a time, or one when memory is short. A small machine, such as
//   a CI runner, runs one cargo step and one other step at a time.
//
// A step's output is printed in one block when the step passes. A failed
// step's output is printed after all the others, so the failures stand right
// above the summary, which ends the run with every step's status and
// duration.
//
// Usage: node scripts/ci.mjs (through `task ci`)

import { spawn } from 'node:child_process';
import { availableParallelism, constants, freemem, setPriority, totalmem } from 'node:os';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** @typedef {'build' | 'clippy'} Lane */

/**
 * A step of `task ci`.
 * @typedef {object} Step
 * @property {string} task the task of Taskfile.yml that the step runs
 * @property {string[]} [after] the steps that must end, whatever their status, before it starts
 * @property {Lane[]} [lanes] the cargo lanes that it holds while it runs
 * @property {string[]} [needs] the steps that must pass before it starts
 */

/**
 * How much the run does at once.
 * @typedef {object} Limits
 * @property {number} otherSteps how many steps without a cargo lane run at a time
 * @property {Record<Lane, number>} sharedJobs the cargo jobs of a step in each
 *   lane while a step of the other lane runs beside it
 * @property {boolean} sharedLanes whether the two cargo lanes take turns
 */

/**
 * What became of a step.
 * @typedef {object} Outcome
 * @property {number} milliseconds how long it ran, 0 when it was skipped
 * @property {string} [output] a failed step's output, which the run prints
 *   above the summary
 * @property {string} [reason] why it was skipped, or how it failed
 * @property {'failed' | 'passed' | 'skipped'} status
 * @property {string} task
 */

/**
 * Runs a step's task and resolves with its exit code and output. It never
 * rejects: a task that cannot start resolves with a failing code.
 * @callback Runner
 * @param {Step} step
 * @param {Record<string, string>} environment the variables to set for the step
 * @returns {Promise<{ code: number, output: string }>}
 */

/**
 * The steps of `task ci`. `gen` comes first, because most checks read what it
 * writes; the steps without a lane are listed with the longest first, so the
 * short ones fill in at the end.
 * @type {Step[]}
 */
export const STEPS = [
  { lanes: ['build'], task: 'gen' },
  { lanes: ['build'], needs: ['gen'], task: 'test:rust:build' },
  { lanes: ['clippy'], needs: ['gen'], task: 'lint:rust' },
  { needs: ['gen'], task: 'lint:ts' },
  { needs: ['gen'], task: 'lint:types' },
  // Vitest's Paraglide plugin compiles the messages again and rewrites their
  // files, which ESLint and svelte-check read, so it runs after them.
  // Cold message evaluation timed out on Windows while Cargo compiled. Keep
  // both test suites out of the compiler lanes without changing timeouts.
  { after: ['lint:ts', 'lint:types'], lanes: ['build', 'clippy'], needs: ['gen'], task: 'test:desktop' },
  // Wait for desktop transforms and tests to finish before timing Rust
  // behavior. These steps still run after a failed build or desktop suite.
  { after: ['test:rust:build', 'test:desktop'], lanes: ['build', 'clippy'], needs: ['gen'], task: 'test:rust' },
  { needs: ['gen'], task: 'lint:deps' },
  { needs: ['gen'], task: 'fmt:check' },
  { needs: ['gen'], task: 'lint:messages' },
  { task: 'docs:lint' },
  { task: 'test:scripts' },
  { task: 'lint:proto' },
  { task: 'lint:toml' },
];

const GIBIBYTE = 1024 ** 3;

/** The states of a step that has ended. */
const ENDED = new Set(['failed', 'passed', 'skipped']);

/** The states of a step that blocks the steps that need it. */
const UNSUCCESSFUL = new Set(['failed', 'skipped']);

/**
 * The limits for a machine.
 * @param {{ freeMemory: number, processors: number, totalMemory: number }} machine
 * @returns {Limits}
 */
export function limitsFor(machine) {
  // Below this size, such as a CI runner with four processors, two cargo
  // builds at once gain little and can run out of memory.
  const isSmall = machine.processors < 8 || machine.totalMemory < 16 * GIBIBYTE;
  // Memory that other programs hold, such as a virtual machine, leaves room
  // for one check beside the cargo steps.
  const isMemoryShort = machine.freeMemory < 8 * GIBIBYTE;
  const isConstrained = isSmall || isMemoryShort;
  // The builds take longer than Clippy, so they get three quarters of the
  // processors while both run, and together the two start as many compilers
  // as one cargo build.
  const buildJobs = Math.max(1, Math.ceil((machine.processors * 3) / 4));
  return {
    otherSteps: isConstrained ? 1 : 3,
    sharedJobs: { build: buildJobs, clippy: Math.max(1, machine.processors - buildJobs) },
    sharedLanes: isSmall,
  };
}

/**
 * A duration for people, such as `45 s` or `3 min 05 s`.
 * @param {number} milliseconds
 */
export function formatDuration(milliseconds) {
  const seconds = Math.round(milliseconds / 1000);
  if (seconds < 60) {
    return `${String(seconds)} s`;
  }
  const minutes = Math.floor(seconds / 60);
  const rest = String(seconds % 60).padStart(2, '0');
  return `${String(minutes)} min ${rest} s`;
}

/**
 * Text that ends with a line break, or nothing.
 * @param {string} text
 */
function withNewline(text) {
  const isClosed = text === '' || text.endsWith('\n');
  return isClosed ? text : `${text}\n`;
}

/**
 * The failed steps' output, each under a line that names the step.
 * @param {Outcome[]} outcomes
 */
export function failures(outcomes) {
  return outcomes
    .filter((outcome) => outcome.output !== undefined)
    .map((outcome) => `ci: output of ${outcome.task}, which failed\n${withNewline(outcome.output ?? '')}`)
    .join('');
}

/**
 * The table that ends a run: every step's status and duration, then the
 * counts and the total time.
 * @param {Outcome[]} outcomes
 * @param {number} milliseconds the run's time
 */
export function summary(outcomes, milliseconds) {
  const width = Math.max(...outcomes.map((outcome) => outcome.task.length));
  const lines = outcomes.map((outcome) => {
    const isSkipped = outcome.status === 'skipped';
    const duration = isSkipped ? '' : formatDuration(outcome.milliseconds);
    const reason = outcome.reason === undefined ? '' : `  ${outcome.reason}`;
    return `  ${outcome.status.padEnd(7)}  ${outcome.task.padEnd(width)}  ${duration.padStart(10)}${reason}`.trimEnd();
  });
  /** @param {Outcome['status']} status */
  const count = (status) => outcomes.filter((outcome) => outcome.status === status).length;
  const totals = `${String(count('passed'))} passed, ${String(count('failed'))} failed, ${String(count('skipped'))} skipped`;
  return ['ci: summary', ...lines, `ci: ${totals} in ${formatDuration(milliseconds)}`, ''].join('\n');
}

/**
 * Runs the steps side by side within the limits.
 * @param {Step[]} steps
 * @param {{ isInterrupted?: () => boolean, limits: Limits, now: () => number, run: Runner, write: (text: string) => void }} options
 * @returns {Promise<Outcome[]>} the outcomes, in the steps' order
 */
export function runSteps(steps, options) {
  const { limits, now, run, write } = options;
  const isInterrupted = options.isInterrupted ?? (() => false);
  /** @type {Map<string, Outcome['status'] | 'pending' | 'running'>} */
  const states = new Map(steps.map((step) => [step.task, 'pending']));
  /** @type {Map<string, Outcome>} */
  const outcomes = new Map();
  /** @type {Set<string>} */
  const busyLanes = new Set();
  let runningCargoSteps = 0;
  let runningOtherSteps = 0;

  /** @param {string} task */
  const hasEnded = (task) => ENDED.has(states.get(task) ?? 'pending');
  /**
   * @param {Step} step
   * @param {string} reason
   */
  const skip = (step, reason) => {
    states.set(step.task, 'skipped');
    outcomes.set(step.task, { milliseconds: 0, reason, status: 'skipped', task: step.task });
    write(`ci: skipped ${step.task}: ${reason}\n`);
  };
  // A step that needs a step that failed or was skipped cannot pass. Skipping
  // it can block another step, so this repeats until nothing changes.
  const skipBlocked = () => {
    let hasChanged = true;
    while (hasChanged) {
      hasChanged = false;
      for (const step of steps) {
        const isPending = states.get(step.task) === 'pending';
        const blocker = (step.needs ?? []).find((need) => UNSUCCESSFUL.has(states.get(need) ?? 'pending'));
        const isBlocked = blocker !== undefined;
        if (isPending && isBlocked) {
          skip(step, `needs ${blocker}`);
          hasChanged = true;
        }
      }
    }
  };
  // On a small machine both lanes are one, so cargo steps take turns.
  /** @param {Step} step */
  const lanesOf = (step) => {
    const lanes = (step.lanes ?? []).map((lane) => (limits.sharedLanes ? 'cargo' : lane));
    return [...new Set(lanes)];
  };

  return new Promise((finish) => {
    /**
     * @param {Step} step
     * @param {string[]} lanes
     * @param {Record<string, string>} environment
     */
    const launch = (step, lanes, environment) => {
      const started = now();
      write(`ci: started ${step.task}\n`);
      run(step, environment).then(({ code, output }) => {
        const milliseconds = now() - started;
        const isPassed = code === 0;
        /** @type {Outcome} */
        const outcome = isPassed
          ? { milliseconds, status: 'passed', task: step.task }
          : { milliseconds, output, reason: `exit code ${String(code)}`, status: 'failed', task: step.task };
        states.set(step.task, outcome.status);
        outcomes.set(step.task, outcome);
        const shown = isPassed ? withNewline(output) : '';
        write(`ci: ${step.task} ${outcome.status} in ${formatDuration(milliseconds)}\n${shown}`);
        if (lanes.length > 0) {
          for (const lane of lanes) {
            busyLanes.delete(lane);
          }
          runningCargoSteps -= 1;
        } else {
          runningOtherSteps -= 1;
        }
        schedule();
      });
    };

    const schedule = () => {
      if (isInterrupted()) {
        for (const step of steps) {
          const isPending = states.get(step.task) === 'pending';
          if (isPending) {
            skip(step, 'interrupted');
          }
        }
      }
      skipBlocked();
      /** @type {{ lanes: string[], step: Step }[]} */
      const starting = [];
      for (const step of steps) {
        const isPending = states.get(step.task) === 'pending';
        const hasPassedNeeds = (step.needs ?? []).every((need) => states.get(need) === 'passed');
        const hasEndedAfter = (step.after ?? []).every(hasEnded);
        const isReady = isPending && hasPassedNeeds && hasEndedAfter;
        if (!isReady) {
          continue;
        }
        const lanes = lanesOf(step);
        const isCargo = lanes.length > 0;
        const isLaneBusy = lanes.some((lane) => busyLanes.has(lane));
        const isFull = runningOtherSteps >= limits.otherSteps;
        const canStart = isCargo ? !isLaneBusy : !isFull;
        if (!canStart) {
          continue;
        }
        if (isCargo) {
          for (const lane of lanes) {
            busyLanes.add(lane);
          }
          runningCargoSteps += 1;
        } else {
          runningOtherSteps += 1;
        }
        states.set(step.task, 'running');
        starting.push({ lanes, step });
      }
      // Counted after the loop, so two cargo steps that start together both
      // get their share of the processors. A step of both lanes runs alone.
      const isSharingProcessors = runningCargoSteps > 1;
      for (const { lanes, step } of starting) {
        const lane = step.lanes?.length === 1 ? step.lanes[0] : undefined;
        const isShared = lane !== undefined && isSharingProcessors;
        const environment = isShared ? { CARGO_BUILD_JOBS: String(limits.sharedJobs[lane]) } : {};
        launch(step, lanes, environment);
      }
      const isRunning = runningCargoSteps + runningOtherSteps > 0;
      if (!isRunning) {
        // Nothing runs and nothing could start: the remaining steps wait for
        // steps that will never end, which a test of the step list rules out.
        for (const step of steps) {
          const isPending = states.get(step.task) === 'pending';
          if (isPending) {
            skip(step, 'waits for a step that is not in the list');
          }
        }
        finish(steps.map((step) => /** @type {Outcome} */ (outcomes.get(step.task))));
      }
    };

    schedule();
  });
}

/**
 * A runner that runs `task <name>` in the repository's root and collects its
 * output.
 * @param {string} root
 * @param {Record<string, string | undefined>} baseEnvironment
 * @returns {Runner}
 */
export function taskRunner(root, baseEnvironment) {
  return (step, environment) =>
    new Promise((settle) => {
      /** @type {Buffer[]} */
      const chunks = [];
      let hasSettled = false;
      /** @param {number} code */
      const end = (code) => {
        if (!hasSettled) {
          hasSettled = true;
          settle({ code, output: Buffer.concat(chunks).toString('utf8') });
        }
      };
      const child = spawn('task', [step.task], {
        cwd: root,
        env: { ...baseEnvironment, ...environment },
        stdio: ['ignore', 'pipe', 'pipe'],
      });
      child.stdout.on('data', (/** @type {Buffer} */ chunk) => chunks.push(chunk));
      child.stderr.on('data', (/** @type {Buffer} */ chunk) => chunks.push(chunk));
      child.on('error', (error) => {
        chunks.push(Buffer.from(`ci: could not run task ${step.task}: ${error.message}\n`));
        end(1);
      });
      // A task ended by a signal has no code and counts as failed.
      child.on('close', (code) => {
        end(code ?? 1);
      });
    });
}

const isMain = process.argv[1] === fileURLToPath(import.meta.url);
if (isMain) {
  const root = resolve(import.meta.dirname, '..');
  const limits = limitsFor({ freeMemory: freemem(), processors: availableParallelism(), totalMemory: totalmem() });
  try {
    setPriority(constants.priority.PRIORITY_BELOW_NORMAL);
  } catch (error) {
    console.error(`ci: the priority stays normal: ${error instanceof Error ? error.message : String(error)}`);
  }
  // The commands write into pipes, so they keep their colors only when this
  // script writes to a terminal.
  const colors = process.stdout.isTTY ? { CARGO_TERM_COLOR: 'always', FORCE_COLOR: '1' } : {};
  const environment = { ...colors, ...process.env };
  let isInterrupted = false;
  // Ctrl+C reaches every command too. The steps that run end on their own,
  // and no new one starts.
  process.on('SIGINT', () => {
    isInterrupted = true;
  });
  const lanes = limits.sharedLanes ? 'one cargo step' : 'two cargo steps';
  console.log(
    `ci: ${String(STEPS.length)} steps; ${lanes} and ${String(limits.otherSteps)} other steps at a time, below normal priority`,
  );
  const started = performance.now();
  const outcomes = await runSteps(STEPS, {
    isInterrupted: () => isInterrupted,
    limits,
    now: () => performance.now(),
    run: taskRunner(root, environment),
    write: (text) => process.stdout.write(text),
  });
  process.stdout.write(failures(outcomes));
  process.stdout.write(summary(outcomes, performance.now() - started));
  const hasPassed = outcomes.every((outcome) => outcome.status === 'passed');
  process.exitCode = hasPassed ? 0 : 1;
}
