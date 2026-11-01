// The low-end laptop profile of the performance suite (docs/product/
// goals-and-scope.md §4; NFR-PERF-06 to 10): `MANNIE_PERF_PROFILE=low-end`
// limits Mannie's processes to 2 cores and, on Windows, slows WebView2's CPU
// 4 times through the DevTools protocol, which Microsoft Edge WebDriver
// serves for its session. On Linux the profile runs in a virtual machine with
// 2 vCPUs and no GPU instead, since WebKitGTK has no such protocol.
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

/** Whether the suite runs under the low-end profile. */
export const LOW_END = process.env.MANNIE_PERF_PROFILE === 'low-end';

/** How many times slower the profile makes WebView2's CPU. */
const CPU_SLOWDOWN = 4;

/** The cores that Mannie's processes may use: the first two. */
const TWO_CORES = 0b11;

/**
 * Slows the CPU of WebView2's page through the DevTools protocol of the
 * session's driver, which tauri-driver passes on to Microsoft Edge
 * WebDriver.
 */
async function slowDownWebview(): Promise<void> {
  const { hostname = '127.0.0.1', port = 4444 } = browser.options;
  const response = await fetch(`http://${hostname}:${String(port)}/session/${browser.sessionId}/ms/cdp/execute`, {
    body: JSON.stringify({ cmd: 'Emulation.setCPUThrottlingRate', params: { rate: CPU_SLOWDOWN } }),
    headers: { 'content-type': 'application/json' },
    method: 'POST',
  });
  if (!response.ok) {
    throw new Error(`the driver did not slow the webview: ${String(response.status)} ${await response.text()}`);
  }
}

/** Every process of the system as its ID and its parent's ID. */
function processPairs(): [number, number][] {
  const listed =
    process.platform === 'win32'
      ? execFileSync(
          'powershell',
          [
            '-NoProfile',
            '-Command',
            'Get-CimInstance Win32_Process | ForEach-Object { "$($_.ProcessId) $($_.ParentProcessId)" }',
          ],
          { encoding: 'utf8' },
        )
      : execFileSync('ps', ['-e', '-o', 'pid=,ppid='], { encoding: 'utf8' });
  const pairs: [number, number][] = [];
  for (const line of listed.split(/\r?\n/u)) {
    const [child = Number.NaN, parent = Number.NaN] = line.trim().split(/\s+/u).map(Number);
    const isPair = Number.isFinite(child) && Number.isFinite(parent);
    if (isPair) {
      pairs.push([child, parent]);
    }
  }
  return pairs;
}

/** The IDs of a process and every process below it. */
function processTree(root: number): number[] {
  const children = new Map<number, number[]>();
  for (const [child, parent] of processPairs()) {
    children.set(parent, [...(children.get(parent) ?? []), child]);
  }
  const tree: number[] = [];
  const waiting = [root];
  for (let next = waiting.pop(); next !== undefined; next = waiting.pop()) {
    tree.push(next);
    waiting.push(...(children.get(next) ?? []));
  }
  return tree;
}

/** Limits Mannie's process and every process below it to two cores. */
function limitToTwoCores(root: number): void {
  for (const pid of processTree(root)) {
    if (process.platform === 'win32') {
      execFileSync('powershell', [
        '-NoProfile',
        '-Command',
        `(Get-Process -Id ${String(pid)}).ProcessorAffinity = ${String(TWO_CORES)}`,
      ]);
    } else {
      execFileSync('taskset', ['-p', TWO_CORES.toString(16), String(pid)]);
    }
  }
}

/** Applies the low-end profile to the app whose process is `root`, when the suite runs under it. */
export async function applyLowEndProfile(root: number): Promise<void> {
  if (!LOW_END) {
    return;
  }
  limitToTwoCores(root);
  if (process.platform === 'win32') {
    await slowDownWebview();
  }
}

/** The CPU time in milliseconds that a process and every process below it used. */
export function cpuMilliseconds(root: number): number {
  const tree = processTree(root);
  if (process.platform === 'win32') {
    const total = execFileSync(
      'powershell',
      [
        '-NoProfile',
        '-Command',
        `(Get-Process -Id ${tree.join(',')} -ErrorAction SilentlyContinue | ForEach-Object { $_.TotalProcessorTime.TotalMilliseconds } | Measure-Object -Sum).Sum`,
      ],
      { encoding: 'utf8' },
    );
    return Number(total.trim());
  }
  const ticksPerSecond = Number(execFileSync('getconf', ['CLK_TCK'], { encoding: 'utf8' }).trim());
  let ticks = 0;
  for (const pid of tree) {
    const stat = readFileSync(`/proc/${String(pid)}/stat`, 'utf8');
    const fields = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
    ticks += Number(fields[11]) + Number(fields[12]);
  }
  return (ticks / ticksPerSecond) * 1_000;
}
