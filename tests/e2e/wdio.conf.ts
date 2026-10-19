// End-to-end tests of the desktop app (engineering standards, testing):
// WebdriverIO drives the validation build through tauri-driver, which runs the
// webview's own WebDriver: Microsoft Edge WebDriver for WebView2 on Windows and
// WebKitWebDriver for WebKitGTK on Linux. `task e2e` builds the app first.
//
// Every spec file gets a new session, so the app starts fresh for each one,
// with an empty data folder unless the spec asks otherwise.
import type { ChildProcess } from 'node:child_process';
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { createConnection } from 'node:net';
import { basename, join } from 'node:path';

import { applicationPath, plantDatabaseFromTheFuture, repository, resetDataDirectory } from './support/app.ts';

const DRIVER_PORT = 4444;

let driver: ChildProcess | undefined;

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

/** Resolves once something listens on the port, or rejects after the timeout. */
async function waitForPort(port: number, timeoutMillis: number): Promise<void> {
  const deadline = Date.now() + timeoutMillis;
  for (;;) {
    const listening = await new Promise<boolean>((resolve) => {
      const socket = createConnection({ host: '127.0.0.1', port }, () => {
        socket.end();
        resolve(true);
      });
      socket.on('error', () => {
        resolve(false);
      });
    });
    if (listening) {
      return;
    }
    if (Date.now() > deadline) {
      throw new Error(`tauri-driver did not listen on port ${String(port)} in time`);
    }
    await new Promise((resolve) => {
      setTimeout(resolve, 100);
    });
  }
}

export const config: WebdriverIO.Config = {
  capabilities: [
    {
      // @ts-expect-error -- tauri-driver's capability is not part of the WebdriverIO types.
      'tauri:options': { application: applicationPath() },
    },
  ],
  framework: 'mocha',
  hostname: '127.0.0.1',
  logLevel: 'warn',
  maxInstances: 1,
  mochaOpts: { timeout: 120_000, ui: 'bdd' },
  port: DRIVER_PORT,
  reporters: ['spec'],
  runner: 'local',
  // The core's specs, and each plugin's in its folder's tests/e2e (ADR-0035,
  // decision 9), which import this folder's support as `#e2e/<module>.ts`.
  specs: [
    './specs/**/*.test.ts',
    '../../plugins/*/tests/e2e/**/*.test.ts',
    '../fixtures/plugins/*/tests/e2e/**/*.test.ts',
  ],
  waitforTimeout: 10_000,

  beforeSession: async (_config, _capabilities, specs) => {
    resetDataDirectory();
    const startsWithNewerDatabase = specs.some((spec) => basename(spec).startsWith('startup-problem.'));
    if (startsWithNewerDatabase) {
      plantDatabaseFromTheFuture();
    }
    driver = spawn('tauri-driver', ['--port', String(DRIVER_PORT), ...nativeDriverArguments()], {
      stdio: ['ignore', 'inherit', 'inherit'],
    });
    await waitForPort(DRIVER_PORT, 15_000);
  },

  afterSession: () => {
    driver?.kill();
    driver = undefined;
  },
};
