// The part of the end-to-end specs that one run takes. CI runs the specs in
// parts, each in a job of its own (.github/workflows/ci.yml), so the suite
// takes as long as its longest part: `MANNIE_E2E_SHARD=1/2` runs the first of
// two parts. Specs are distributed across parts by weight rather than in
// alphabetical halves, so plugin specs and restart-heavy core specs balance
// across every part instead of clustering in the last one.
import { basename } from 'node:path';

/** A part of the specs, as WebdriverIO's `shard` option takes it. */
export interface Shard {
  current: number;
  total: number;
}

/**
 * Relative runtime weights of the slower core and plugin specs, so balanced
 * sharding keeps every shard close in duration and complexity.
 */
const SPEC_WEIGHTS: Readonly<Record<string, number>> = {
  'browser.test.ts': 5,
  'chat-compaction.test.ts': 2,
  'chat-durability.test.ts': 2,
  'chat-security.test.ts': 2,
  'child-agents.test.ts': 3,
  'databases.test.ts': 3,
  'git.test.ts': 2,
  'images.test.ts': 2,
  'mcp.test.ts': 3,
  'notebooks.test.ts': 2,
  'notifications.test.ts': 3,
  'shell.test.ts': 4,
  'ssh.test.ts': 2,
  'terminal.test.ts': 3,
  'tools.test.ts': 4,
  'updates.test.ts': 3,
  'window.test.ts': 3,
  'workers.test.ts': 3,
};

/** Normalized POSIX-style path for stable cross-platform ordering. */
function normalizedPath(spec: string): string {
  return spec.replaceAll('\\', '/');
}

/** Estimated relative weight of one spec file on `platform`. */
export function specWeight(spec: string, platform: NodeJS.Platform = process.platform): number {
  const name = basename(spec);
  // The Browser plugin runs only its unsupported-platform check on Linux.
  const isUnsupportedBrowser = name === 'browser.test.ts' && platform !== 'win32';
  if (isUnsupportedBrowser) {
    return 1;
  }
  const known = SPEC_WEIGHTS[name];
  if (known !== undefined) {
    return known;
  }
  const isPluginSpec = normalizedPath(spec).includes('/plugins/');
  return isPluginSpec ? 2 : 1;
}

/**
 * Splits `specs` into `shard.total` balanced parts using longest-processing-
 * time-first assignment and returns the part named by `shard.current`, sorted
 * by path. Every spec appears in exactly one part.
 */
export function selectShardSpecs(
  specs: readonly string[],
  shard: Shard,
  platform: NodeJS.Platform = process.platform,
): string[] {
  const ordered = [...specs].toSorted((left, right) => {
    const difference = specWeight(right, platform) - specWeight(left, platform);
    return difference !== 0 ? difference : normalizedPath(left).localeCompare(normalizedPath(right));
  });
  const loads = Array.from({ length: shard.total }, () => 0);
  const buckets = Array.from({ length: shard.total }, (): string[] => []);
  for (const spec of ordered) {
    let lightest = 0;
    for (let index = 1; index < shard.total; index += 1) {
      if ((loads[index] ?? 0) < (loads[lightest] ?? 0)) {
        lightest = index;
      }
    }
    buckets[lightest]?.push(spec);
    loads[lightest] = (loads[lightest] ?? 0) + specWeight(spec, platform);
  }
  const chosen = buckets[shard.current - 1] ?? [];
  return chosen.toSorted((left, right) => normalizedPath(left).localeCompare(normalizedPath(right)));
}

/**
 * The part that `MANNIE_E2E_SHARD` names, such as `2/3`, or none when the
 * variable is unset or empty.
 */
export function shardOf(value: string | undefined): Shard | undefined {
  const text = value?.trim() ?? '';
  if (text === '') {
    return undefined;
  }
  const match = /^(?<current>[1-9]\d*)\/(?<total>[1-9]\d*)$/u.exec(text);
  const current = Number(match?.groups?.current);
  const total = Number(match?.groups?.total);
  // A value that does not match gives NaN, which is no part of anything.
  const isPart = current <= total;
  if (!isPart) {
    throw new Error('MANNIE_E2E_SHARD must name a part of the specs, such as 1/2');
  }
  return { current, total };
}
