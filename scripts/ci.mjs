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
// The steps belong to groups, and a run that names groups runs only their
// steps and the steps that those need. CI runs each group as a job of its
// own, side by side on a runner each (.github/workflows/ci.yml), so the
// slowest group sets the time of the checks, and a group that fails names
// the command that repeats it.
//
// Usage: node scripts/ci.mjs [group...] (through `task ci` and `task ci -- <group>...`)

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
 * The steps of `task ci`. The two halves of `task gen` come first, because
 * most checks read what they write: the launcher, which takes seconds, and
 * the bindings, whose programs build first. The steps without a lane are
 * listed with the longest first, so the short ones fill in at the end.
 * @type {Step[]}
 */
export const STEPS = [
  { task: 'gen:launcher' },
  { lanes: ['build'], needs: ['gen:launcher'], task: 'gen:bindings' },
  { lanes: ['build'], needs: ['gen:bindings'], task: 'test:rust:build' },
  { lanes: ['clippy'], needs: ['gen:bindings'], task: 'lint:rust' },
  { needs: ['gen:bindings'], task: 'lint:ts' },
  { needs: ['gen:bindings'], task: 'lint:types' },
  { needs: ['gen:bindings'], task: 'lint:messages' },
  // Vitest's Paraglide plugin compiles the messages again and rewrites their
  // files, which ESLint, svelte-check and the message check read, so it runs
  // after them. Cold message evaluation timed out on Windows while Cargo
  // compiled. Keep both test suites out of the compiler lanes without
  // changing timeouts.
  {
    after: ['lint:ts', 'lint:types', 'lint:messages'],
    lanes: ['build', 'clippy'],
    needs: ['gen:bindings'],
    task: 'test:desktop',
  },
  // Wait for desktop transforms and tests to finish before timing Rust
  // behavior. These steps still run after a failed build or desktop suite,
  // and a step builds its own tests where `test:rust:build` has not. Only
  // the desktop binary needs what `task gen` writes.
  { after: ['test:rust:build', 'test:desktop'], lanes: ['build', 'clippy'], task: 'test:rust:core' },
  { after: ['test:rust:build', 'test:desktop'], lanes: ['build', 'clippy'], task: 'test:rust:plugins' },
  {
    after: ['test:rust:build', 'test:desktop'],
    lanes: ['build', 'clippy'],
    needs: ['gen:bindings'],
    task: 'test:rust:desktop',
  },
  // cargo-deny and rustfmt read the launcher and its lock, not the bindings.
  { needs: ['gen:launcher'], task: 'lint:deps' },
  { needs: ['gen:launcher'], task: 'fmt:check' },
  { task: 'docs:lint' },
  { task: 'test:scripts' },
  { task: 'lint:proto' },
  { task: 'lint:toml' },
];

/**
 * The groups of steps, by name. Every check belongs to exactly one group, and
 * a group also runs the steps that its own need, such as the halves of `task
 * gen`. `test:rust:build` belongs to none: it builds every Rust test ahead of
 * the three test steps for a run of all groups, and a group's test step
 * builds its own.
 *
 * - `static` needs no build, so it reports formatting, dependency and
 *   documentation problems within minutes.
 * - `desktop` checks the desktop app's UI and runs the desktop binary's tests.
 * - `clippy` lints the Rust code of every workspace.
 * - `core` and `plugins` run the Rust tests of the root workspace and of the
 *   plugins' workspaces.
 * @type {Record<string, string[]>}
 */
export const GROUPS = {
  clippy: ['lint:rust'],
  core: ['test:rust:core'],
  desktop: ['lint:ts', 'lint:types', 'lint:messages', 'test:desktop', 'test:rust:desktop'],
  plugins: ['test:rust:plugins'],
  static: ['docs:lint', 'test:scripts', 'lint:proto', 'lint:toml', 'fmt:check', 'lint:deps'],
};

/**
 * The steps of the named groups with the steps that they need, in the order
 * of `steps`. A step that waits for a step outside the selection no longer
 * waits for it.
 * @param {Step[]} steps
 * @param {Record<string, string[]>} groups
 * @param {string[]} names the groups to run
 * @returns {Step[]}
 */
export function stepsOfGroups(steps, groups, names) {
  const byTask = new Map(steps.map((step) => [step.task, step]));
  /** @type {Set<string>} */
  const selected = new Set();
  const pending = names.flatMap((name) => groups[name] ?? []);
  while (pending.length > 0) {
    const task = /** @type {string} */ (pending.pop());
    if (!selected.has(task)) {
      selected.add(task);
      pending.push(...(byTask.get(task)?.needs ?? []));
    }
  }
  return steps
    .filter((step) => selected.has(step.task))
    .map((step) => {
      const after = (step.after ?? []).filter((task) => selected.has(task));
      return step.after === undefined ? step : { ...step, after };
    });
}

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
  const requested = process.argv.slice(2);
  const unknown = requested.filter((name) => !Object.hasOwn(GROUPS, name));
  if (unknown.length > 0) {
    console.error(`ci: no group is named ${unknown.join(' or ')}; the groups are ${Object.keys(GROUPS).join(', ')}`);
    process.exit(2);
  }
  const runsEveryStep = requested.length === 0;
  const steps = runsEveryStep ? STEPS : stepsOfGroups(STEPS, GROUPS, requested);
  const selection = runsEveryStep ? '' : ` of ${requested.join(' and ')}`;
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
    `ci: ${String(steps.length)} steps${selection}; ${lanes} and ${String(limits.otherSteps)} other steps at a time, below normal priority`,
  );
  const started = performance.now();
  const outcomes = await runSteps(steps, {
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
