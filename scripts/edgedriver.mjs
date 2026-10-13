// @ts-check
// Installs the Microsoft Edge WebDriver that matches this machine's WebView2
// runtime. tauri-driver needs it for the end-to-end tests on Windows
// (`task e2e`), and its version must equal the runtime's, so it is downloaded
// per machine into target/edgedriver/ and never committed. Node's fetch uses
// the HTTPS_PROXY variable only when NODE_USE_ENV_PROXY=1 is set.

import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

/** The registry keys where the WebView2 runtime records its version: per machine, then per user. */
const RUNTIME_KEYS = [
  String.raw`HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`,
  String.raw`HKCU\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`,
];

/** Where tests/e2e/wdio.conf.ts looks for the driver, relative to the repository. */
export const DRIVER_PATH = join('target', 'edgedriver', 'msedgedriver.exe');

/**
 * The runtime version in the output of `reg query <key> /v pv`.
 * @param {string} output
 * @returns {string | undefined}
 */
export function versionFromRegistry(output) {
  return /\bpv\s+REG_SZ\s+(\d+(?:\.\d+){3})\b/.exec(output)?.[1];
}

/**
 * The installed WebView2 runtime's version.
 * @returns {string}
 */
function webView2Version() {
  const system = process.env.SystemRoot ?? String.raw`C:\Windows`;
  for (const key of RUNTIME_KEYS) {
    try {
      const output = execFileSync(join(system, 'System32', 'reg.exe'), ['query', key, '/v', 'pv'], {
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'ignore'],
      });
      const version = versionFromRegistry(output);
      if (version !== undefined) {
        return version;
      }
    } catch {
      // The key does not exist; try the next one.
    }
  }
  throw new Error('the WebView2 runtime is not installed');
}

/**
 * Downloads and unpacks the driver for `version` unless it is already there.
 * @param {string} repository
 * @param {string} version
 * @returns {Promise<string>} the driver's path
 */
async function install(repository, version) {
  const driver = join(repository, DRIVER_PATH);
  const folder = dirname(driver);
  const versionFile = join(folder, 'version.txt');
  const isCurrent = existsSync(driver) && existsSync(versionFile) && readFileSync(versionFile, 'utf8') === version;
  if (isCurrent) {
    return driver;
  }
  rmSync(folder, { force: true, recursive: true });
  mkdirSync(folder, { recursive: true });
  const url = `https://msedgedriver.microsoft.com/${version}/edgedriver_win64.zip`;
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`downloading ${url} failed with HTTP ${String(response.status)}`);
  }
  const archive = join(folder, 'edgedriver_win64.zip');
  writeFileSync(archive, Buffer.from(await response.arrayBuffer()));
  // Windows' own tar unpacks zip archives; other tar programs on the path may not.
  const system = process.env.SystemRoot ?? String.raw`C:\Windows`;
  execFileSync(join(system, 'System32', 'tar.exe'), ['-xf', archive, '-C', folder], { stdio: 'inherit' });
  rmSync(archive);
  writeFileSync(versionFile, version);
  return driver;
}

const isMain = process.argv[1] === fileURLToPath(import.meta.url);
if (isMain) {
  if (process.platform !== 'win32') {
    console.error('edgedriver: only Windows needs Microsoft Edge WebDriver; Linux uses WebKitWebDriver');
    process.exitCode = 1;
  } else {
    const repository = join(dirname(fileURLToPath(import.meta.url)), '..');
    const version = webView2Version();
    const driver = await install(repository, version);
    console.log(`edgedriver: Microsoft Edge WebDriver ${version} for WebView2 ${version} is at ${driver}`);
  }
}
