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

/** How a test worker is configured. */
export interface TestWorkerOptions {
  /**
   * Whether its configuration allows the master's requests on the databases
   * of its host (ADR-0057, decision 2), in an allowed folder of their own.
   */
  databases?: boolean;
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
  /**
   * The allowed folder for databases, outside the worker's folder: the
   * worker refuses files in the folder of its configuration (ADR-0057,
   * decision 3), where the other allowed folder lies.
   */
  readonly #databases: string | null;
  #process: ChildProcess | null = null;

  private constructor(folder: string, name: string, port: number, options: TestWorkerOptions) {
    this.folder = folder;
    this.name = name;
    this.address = `127.0.0.1:${String(port)}`;
    this.allowed = join(folder, 'work');
    this.#data = join(folder, 'data');
    this.#config = join(folder, 'worker.toml');
    this.#databases =
      options.databases === true
        ? realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-e2e-worker-databases-')))
        : null;
    mkdirSync(this.allowed, { recursive: true });
    const allowedFolders = this.#databases === null ? [this.allowed] : [this.allowed, this.#databases];
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
        `allowed_folders = [${allowedFolders.map(literal).join(', ')}]`,
        '',
        ...(this.#databases === null ? [] : ['[databases]', 'enabled = true', '']),
      ].join('\n'),
    );
  }

  /** A new worker named `name`, started. */
  static async start(name: string, options: TestWorkerOptions = {}): Promise<TestWorker> {
    // The folder as the system names it, which is how the worker compares paths.
    const folder = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-e2e-worker-')));
    const worker = new TestWorker(folder, name, await freePort(), options);
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

  /** A path in the worker's allowed folder for databases, which its options must allow. */
  databasePath(file: string): string {
    if (this.#databases === null) {
      throw new Error('the worker was started without databases');
    }
    return join(this.#databases, file);
  }

  /** Deletes the worker's folders; the worker must have stopped. */
  remove(): void {
    for (const folder of [this.folder, this.#databases]) {
      if (folder !== null) {
        rmSync(folder, { force: true, maxRetries: 20, recursive: true, retryDelay: 250 });
      }
    }
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
