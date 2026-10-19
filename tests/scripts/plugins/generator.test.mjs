// @ts-check
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, statSync, utimesSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, describe, it } from 'node:test';

import { isCurrent, writeIfChanged } from '../../../scripts/plugins/lib/generator.mjs';

const scratch = mkdtempSync(join(tmpdir(), 'mannie-generator-'));
after(() => {
  rmSync(scratch, { force: true, recursive: true });
});

describe('writeIfChanged', () => {
  it('writes a new or changed file, and leaves an unchanged one and its time alone', () => {
    const file = { path: 'launcher/plugin-list/src/lib.rs', text: 'pub fn plugins() {}\n' };
    const path = join(scratch, file.path);

    const wroteNew = writeIfChanged(scratch, file);
    const past = new Date('2026-01-01T00:00:00Z');
    utimesSync(path, past, past);
    const wroteSame = writeIfChanged(scratch, file);
    const timeAfterSame = statSync(path).mtime.getTime();
    const wroteChanged = writeIfChanged(scratch, { ...file, text: 'pub fn plugins() -> Vec<()> { vec![] }\n' });

    assert.equal(wroteNew, true);
    assert.equal(wroteSame, false);
    assert.equal(timeAfterSame, past.getTime());
    assert.equal(wroteChanged, true);
    assert.equal(readFileSync(path, 'utf8'), 'pub fn plugins() -> Vec<()> { vec![] }\n');
  });
});

describe('isCurrent', () => {
  it('tells a missing or stale file from a current one', () => {
    const file = { path: 'launcher/plugins.json', text: '[]\n' };

    const missing = isCurrent(scratch, file);
    writeIfChanged(scratch, file);
    const current = isCurrent(scratch, file);
    const stale = isCurrent(scratch, { ...file, text: '[{"id":"notes"}]\n' });

    assert.equal(missing, false);
    assert.equal(current, true);
    assert.equal(stale, false);
  });
});
