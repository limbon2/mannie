// @ts-check
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';

const sdk = new URL('../../crates/mannie-plugin-sdk/', import.meta.url);

/**
 * A JSON file of the plugin SDK, parsed.
 * @param {string} path the file's path inside the SDK's folder
 */
function readSdkJson(path) {
  return JSON.parse(readFileSync(new URL(path, sdk), 'utf8'));
}

describe('plugin.schema.json', () => {
  // JSON Schema writes a pattern in the dialect of ECMA-262, JavaScript's
  // regular expressions, and does not anchor it, so this runs the pattern as
  // JavaScript does. The SDK's test `manifest` keeps the fixture's lists true
  // to `PluginId::parse`.
  it('gives an ID the pattern of the syntax of plugin IDs', () => {
    const schema = readSdkJson('plugin.schema.json');
    const ids = readSdkJson('tests/fixtures/plugin-id-syntax.json');
    const pattern = new RegExp(schema.properties.id.pattern, 'u');

    for (const id of ids.follows) {
      assert.match(id, pattern, JSON.stringify(id));
    }
    for (const id of ids.breaks) {
      assert.doesNotMatch(id, pattern, JSON.stringify(id));
    }
  });
});
