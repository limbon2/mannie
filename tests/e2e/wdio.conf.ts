// End-to-end tests of the desktop app (engineering standards, testing):
// WebdriverIO drives the validation build through tauri-driver, which runs the
// webview's own WebDriver: Microsoft Edge WebDriver for WebView2 on Windows and
// WebKitWebDriver for WebKitGTK on Linux. `task e2e` builds the app first.
//
// Every spec file gets a new session on fresh ephemeral driver ports, so the
// app starts fresh for each one with an empty data folder unless the spec asks
// otherwise, and teardown waits for the driver tree and ports to close before
// the next spec starts.
import { type ChildProcess, spawn, spawnSync } from 'node:child_process';
import { existsSync, globSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { type AddressInfo, createConnection, createServer } from 'node:net';
import { basename, dirname, isAbsolute, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  applicationPath,
  dataDirectory,
  plantDatabaseFromTheFuture,
  repository,
  resetDataDirectory,
} from './support/app.ts';
import { selectShardSpecs, shardOf } from './support/shard.ts';

const CONFIG_DIRECTORY = dirname(fileURLToPath(import.meta.url));
const DEFAULT_SPEC_PATTERNS = [
  './specs/**/*.test.ts',
  '../../plugins/*/tests/e2e/**/*.test.ts',
  '../fixtures/plugins/*/tests/e2e/**/*.test.ts',
];
interface DriverPorts {
  readonly driver: number;
  readonly native: number;
}

interface AppLogFile {
  readonly content: string;
  readonly name: string;
}

interface FailedTestMetadata {
  readonly parent?: string;
  readonly title?: string;
}

const FALLBACK_DRIVER_PORT = 4444;

let driver: ChildProcess | undefined;
let activePorts: DriverPorts | undefined;

const focusedSpec = process.env.MANNIE_E2E_SPEC?.trim();
const hasFocusedSpec = focusedSpec !== undefined && focusedSpec !== '';
if (hasFocusedSpec) {
  const insideRepository = resolve(repository, focusedSpec).startsWith(resolve(repository) + sep);
  const testFile = focusedSpec.endsWith('.test.ts') && existsSync(resolve(repository, focusedSpec));
  if (isAbsolute(focusedSpec) || !insideRepository || !testFile) {
    throw new Error('MANNIE_E2E_SPEC must name a .test.ts file inside the repository');
  }
}

/** Resolves all end-to-end specs, balanced across shards when `MANNIE_E2E_SHARD` is set. */
function resolvedSpecs(): string[] {
  if (hasFocusedSpec) {
    return [resolve(repository, focusedSpec)];
  }
  const shard = shardOf(process.env.MANNIE_E2E_SHARD);
  if (shard === undefined) {
    return DEFAULT_SPEC_PATTERNS;
  }
  const discovered = DEFAULT_SPEC_PATTERNS.flatMap((pattern) =>
    globSync(pattern, { cwd: CONFIG_DIRECTORY }).map((entry) => resolve(CONFIG_DIRECTORY, entry)),
  );
  return selectShardSpecs([...new Set(discovered)], shard);
}

/**
 * On Windows, the Microsoft Edge WebDriver that `node scripts/edgedriver.mjs`
 * installed for this machine's WebView2 runtime; without it, tauri-driver
 * looks for msedgedriver on the path. Linux uses WebKitWebDriver from the path.
 */
function nativeDriverArguments(): string[] {
  const edgeDriver = join(repository, 'target', 'edgedriver', 'msedgedriver.exe');
  const useInstalledDriver = process.platform === 'win32' && existsSync(edgeDriver);
  return useInstalledDriver ? ['--native-driver', edgeDriver] : [];
}

const tauriOptions = { application: applicationPath() };

/** Allocates a currently free TCP port on loopback. */
async function freePort(): Promise<number> {
  return new Promise((resolvePort, reject) => {
    const server = createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const address = server.address() as AddressInfo;
      server.close(() => {
        resolvePort(address.port);
      });
    });
  });
}

/** Allocates two distinct free TCP ports on loopback for tauri-driver and its native driver. */
async function allocateDriverPorts(): Promise<DriverPorts> {
  const first = await freePort();
  for (;;) {
    const second = await freePort();
    if (second !== first) {
      return { driver: first, native: second };
    }
  }
}

/** Returns whether anything accepts TCP connections on `port`. */
async function isPortOpen(port: number): Promise<boolean> {
  return new Promise<boolean>((resolveOpen) => {
    const socket = createConnection({ host: '127.0.0.1', port }, () => {
      socket.end();
      resolveOpen(true);
    });
    socket.on('error', () => {
      resolveOpen(false);
    });
  });
}

