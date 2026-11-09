// @ts-check
// The checks that a release's commit already has (docs/engineering/releasing.md,
// steps). The push of a commit to main starts the CI workflow for it, and a
// release tag is pushed with its commit or after it, so the Release workflow
// would run the same jobs a second time, beside the first. This script lets
// the release take the result of the push's run instead:
//
// - The commit has a run that a push started: the script waits for it. It
//   writes `result=passed` to $GITHUB_OUTPUT when the run passed, and fails
//   with the jobs that failed when it did not.
// - The commit has no such run, or its run was cancelled, as a later push to
//   main does to it: the script writes `result=none`, and the release runs
//   the checks itself.
//
// A problem with GitHub's API before a run is found also gives `none`, so
// the release then runs its own checks, as it always did.
//
// Usage: node scripts/ci/commit-checks.mjs <commit>
//        (in release.yml, with GITHUB_TOKEN and GitHub's variables)

import { appendFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/**
 * A run of a workflow, as GitHub's REST API returns it.
 * @typedef {object} WorkflowRun
 * @property {string | null} conclusion
 * @property {string} event
 * @property {string} html_url
 * @property {number} id
 * @property {number} run_number
 * @property {string} status
 */

/**
 * A job of a run, as GitHub's REST API returns it.
 * @typedef {object} WorkflowJob
 * @property {string | null} conclusion
 * @property {string} html_url
 * @property {string} name
 */

/**
 * What the release does about a run: wait for it, take its result, or run
 * the checks itself.
 * @typedef {'failed' | 'none' | 'passed' | 'waiting'} RunState
 */

/**
 * What the script needs from its surroundings, which the tests replace.
 * @typedef {object} Surroundings
 * @property {(path: string) => Promise<unknown>} get reads a path of GitHub's API as JSON
 * @property {() => number} now the time in milliseconds
 * @property {(milliseconds: number) => Promise<void>} sleep
 * @property {(text: string) => void} write prints a line
 */

/**
 * How the script waits.
 * @typedef {object} Waits
 * @property {number} appearMillis how long a run may take to appear after a tag's push
 * @property {number} failedReads how many reads in a row may fail while a run is awaited
 * @property {number} lookMillis the pause between looks for a run that has not appeared
 * @property {number} pollMillis the pause between reads of a run that has not ended
 */

/** @type {Waits} */
export const WAITS = { appearMillis: 90_000, failedReads: 5, lookMillis: 10_000, pollMillis: 30_000 };

/** The conclusions of a run whose checks failed. */
const FAILED = new Set(['failure', 'startup_failure', 'timed_out']);

/** The conclusions of a job that the report of a failed run names. */
const UNSUCCESSFUL = new Set(['cancelled', 'failure', 'timed_out']);

/**
 * The newest run that a push started. CI runs for pushes to main alone, so a
 * push's run of a commit is the run of main.
 * @param {WorkflowRun[]} runs the runs of the CI workflow for one commit
 * @returns {WorkflowRun | undefined}
 */
export function pushRun(runs) {
  const [newest] = runs.filter((run) => run.event === 'push').sort((a, b) => b.run_number - a.run_number);
  return newest;
}

/**
 * What a run means for the release. A cancelled, skipped or stale run has
 * checked nothing, which counts as no run.
 * @param {WorkflowRun | undefined} run
 * @returns {RunState}
 */
export function stateOf(run) {
  if (run === undefined) {
    return 'none';
  }
  if (run.status !== 'completed') {
    return 'waiting';
  }
  if (run.conclusion === 'success') {
    return 'passed';
  }
  const hasFailed = FAILED.has(run.conclusion ?? '');
  return hasFailed ? 'failed' : 'none';
}

/**
 * The lines that name the jobs of a failed run that did not pass.
 * @param {WorkflowJob[]} jobs
 */
export function failedJobs(jobs) {
  return jobs
    .filter((job) => UNSUCCESSFUL.has(job.conclusion ?? ''))
    .map((job) => `  ${job.conclusion ?? ''}  ${job.name}  ${job.html_url}`);
}

/**
 * Finds the run that the push of the commit started, waits for it, and
 * returns what the release does: `passed` and `none` let it go on, `failed`
 * stops it.
 * @param {{ commit: string, isTagPush: boolean, repository: string }} release
 * @param {Surroundings} surroundings
 * @param {Waits} [waits]
 * @returns {Promise<'failed' | 'none' | 'passed'>}
 */
export async function commitChecks(release, surroundings, waits = WAITS) {
  const { get, now, sleep, write } = surroundings;
  const runsPath = `/repos/${release.repository}/actions/workflows/ci.yml/runs?head_sha=${release.commit}&event=push&per_page=20`;
  const find = async () => {
    const listed = /** @type {{ workflow_runs: WorkflowRun[] }} */ (await get(runsPath));
    return pushRun(listed.workflow_runs);
  };

  /** @type {WorkflowRun | undefined} */
  let run;
  try {
    run = await find();
    // A tag that is pushed with its commit starts the release in the moment
    // in which the push starts CI, so the run may not be listed yet. A manual
    // release does not wait: its commit was pushed before it.
    const appearsUntil = now() + (release.isTagPush ? waits.appearMillis : 0);
    while (run === undefined) {
      const isLate = now() >= appearsUntil;
      if (isLate) {
        break;
      }
      await sleep(waits.lookMillis);
      run = await find();
    }
  } catch (error) {
    write(`commit-checks: could not look for the commit's CI run: ${messageOf(error)}`);
    return 'none';
  }
  if (run === undefined) {
    write(`commit-checks: no push started CI for ${release.commit}`);
    return 'none';
  }

  write(`commit-checks: the push of ${release.commit} started ${run.html_url}`);
  const runPath = `/repos/${release.repository}/actions/runs/${String(run.id)}`;
  let failedReads = 0;
  let isAnnounced = false;
  for (;;) {
    const state = stateOf(run);
    if (state !== 'waiting') {
      break;
    }
    if (!isAnnounced) {
      write('commit-checks: waiting for the run to end');
      isAnnounced = true;
    }
    await sleep(waits.pollMillis);
    try {
      run = /** @type {WorkflowRun} */ (await get(runPath));
      failedReads = 0;
    } catch (error) {
      failedReads += 1;
      write(`commit-checks: could not read the run: ${messageOf(error)}`);
      if (failedReads >= waits.failedReads) {
        write('commit-checks: the result of the run is unknown. Re-run this job once the run has ended.');
        return 'failed';
      }
    }
  }

  const state = stateOf(run);
  if (state === 'passed') {
    write('commit-checks: the run passed, and the release takes its result');
    return 'passed';
  }
  if (state === 'none') {
    write(`commit-checks: the run ended as ${run.conclusion ?? 'unknown'} and checked nothing`);
    return 'none';
  }
  write(`commit-checks: the run failed (${run.conclusion ?? 'unknown'})`);
  try {
    const listed = /** @type {{ jobs: WorkflowJob[] }} */ (await get(`${runPath}/jobs?filter=latest&per_page=100`));
    for (const line of failedJobs(listed.jobs)) {
      write(line);
    }
  } catch (error) {
    write(`commit-checks: could not list its jobs: ${messageOf(error)}`);
  }
  write('commit-checks: re-run the failed jobs of that run, or push a fix, then re-run this job.');
  return 'failed';
}

/**
 * The message of something that was thrown.
 * @param {unknown} error
 */
function messageOf(error) {
  return error instanceof Error ? error.message : String(error);
}

const runsDirectly = process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (runsDirectly) {
  const [commit] = process.argv.slice(2);
  const { GITHUB_API_URL, GITHUB_EVENT_NAME, GITHUB_OUTPUT, GITHUB_REF_TYPE, GITHUB_REPOSITORY, GITHUB_TOKEN } =
    process.env;
  // An unset variable and an empty one both leave the script without a value.
  const isComplete = [commit, GITHUB_OUTPUT, GITHUB_REPOSITORY, GITHUB_TOKEN].every(Boolean);
  if (!isComplete) {
    console.error('usage: node scripts/ci/commit-checks.mjs <commit>, with GITHUB_TOKEN and the variables of a job');
    process.exit(2);
  }
  const api = GITHUB_API_URL ?? 'https://api.github.com';
  const isPush = GITHUB_EVENT_NAME === 'push';
  const isTag = GITHUB_REF_TYPE === 'tag';
  const result = await commitChecks(
    { commit: String(commit), isTagPush: isPush && isTag, repository: String(GITHUB_REPOSITORY) },
    {
      get: async (path) => {
        const response = await fetch(`${api}${path}`, {
          headers: {
            Accept: 'application/vnd.github+json',
            Authorization: `Bearer ${String(GITHUB_TOKEN)}`,
            'X-GitHub-Api-Version': '2022-11-28',
          },
        });
        if (!response.ok) {
          throw new Error(`GitHub answered ${String(response.status)} for ${path}`);
        }
        return response.json();
      },
      now: () => Date.now(),
      sleep: (milliseconds) =>
        new Promise((wake) => {
          setTimeout(wake, milliseconds);
        }),
      write: (text) => {
        console.log(text);
      },
    },
  );
  if (result === 'failed') {
    process.exitCode = 1;
  } else {
    appendFileSync(String(GITHUB_OUTPUT), `result=${result}\n`);
  }
}
