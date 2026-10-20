// @ts-check
// A manual CI repair runs one package's selected Rust tests. Arguments cross
// the workflow boundary as JSON and go directly to cargo without a shell.
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** @param {string} raw @returns {string[]} */
export function cargoTestArguments(raw) {
  const argumentsList = /** @type {unknown} */ (JSON.parse(raw));
  if (!Array.isArray(argumentsList)) {
    throw new Error('Cargo arguments must be a JSON array of nonempty strings.');
  }
  const stringsOnly = argumentsList.every((argument) => typeof argument === 'string');
  const hasEmptyArgument = argumentsList.includes('');
  if (!stringsOnly || hasEmptyArgument) {
    throw new Error('Cargo arguments must be a JSON array of nonempty strings.');
  }
  const isTest = argumentsList[0] === 'test' || (argumentsList[0] === 'nextest' && argumentsList[1] === 'run');
  const selectsPackage = argumentsList.includes('-p') || argumentsList.includes('--package');
  const selectsTest =
    argumentsList.includes('--exact') || argumentsList.includes('-E') || argumentsList.includes('--filter-expr');
  if (!isTest || !selectsPackage || !selectsTest || !argumentsList.includes('--locked')) {
    throw new Error('Select a locked cargo test or nextest run with a package and an exact test or filter.');
  }
  return argumentsList;
}

const runsDirectly = process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (runsDirectly) {
  const result = spawnSync('cargo', cargoTestArguments(process.env.MANNIE_CARGO_ARGUMENTS ?? ''), { stdio: 'inherit' });
  if (result.error !== undefined) {
    throw result.error;
  }
  process.exitCode = result.status ?? 1;
}