/** Resolves once `port` listens, or rejects immediately if `child` exits or the timeout elapses. */
async function waitForDriverReady(child: ChildProcess, port: number, timeoutMillis: number): Promise<void> {
  const deadline = Date.now() + timeoutMillis;
  let spawnFailure: Error | undefined;
  let exitedEarly: string | undefined;
  const onError = (error: Error): void => {
    spawnFailure = error;
  };
  const onExit = (code: number | null, signal: NodeJS.Signals | null): void => {
    exitedEarly = `tauri-driver exited before listening on port ${String(port)} (code=${String(code)}, signal=${String(signal)})`;
  };
  child.once('error', onError);
  child.once('exit', onExit);
  try {
    for (;;) {
      if (spawnFailure !== undefined) {
        throw spawnFailure;
      }
      if (exitedEarly !== undefined) {
        throw new Error(exitedEarly);
      }
      if (await isPortOpen(port)) {
        return;
      }
      if (Date.now() > deadline) {
        throw new Error(`tauri-driver did not listen on port ${String(port)} in time`);
      }
      await new Promise((resolvePause) => {
        setTimeout(resolvePause, 100);
      });
    }
  } finally {
    child.off('error', onError);
    child.off('exit', onExit);
  }
}

/** Waits until `port` no longer accepts connections, up to `timeoutMillis`. */
async function waitForPortClosed(port: number, timeoutMillis: number): Promise<void> {
  const deadline = Date.now() + timeoutMillis;
  while (Date.now() < deadline) {
    if (!(await isPortOpen(port))) {
      return;
    }
    await new Promise((resolvePause) => {
      setTimeout(resolvePause, 100);
    });
  }
}

/** Stops `child` and its child processes (native WebDriver and app) and waits until it exits. */
async function stopDriverTree(child: ChildProcess): Promise<void> {
  const alreadyEnded = child.exitCode !== null || child.signalCode !== null;
  if (alreadyEnded) {
    return;
  }
  const exited = new Promise<void>((resolveExit) => {
    child.once('exit', () => {
      resolveExit();
    });
  });
  const pid = child.pid;
  if (process.platform === 'win32') {
    if (pid === undefined) {
      child.kill();
    } else {
      spawnSync('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' });
    }
    await Promise.race([
      exited,
      new Promise<void>((resolvePause) => {
        setTimeout(resolvePause, 5_000);
      }),
    ]);
    return;
  }
  // On Unix, tauri-driver starts in its own process group so signaling -pid
  // reaches WebKitWebDriver and any lingering validation app process together.
  const sendGroupSignal = (signal: NodeJS.Signals): void => {
    if (pid === undefined) {
      child.kill(signal);
      return;
    }
    try {
      process.kill(-pid, signal);
    } catch {
      child.kill(signal);
    }
  };
  sendGroupSignal('SIGTERM');
  const graceful = await Promise.race([
    exited.then(() => true),
    new Promise<boolean>((resolvePause) => {
      setTimeout(() => {
        resolvePause(false);
      }, 3_000);
    }),
  ]);
  if (!graceful) {
    sendGroupSignal('SIGKILL');
    await Promise.race([
      exited,
      new Promise<void>((resolvePause) => {
        setTimeout(resolvePause, 3_000);
      }),
    ]);
  }
}

/** Kills any leftover driver if the worker exits abnormally before afterSession runs. */
process.once('exit', () => {
  const pid = driver?.pid;
  if (pid === undefined) {
    return;
  }
  if (process.platform === 'win32') {
    spawnSync('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' });
    return;
  }
  try {
    process.kill(-pid, 'SIGKILL');
  } catch {
    driver?.kill('SIGKILL');
  }
});

/** Turns a test or spec label into a short, safe folder name. */
function safeSegment(text: string): string {
  return (
    text
      .toLowerCase()
      .replaceAll(/[^a-z0-9]+/gu, '-')
      .replaceAll(/^-+|-+$/gu, '')
      .slice(0, 60) || 'unknown'
  );
}

/** Reads the newest log file from the validation app's log folder, if any. */
function latestAppLog(): AppLogFile | null {
  const logsFolder = join(dataDirectory(), 'logs');
  if (!existsSync(logsFolder)) {
    return null;
  }
  const files = readdirSync(logsFolder)
    .filter((name) => name.endsWith('.log'))
    .map((name) => {
      const path = join(logsFolder, name);
      return { mtimeMs: statSync(path).mtimeMs, name, path };
    })
    .toSorted((left, right) => right.mtimeMs - left.mtimeMs);
  const newest = files[0];
  if (newest === undefined) {
    return null;
  }
  return { content: readFileSync(newest.path, 'utf8'), name: newest.name };
}

