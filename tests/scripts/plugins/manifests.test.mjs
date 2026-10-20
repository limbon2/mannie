// @ts-check
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { after, describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  checkId,
  checkManifest,
  findPlugins,
  isInheritable,
  isProcessName,
  memberFolders,
  prefixesNest,
  readRules,
} from '../../../scripts/plugins/lib/manifests.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const rules = readRules(root);

/** A manifest with the fields that every manifest has. */
const MINIMAL = `id = "notes"
sdk = 1
order = 30

[rust]
entry = "mannie-notes"
`;

/**
 * The minimal manifest with more TOML after it.
 * @param {string} more
 */
function withMore(more) {
  return `${MINIMAL}\n${more}\n`;
}

/**
 * The problems of a manifest's text.
 * @param {string} text
 */
function problemsOf(text) {
  return checkManifest(text, rules).problems;
}

const scratch = mkdtempSync(join(tmpdir(), 'mannie-plugins-'));
after(() => {
  rmSync(scratch, { force: true, recursive: true });
});

/**
 * A tree of files in a new folder of the scratch folder.
 * @param {Record<string, string>} files the files' texts by their paths
 */
function tree(files) {
  const folder = mkdtempSync(join(scratch, 'tree-'));
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(dirname(join(folder, path)), { recursive: true });
    writeFileSync(join(folder, path), text);
  }
  return folder;
}

/**
 * The files of a plugin's folder: its manifest, its workspace and its entry
 * crate.
 * @param {string} folder such as `plugins/notes`
 * @param {string} manifest
 */
function plugin(folder, manifest) {
  return {
    [`${folder}/Cargo.toml`]: '[workspace]\nmembers = ["crates/*"]\n',
    [`${folder}/crates/entry/Cargo.toml`]: '[package]\nname = "entry"\n',
    [`${folder}/plugin.toml`]: manifest,
  };
}

/**
 * A manifest with an ID and an order.
 * @param {string} id
 * @param {number} order
 */
function manifestOf(id, order) {
  return `id = "${id}"\nsdk = 1\norder = ${String(order)}\n\n[rust]\nentry = "mannie-${id}"\n`;
}

describe('checkId', () => {
  // The SDK's test `manifest` holds `PluginId::parse` to the same fixture.
  it('judges the IDs of the SDK fixture as the host does', () => {
    /** @type {{ accepted: string[], refused: Record<string, string[]> }} */
    const fixture = JSON.parse(
      readFileSync(join(root, 'crates/mannie-plugin-sdk/tests/fixtures/plugin-ids.json'), 'utf8'),
    );

    for (const id of fixture.accepted) {
      assert.equal(checkId(id, rules), null, JSON.stringify(id));
    }
    for (const [rule, ids] of Object.entries(fixture.refused)) {
      for (const id of ids) {
        assert.equal(checkId(id, rules)?.rule, rule, JSON.stringify(id));
      }
    }
  });
});

describe('isProcessName and isInheritable', () => {
  // The SDK's test `manifest` holds `Manifest::parse` to the same fixture.
  it('judge the names of the SDK fixture as the host does', () => {
    /** @type {Record<'inherited' | 'names', { accepted: string[], refused: string[] }>} */
    const fixture = JSON.parse(
      readFileSync(join(root, 'crates/mannie-plugin-sdk/tests/fixtures/plugin-processes.json'), 'utf8'),
    );

    for (const name of fixture.names.accepted) {
      assert.equal(isProcessName(name), true, JSON.stringify(name));
    }
    for (const name of fixture.names.refused) {
      assert.equal(isProcessName(name), false, JSON.stringify(name));
    }
    for (const name of fixture.inherited.accepted) {
      assert.equal(isInheritable(name, rules), true, JSON.stringify(name));
    }
    for (const name of fixture.inherited.refused) {
      assert.equal(isInheritable(name, rules), false, JSON.stringify(name));
    }
  });
});

describe('prefixesNest', () => {
  it('nests a prefix with the prefixes that begin with it and an underscore', () => {
    assert.equal(prefixesNest('log', 'log_viewer'), true);
    assert.equal(prefixesNest('log_viewer', 'log'), true);
    assert.equal(prefixesNest('log', 'log'), true);
    assert.equal(prefixesNest('image', 'images'), false);
  });
});

