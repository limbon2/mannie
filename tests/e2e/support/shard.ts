// The part of the end-to-end specs that one run takes. CI runs the specs in
// parts, each in a job of its own (.github/workflows/ci.yml), so the suite
// takes as long as its longest part: `MANNIE_E2E_SHARD=1/2` runs the first of
// two parts. WebdriverIO sorts the spec files and gives each part the next
// share of them, so the parts of one commit run every spec once.

/** A part of the specs, as WebdriverIO's `shard` option takes it. */
export interface Shard {
  current: number;
  total: number;
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