/** Saves screenshot, context metadata and app log tail for a failed end-to-end test. */
async function captureFailureDiagnostics(test: FailedTestMetadata, error: unknown): Promise<void> {
  const label = `${safeSegment(test.parent ?? 'suite')}-${safeSegment(test.title ?? 'test')}-${String(Date.now())}`;
  const folder = join(repository, 'target', 'e2e-failures', label);
  mkdirSync(folder, { recursive: true });
  let currentUrl: string | null = null;
  let pageTitle: string | null = null;
  try {
    currentUrl = await browser.getUrl();
    pageTitle = await browser.getTitle();

    await browser.saveScreenshot(join(folder, 'screenshot.png'));
  } catch {
    // The session or window may already have closed when the test failed.
  }
  const errorMessage = error instanceof Error ? `${error.message}\n${error.stack ?? ''}` : String(error);
  const contextPayload = {
    error: errorMessage,
    pageTitle,
    parent: test.parent,
    title: test.title,
    url: currentUrl,
  };
  writeFileSync(join(folder, 'context.json'), `${JSON.stringify(contextPayload, undefined, 2)}\n`);
  const log = latestAppLog();
  if (log !== null) {
    writeFileSync(join(folder, log.name), log.content);
    const tail = log.content.trimEnd().split(/\r?\n/u).slice(-40).join('\n');
    console.error(`[e2e failure] ${test.parent ?? ''} > ${test.title ?? ''} (url=${String(currentUrl)})`);
    console.error(`[e2e failure] last lines of ${log.name}:\n${tail}`);
  }
}

export const config: WebdriverIO.Config = {
  capabilities: [
    {
      // @ts-expect-error -- tauri-driver's capability is not part of the WebdriverIO types.
      'tauri:options': tauriOptions,
    },
  ],
  // Driver requests stay on this machine. Retrying a failed new session
  // relaunches the same broken runtime and hides the startup failure.
  connectionRetryCount: 0,
  framework: 'mocha',
  hostname: '127.0.0.1',
  logLevel: 'warn',
  maxInstances: 1,
  mochaOpts: { timeout: 120_000, ui: 'bdd' },
  port: FALLBACK_DRIVER_PORT,
  reporters: ['spec'],
  runner: 'local',
  // Specs are already balanced across shards by `resolvedSpecs()`.
  shard: undefined,
  // The core's specs, and each plugin's in its folder's tests/e2e (ADR-0035,
  // decision 9), which import this folder's support as `#e2e/<module>.ts`.
  specs: resolvedSpecs(),
  waitforTimeout: 10_000,

  beforeSession: async (wdioConfig, capabilities, specs) => {
    resetDataDirectory();
    // The shell spec checks local storage across a clean app restart. Edge
    // WebDriver creates a fresh profile per session for the other specs.
    const isWindows = process.platform === 'win32';
    const testsShell = specs.some((spec) => basename(spec) === 'shell.test.ts');
    if (isWindows && testsShell) {
      Object.assign(capabilities, {
        'tauri:options': {
          ...tauriOptions,
          webviewOptions: { userDataFolder: join(dataDirectory(), 'webdriver-profile') },
        },
      });
    }
    const startsWithNewerDatabase = specs.some((spec) => basename(spec).startsWith('startup-problem.'));
    if (startsWithNewerDatabase) {
      plantDatabaseFromTheFuture();
    }
    const ports = await allocateDriverPorts();

    activePorts = ports;
    Object.assign(wdioConfig, { port: ports.driver });
    const spawned = spawn(
      'tauri-driver',
      ['--port', String(ports.driver), '--native-port', String(ports.native), ...nativeDriverArguments()],
      {
        detached: !isWindows,
        stdio: ['ignore', 'inherit', 'inherit'],
      },
    );

    driver = spawned;

    try {
      await waitForDriverReady(spawned, ports.driver, 15_000);
    } catch (error) {
      await stopDriverTree(spawned);

      driver = undefined;
      activePorts = undefined;
      throw error;
    }
  },

  afterTest: async (test, _context, { error, passed }) => {
    if (!passed) {
      await captureFailureDiagnostics(test, error);
    }
  },

  afterSession: async () => {
    const running = driver;
    const ports = activePorts;

    driver = undefined;
    activePorts = undefined;
    if (running !== undefined) {
      await stopDriverTree(running);
    }
    if (ports !== undefined) {
      await Promise.all([waitForPortClosed(ports.driver, 5_000), waitForPortClosed(ports.native, 5_000)]);
    }
  },
};
