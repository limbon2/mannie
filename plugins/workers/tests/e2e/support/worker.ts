// A real worker for the end-to-end tests (plugins/workers/docs/
// worker-activity.md, acceptance): the `mannie-worker` program that `task
// e2e` builds, run as in a terminal with a configuration in a temporary
// folder, listening on a loopback port. The test's own account owns that
// configuration, so the worker runs with `--allow-writable-config`, which
// the service definitions never pass.
import { type ChildProcess, execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { type AddressInfo, createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { repository } from '#e2e/app.ts';

/** How long the worker has to open its store and listen. */
const START_TIMEOUT_MILLIS = 30_000;

/** The worker program next to the validation build of the desktop app. */
export function workerPath(): string {
  const binary = process.platform === 'win32' ? 'mannie-worker.exe' : 'mannie-worker';
  const targetDirectory = resolve(repository, process.env.CARGO_TARGET_DIR ?? 'target');
  return join(targetDirectory, 'debug', binary);
}

/** A free port on the loopback interface. */
async function freePort(): Promise<number> {
  return new Promise((resolvePort, reject) => {
    const server = createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      // A server that listens on a port has an address object.
      const address = server.address() as AddressInfo;
      server.close(() => {
        resolvePort(address.port);
      });
    });
  });
}

/** A string as a TOML literal string, which keeps backslashes as they are. */
function literal(text: string): string {
  return `'${text}'`;
}

/** A worker in a temporary folder, which a test starts, stops and starts again. */
export class TestWorker {
  /** The address that the app pairs with. */
  readonly address: string;
  /** The worker's allowed folder, which is its tasks' workspace. */
  readonly allowed: string;
  readonly folder: string;
  /** The name that the worker gives itself. */
  readonly name: string;
  readonly #config: string;
  readonly #data: string;
  #process: ChildProcess | null = null;

  private constructor(folder: string, name: string, port: number) {
    this.folder = folder;
    this.name = name;
    this.address = `127.0.0.1:${String(port)}`;
    this.allowed = join(folder, 'work');
    this.#data = join(folder, 'data');
    this.#config = join(folder, 'worker.toml');
    mkdirSync(this.allowed, { recursive: true });
    writeFileSync(
      this.#config,
      [
        'version = 1',
        `name = "${name}"`,
        'purpose = "Runs the end-to-end tests"',
        `listen = "${this.address}"`,
        `data_dir = ${literal(this.#data)}`,
        '',
        '[ceiling]',
        'mode = "ask"',
        `allowed_folders = [${literal(this.allowed)}]`,
        '',
      ].join('\n'),
    );
  }

  /** A new worker named `name`, started. */
  static async start(name: string): Promise<TestWorker> {
    // The folder as the system names it, which is how the worker compares paths.
    const folder = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-e2e-worker-')));
    const worker = new TestWorker(folder, name, await freePort());
    await worker.start();
    return worker;
  }

  /** A new pairing code, as `mannie-worker pair` prints it on the host. */
  code(): string {
    const printed = execFileSync(workerPath(), ['pair', '--config', this.#config], { encoding: 'utf8' });
    const code = /^\s+Code:\s+(\S+)/mu.exec(printed)?.[1];
    if (code === undefined) {
      throw new Error(`mannie-worker pair printed no code: ${printed}`);
    }
    return code;
  }

  /** Deletes the worker's folder; the worker must have stopped. */
  remove(): void {
    rmSync(this.folder, { force: true, maxRetries: 20, recursive: true, retryDelay: 250 });
  }

  /** Starts the worker and waits until it opened its store and listens. */
  async start(): Promise<void> {
    const health = join(this.#data, 'health');
    // The marker of a worker that was killed stays behind.
    rmSync(health, { force: true });
    this.#process = spawn(workerPath(), ['run', '--config', this.#config, '--allow-writable-config'], {
      stdio: 'ignore',
    });
    await browser.waitUntil(() => existsSync(health), {
      interval: 100,
      timeout: START_TIMEOUT_MILLIS,
      timeoutMsg: `the worker did not start within ${String(START_TIMEOUT_MILLIS / 1000)} seconds`,
    });
  }

  /** Stops the worker at once, as when its host goes away. */
  async stop(): Promise<void> {
    const running = this.#process;
    this.#process = null;
    if (running === null) {
      return;
    }
    // A worker that ended by itself has nothing to stop.
    if (running.exitCode !== null) {
      return;
    }
    const exited = new Promise((resolveExit) => {
      running.once('exit', resolveExit);
    });
    running.kill('SIGKILL');
    await exited;
  }
}
