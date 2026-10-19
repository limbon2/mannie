// @ts-check
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { after, describe, it } from 'node:test';

import { fingerprint, fingerprintedFiles } from '../../../scripts/plugins/lib/fingerprint.mjs';

const scratch = mkdtempSync(join(tmpdir(), 'mannie-fingerprint-'));
after(() => {
  rmSync(scratch, { force: true, recursive: true });
});

/**
 * Writes files into the scratch folder.
 * @param {Record<string, string>} files the files' texts by their paths
 */
function write(files) {
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(dirname(join(scratch, path)), { recursive: true });
    writeFileSync(join(scratch, path), text);
  }
}

write({
  'Cargo.lock': 'version = 4\n',
  'Cargo.toml': '[workspace]\n',
  'apps/desktop/core/Cargo.toml': '[package]\n',
  'apps/desktop/src-tauri/Cargo.toml': '[package]\n',
  'plugins/notes/Cargo.lock': 'version = 4\n',
  'plugins/notes/Cargo.toml': '[workspace]\n',
  'plugins/notes/crates/mannie-notes/Cargo.toml': '[package]\n',
  'plugins/notes/plugin.toml': 'id = "notes"\n',
  'plugins/notes/target/debug/Cargo.toml': 'build output\n',
  'plugins/notes/ui/generated/Cargo.toml': 'generated\n',
  'plugins/notes/ui/index.ts': 'export {};\n',
  'plugins/stray/README.md': 'not a plugin\n',
});

describe('fingerprintedFiles', () => {
  it("lists the core's files and each plugin's manifest, lock and Cargo manifests, sorted", () => {
    assert.deepEqual(fingerprintedFiles(scratch), [
      '.generated/desktop/interfaces.json',
      'Cargo.lock',
      'Cargo.toml',
      'apps/desktop/core/Cargo.toml',
      'apps/desktop/src-tauri/Cargo.toml',
      'plugins/notes/Cargo.lock',
      'plugins/notes/Cargo.toml',
      'plugins/notes/crates/mannie-notes/Cargo.toml',
      'plugins/notes/plugin.toml',
    ]);
  });
});

describe('fingerprint', () => {
  it('marks a missing file and changes when a fingerprinted file changes', () => {
    const before = fingerprint(scratch);
    write({ 'plugins/notes/ui/index.ts': 'export const changed = true;\n' });
    const afterUiEdit = fingerprint(scratch);
    write({ 'plugins/notes/crates/mannie-notes/Cargo.toml': '[package]\nname = "mannie-notes"\n' });
    const afterManifestEdit = fingerprint(scratch);

    assert.match(before, /^missing {2}\.generated\/desktop\/interfaces\.json$/mu);
    assert.match(before, /^[0-9a-f]{64} {2}plugins\/notes\/plugin\.toml$/mu);
    assert.equal(afterUiEdit, before);
    assert.notEqual(afterManifestEdit, before);
  });
});