describe('checkManifest', () => {
  it('accepts a manifest with the required fields', () => {
    const checked = checkManifest(MINIMAL, rules);

    assert.deepEqual(checked.problems, []);
    assert.equal(checked.manifest?.id, 'notes');
    assert.equal(checked.manifest?.rust.entry, 'mannie-notes');
  });

  it('refuses a manifest for another SDK major before its other fields', () => {
    const problems = problemsOf('id = "notes"\nsdk = 2\nunknown = true\n');

    assert.deepEqual(problems, ['the manifest names major 2 of the plugin SDK, and this SDK is major 1']);
  });

  it('refuses unknown and missing fields with their names', () => {
    assert.deepEqual(problemsOf(MINIMAL.replace('order = 30', 'order = 30\ncolour = "red"')), [
      'the manifest has the unknown field colour',
    ]);
    assert.deepEqual(problemsOf(withMore('[rust]\nlanguage = "rust"')).length, 1);
    assert.deepEqual(problemsOf('id = "notes"\nsdk = 1\norder = 30\n'), ['the manifest is missing the field rust']);
    assert.deepEqual(problemsOf(withMore('[crates]\nshared = ["tonic"]')), ['crates has the unknown field shared']);
  });

  it('refuses values of the wrong type or range', () => {
    assert.deepEqual(problemsOf(MINIMAL.replace('order = 30', 'order = -1')), ['order must be from 0 to 4294967295']);
    assert.deepEqual(problemsOf(MINIMAL.replace('order = 30', 'order = 4294967296')), [
      'order must be from 0 to 4294967295',
    ]);
    assert.deepEqual(problemsOf(MINIMAL.replace('order = 30', 'order = "first"')), [
      'order must be of type integer, not string',
    ]);
  });

  it('refuses text that is not TOML', () => {
    const [problem] = problemsOf('id = ');

    assert.match(problem, /^the manifest is not valid TOML/u);
  });

  it('refuses an ID that breaks the rules, with the rule', () => {
    assert.match(problemsOf(MINIMAL.replace('"notes"', '"Notes"'))[0], /is not a plugin ID/u);
    assert.match(problemsOf(MINIMAL.replace('"notes"', '"window"'))[0], /is reserved/u);
    assert.match(problemsOf(MINIMAL.replace('"notes"', '"tauri-plugin-notes"'))[0], /starts with "tauri-plugin-"/u);
    assert.match(problemsOf(MINIMAL.replace('"notes"', '"runtime-extra"'))[0], /runtime_\*/u);
  });

  it('refuses an unknown service and another major of a service', () => {
    assert.deepEqual(problemsOf(withMore('[services]\nweather = 1')), [
      'the manifest declares the service "weather", which the plugin SDK does not have',
    ]);
    assert.deepEqual(problemsOf(withMore('[services]\nstore = 2')), [
      'the manifest declares major 2 of the service store, which the plugin SDK does not have',
    ]);
  });

  it('refuses bad secret names, a name in both lists and secrets without the service', () => {
    const secrets = (lists) => withMore(`[services]\nsecrets = 1\n\n[secrets]\n${lists}`);

    assert.match(problemsOf(secrets('entries = ["a//b"]'))[0], /is not a secret's name/u);
    assert.match(problemsOf(secrets('entries = ["key"]\nlegacy = ["key"]'))[0], /listed both under entries/u);
    assert.deepEqual(problemsOf(withMore('[secrets]\nentries = ["key"]')), [
      'the manifest lists secrets but does not declare the service secrets',
    ]);
  });

  it('accepts keyed families and refuses a bad family, a fixed name inside one and a family without the service', () => {
    const secrets = (lists) => withMore(`[services]\nsecrets = 1\n\n[secrets]\n${lists}`);

    assert.deepEqual(problemsOf(secrets('keyed = ["host"]\nentries = ["hostname"]')), []);
    assert.match(problemsOf(secrets('keyed = ["host/"]'))[0], /is not a secret's name/u);
    assert.deepEqual(problemsOf(secrets('keyed = ["host"]\nentries = ["host/main"]')), [
      'the secret "host/main" is a keyed family, or an entry of one',
    ]);
    assert.deepEqual(problemsOf(secrets('keyed = ["host"]\nlegacy = ["host"]')), [
      'the secret "host" is a keyed family, or an entry of one',
    ]);
    assert.deepEqual(problemsOf(withMore('[secrets]\nkeyed = ["host"]')), [
      'the manifest lists secrets but does not declare the service secrets',
    ]);
  });

  it('accepts programs and refuses an unknown field of a program', () => {
    const programs = withMore(
      '[[programs]]\npackage = "mannie-notes-sync"\nheadless = true\nsmoke = { unix = "ci/smoke.sh" }',
    );
    const unknown = withMore('[[programs]]\npackage = "mannie-notes-sync"\nsigning = true');
    const wrongType = withMore('[[programs]]\npackage = "mannie-notes-sync"\nsmoke = { unix = 1 }');

    assert.deepEqual(problemsOf(programs), []);
    assert.deepEqual(problemsOf(unknown), ['programs[0] has the unknown field signing']);
    assert.deepEqual(problemsOf(wrongType), ['programs[0].smoke.unix must be of type string or null, not integer']);
  });

  it('accepts the crates that only the plugin may use', () => {
    const checked = checkManifest(withMore('[crates]\nexclusive = ["tonic"]'), rules);

    assert.deepEqual(checked.manifest?.crates?.exclusive, ['tonic']);
  });

  it('accepts native grants with reasons and refuses an unknown category and a blank reason, as the SDK does', () => {
    const checked = checkManifest(withMore('[native]\nsockets = "The hub connects to workers."'), rules);

    assert.deepEqual(checked.problems, []);
    assert.equal(checked.manifest?.native?.sockets, 'The hub connects to workers.');
    assert.deepEqual(problemsOf(withMore('[native]\nthreads = "A sync thread."')), [
      'native has the unknown field threads',
    ]);
    assert.deepEqual(problemsOf(withMore('[native]\nprocesses = ""')), [
      'the manifest grants processes under [native] without a reason',
    ]);
    assert.deepEqual(problemsOf(withMore('[native]\nsockets = "Connects."\nfile-writes = " \\t\u00a0"')), [
      'the manifest grants file-writes under [native] without a reason',
    ]);
  });

  it('accepts installed programs and refuses what the SDK refuses, in its order', () => {
    /** @param {string} table the lines of the table of the program git */
    const running = (table) => withMore(`[services]\nprocesses = 1\n\n[processes.git]\n${table}`);
    const checked = checkManifest(
      running('reason = "Reads the history."\nfixed = ["-c", "gc.auto=0"]\ninherit = ["HOME"]'),
      rules,
    );

    assert.deepEqual(checked.problems, []);
    assert.deepEqual(checked.manifest?.processes?.git.fixed, ['-c', 'gc.auto=0']);
    assert.deepEqual(problemsOf(running('reason = "Runs it."\nshell = true')), [
      'processes.git has the unknown field shell',
    ]);
    assert.deepEqual(problemsOf(running('fixed = []')), ['processes.git is missing the field reason']);
    assert.deepEqual(problemsOf(withMore('[services]\nprocesses = 1\n\n[processes."../git"]\nreason = "Runs it."')), [
      '"../git" is not the name of an installed program, which is a letter or a digit, then letters, digits, ".", "_" and "-", at most 64 characters, never ending in a dot',
    ]);
    assert.deepEqual(problemsOf(running('reason = " \\t"')), [
      'the manifest runs the program git under [processes] without a reason',
    ]);
    assert.deepEqual(problemsOf(running('reason = "Runs it."\nfixed = ["a\\u0000b"]')), [
      'a fixed argument of the program git holds a NUL character',
    ]);
    assert.deepEqual(problemsOf(running('reason = "Runs it."\ninherit = ["GITHUB_TOKEN"]')), [
      'the program git inherits "GITHUB_TOKEN", which is not a variable\'s name, looks like a secret\'s or loads libraries',
    ]);
    assert.deepEqual(problemsOf(withMore('[processes.git]\nreason = "Runs it."')), [
      'the manifest lists programs under [processes] but does not declare the service processes',
    ]);
    // A program's own checks come before the service that it needs.
    assert.match(problemsOf(withMore('[processes."../git"]\nreason = "Runs it."'))[0], /is not the name/u);
  });
});

describe('memberFolders', () => {
  it('expands a pattern in a member to the folders that it matches', () => {
    const folder = tree({ 'crates/a/Cargo.toml': '', 'crates/b/x.txt': '', 'worker/Cargo.toml': '' });

    assert.deepEqual(memberFolders(folder, 'crates/*').sort(), ['crates/a', 'crates/b']);
    assert.deepEqual(memberFolders(folder, 'worker'), ['worker']);
  });
});

describe('findPlugins', () => {
  it('finds the plugins and the fixture plugins in the order of their starts', () => {
    const folder = tree({
      ...plugin('plugins/notes', manifestOf('notes', 20)),
      ...plugin('plugins/calendar', manifestOf('calendar', 20)),
      ...plugin('plugins/mail', manifestOf('mail', 10)),
      ...plugin('tests/fixtures/plugins/example', manifestOf('example', 90)),
    });

    const found = findPlugins(folder, rules);

    assert.deepEqual(found.problems, []);
    assert.deepEqual(
      found.plugins.map((entry) => [entry.id, entry.folder, entry.validation]),
      [
        ['mail', 'plugins/mail', false],
        ['calendar', 'plugins/calendar', false],
        ['notes', 'plugins/notes', false],
        ['example', 'tests/fixtures/plugins/example', true],
      ],
    );
  });

  it('refuses a manifest whose ID is not its folder', () => {
    const folder = tree(plugin('plugins/notes', manifestOf('notes-x', 10)));

    assert.deepEqual(findPlugins(folder, rules).problems, [
      'plugins/notes/plugin.toml: the ID "notes-x" must equal the folder\'s name, "notes"',
    ]);
  });

  it("refuses a Tauri plugin's name", () => {
    const folder = tree(plugin('plugins/window', manifestOf('window', 10)));

    const [problem] = findPlugins(folder, rules).problems;

    assert.match(problem, /^plugins\/window\/plugin\.toml: "window" is reserved/u);
  });

  it('refuses two plugins whose prefixes nest', () => {
    const folder = tree({
      ...plugin('plugins/log', manifestOf('log', 10)),
      ...plugin('plugins/log-viewer', manifestOf('log-viewer', 20)),
    });

    assert.deepEqual(findPlugins(folder, rules).problems, [
      'the plugins log and log-viewer have prefixes that nest, so their tables and message keys could not be told apart',
    ]);
  });

  it('refuses a folder without a manifest and a manifest without a workspace', () => {
    const folder = tree({
      'plugins/leftover/ui/generated/bindings.ts': '',
      'plugins/notes/plugin.toml': manifestOf('notes', 10),
    });

    assert.deepEqual(findPlugins(folder, rules).problems, [
      "plugins/leftover has no plugin.toml; add the plugin's manifest or remove the folder",
      "plugins/notes has a plugin.toml but no Cargo.toml, the plugin's Cargo workspace",
    ]);
  });

  it('names a member folder without Cargo.toml', () => {
    const folder = tree({
      ...plugin('plugins/notes', manifestOf('notes', 10)),
      'plugins/notes/crates/empty/.keep': '',
    });

    assert.deepEqual(findPlugins(folder, rules).problems, [
      "plugins/notes/crates/empty is a member of the plugin's workspace but has no Cargo.toml; add one or remove the folder",
    ]);
  });

  it("refuses a plugin's own Clippy configuration", () => {
    const folder = tree({
      ...plugin('plugins/notes', manifestOf('notes', 10)),
      'plugins/notes/crates/entry/clippy.toml': '',
    });

    assert.deepEqual(findPlugins(folder, rules).problems, [
      "plugins/notes/crates/entry/clippy.toml: a plugin's crates are linted with the core's Clippy configuration; remove the file",
    ]);
  });

  it('refuses a manifest for an SDK major that is too new', () => {
    const folder = tree(plugin('plugins/notes', manifestOf('notes', 10).replace('sdk = 1', 'sdk = 2')));

    assert.deepEqual(findPlugins(folder, rules).problems, [
      'plugins/notes/plugin.toml: the manifest names major 2 of the plugin SDK, and this SDK is major 1',
    ]);
  });
});
