// The SSH server of the end-to-end tests: the example program of the
// plugin's test-only crate, `mannie-ssh-testing`, which this file builds with
// cargo, because a `[[programs]]` entry of the manifest would ship it in
// releases. The server listens on a loopback port and prints one line of
// JSON with its port, its account and the fingerprint of its host key.
import { type ChildProcess, execFileSync, spawn } from 'node:child_process';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';

import { repository } from '#e2e/app.ts';

/** How long the server has to start and print its line. */
const START_TIMEOUT_MILLIS = 30_000;

/**
 * How long a hook that builds and starts the server may take. On a new
 * checkout, cargo first builds russh and its dependencies for the server.
 */
export const BUILD_AND_START_TIMEOUT_MILLIS = 600_000;

/** What the server printed about itself. */
interface ServerLine {
  readonly fingerprint: string;
  readonly password: string;
  readonly port: number;
  readonly username: string;
}

/** The example program in the shared target folder. */
function serverPath(): string {
  const binary = process.platform === 'win32' ? 'ssh-test-server.exe' : 'ssh-test-server';
  const targetDirectory = resolve(repository, process.env.CARGO_TARGET_DIR ?? 'target');
  return join(targetDirectory, 'debug', 'examples', binary);
}

/** Builds the example program; cargo builds nothing when it is current. */
function build(): void {
  execFileSync(
    'cargo',
    [
      'build',
      '--locked',
      '--manifest-path',
      join(repository, 'plugins', 'ssh', 'Cargo.toml'),
      '-p',
      'mannie-ssh-testing',
      '--example',
      'ssh-test-server',
    ],
    { stdio: 'inherit' },
  );
}

/** Whether a server process has ended, by an exit code or by a signal. */
function hasEnded(child: ChildProcess): boolean {
  const hasExited = child.exitCode !== null;
  const wasKilled = child.signalCode !== null;
  return hasExited || wasKilled;
}

/** Waits for the first line of the server's output. */
async function firstLine(child: ChildProcess): Promise<ServerLine> {
  const output = child.stdout;
  if (output === null) {
    throw new Error('the SSH test server has no output');
  }
  const lines = createInterface({ input: output });
  const received: { line: string | null } = { line: null };
  lines.once('line', (line) => {
    received.line = line;
  });
  try {
    await browser.waitUntil(
      () => {
        const hasPrinted = received.line !== null;
        return hasPrinted || hasEnded(child);
      },
      {
        interval: 100,
        timeout: START_TIMEOUT_MILLIS,
        timeoutMsg: `the SSH test server printed nothing within ${String(START_TIMEOUT_MILLIS / 1000)} seconds`,
      },
    );
  } finally {
    lines.close();
  }
  if (received.line === null) {
    throw new Error(`the SSH test server ended before it printed its line, with exit code ${String(child.exitCode)}`);
  }
  // The server prints the line as JSON of this shape.
  return JSON.parse(received.line) as ServerLine;
}

/** A running SSH test server. */
export class TestSshServer {
  readonly fingerprint: string;
  readonly password: string;
  readonly port: number;
  readonly username: string;
  readonly #process: ChildProcess;

  private constructor(child: ChildProcess, line: ServerLine) {
    this.#process = child;
    this.fingerprint = line.fingerprint;
    this.password = line.password;
    this.port = line.port;
    this.username = line.username;
  }

  /** Builds the server if it changed, starts it and waits for its line. A server that does not start is ended. */
  static async start(): Promise<TestSshServer> {
    build();
    const child = spawn(serverPath(), [], { stdio: ['ignore', 'pipe', 'inherit'] });
    try {
      return new TestSshServer(child, await firstLine(child));
    } catch (error) {
      child.kill();
      throw error;
    }
  }

  /** Ends the server and waits until it exited. */
  async stop(): Promise<void> {
    const child = this.#process;
    if (hasEnded(child)) {
      return;
    }
    const exited = new Promise((resolveExit) => {
      child.once('exit', resolveExit);
    });
    child.kill();
    await exited;
  }
}
