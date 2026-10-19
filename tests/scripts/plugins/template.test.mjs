// @ts-check
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  namesOf,
  renamed,
  renamedPath,
  TEMPLATE_FOLDER,
  templateFile,
  withLeadingComment,
} from '../../../scripts/plugins/lib/template.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));

/**
 * The committed files of the template, relative to its folder.
 * @param {string} folder
 * @returns {string[]}
 */
function templateFiles(folder) {
  return readdirSync(join(root, TEMPLATE_FOLDER, folder), { withFileTypes: true }).flatMap((entry) => {
    const path = folder === '' ? entry.name : `${folder}/${entry.name}`;
    const isOutput = ['Cargo.lock', 'generated', 'target'].includes(entry.name);
    if (isOutput) {
      return [];
    }
    return entry.isDirectory() ? templateFiles(path) : [path];
  });
}

describe('namesOf', () => {
  it('gives the forms of an ID that code uses', () => {
    assert.deepEqual(namesOf('log-viewer'), {
      id: 'log-viewer',
      pascal: 'LogViewer',
      prefix: 'log_viewer',
      upper: 'LOG_VIEWER',
    });
  });
});

describe('renamed', () => {
  it("changes each of the fixture's names to the new plugin's", () => {
    const text = [
      'use mannie_example::plugin::ExamplePlugin;',
      'pub const EXAMPLE: &str = "example";',
      'pub const PREFIX: &str = "example";',
      'CREATE TABLE example_notes',
      'entry = "mannie-example"',
      'mannie-plugin-sdk = { path = "../../../../crates/mannie-plugin-sdk" }',
      '"example.storage_failed"',
      "into [Mannie's changelog](../../../../CHANGELOG.md)",
    ].join('\n');

    assert.equal(
      renamed(text, 'log-viewer'),
      [
        'use mannie_log_viewer::plugin::LogViewerPlugin;',
        'pub const LOG_VIEWER: &str = "log-viewer";',
        'pub const PREFIX: &str = "log_viewer";',
        'CREATE TABLE log_viewer_notes',
        'entry = "mannie-log-viewer"',
        'mannie-plugin-sdk = { path = "../../crates/mannie-plugin-sdk" }',
        '"log-viewer.storage_failed"',
        "into [Mannie's changelog](../../CHANGELOG.md)",
      ].join('\n'),
    );
    assert.equal(renamedPath('crates/mannie-example/src/lib.rs', 'log-viewer'), 'crates/mannie-log-viewer/src/lib.rs');
  });
});

describe('withLeadingComment', () => {
  it('replaces the leading comment and keeps the schema line before it', () => {
    const text = '#:schema ../schema.json\n# One.\n# Two.\n\nid = "x"\n';

    assert.equal(withLeadingComment(text, '#', 'New.'), '#:schema ../schema.json\n# New.\n\nid = "x"\n');
    assert.equal(withLeadingComment('id = "x"\n', '#', 'New.'), 'id = "x"\n');
  });
});

describe('templateFile', () => {
  it("leaves no name and no role of the fixture in a new plugin's files", () => {
    const files = templateFiles('');

    assert.ok(files.includes('plugin.toml'));
    for (const path of files) {
      const text = templateFile(path, readFileSync(join(root, TEMPLATE_FOLDER, path), 'utf8'), 'notes-x');

      assert.doesNotMatch(text, /example|fixture/iu, path);
    }
  });

  it("writes the new plugin's name where the fixture states its role", () => {
    const manifest = templateFile(
      'plugin.toml',
      readFileSync(join(root, TEMPLATE_FOLDER, 'plugin.toml'), 'utf8'),
      'notes-x',
    );

    assert.match(
      manifest,
      /^#:schema \.\.\/\.\.\/crates\/mannie-plugin-sdk\/plugin\.schema\.json\n# The notes-x plugin\.\n\nid = "notes-x"\n/u,
    );
    assert.match(manifest, /^order = 50$/mu);
  });

  it("names the new plugin in its UI's entry and in its catalog's description", () => {
    const entry = templateFile(
      'ui/index.ts',
      readFileSync(join(root, TEMPLATE_FOLDER, 'ui', 'index.ts'), 'utf8'),
      'notes-x',
    );
    const catalog = JSON.parse(
      templateFile(
        'messages/en.json',
        readFileSync(join(root, TEMPLATE_FOLDER, 'messages', 'en.json'), 'utf8'),
        'notes-x',
      ),
    );

    assert.match(entry, /^\/\/ The notes-x plugin's UI\.\nimport /u);
    assert.equal(catalog.notes_x_plugin_description, 'The notes-x plugin.');
    assert.equal(catalog.notes_x_plugin_name, 'NotesX');
  });
});
