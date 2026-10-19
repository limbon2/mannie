// @ts-check
// The release steps that run in Node, so they work the same on every runner
// (docs/engineering/releasing.md):
//
//   node scripts/release.mjs version                   print the workspace version
//   node scripts/release.mjs prepare <version>         set the version in every workspace, move the plugins' changes into
//                                                      the changelog and date its section
//   node scripts/release.mjs notes <version>           print the changelog section of a version
//   node scripts/release.mjs checksums <folder>        write SHA256SUMS for the files in a folder
//   node scripts/release.mjs keygen <folder>           write a new Ed25519 signing key pair
//   node scripts/release.mjs sign <file>...            sign files with MANNIE_RELEASE_SIGNING_KEY
//   node scripts/release.mjs verify <public key> <file>...  check the signatures of files
//   node scripts/release.mjs fingerprint <public key>  print the SHA-256 of a public key
//
// Signatures are Ed25519 over the whole file, stored base64-encoded beside it
// as <file>.sig. The private key is a PKCS #8 PEM that lives only in the CI
// secret MANNIE_RELEASE_SIGNING_KEY; the public key is an SPKI PEM. The
// fingerprint is the SHA-256 of the key's SubjectPublicKeyInfo, which the
// worker's `self-check --json` reports for the key built into it.

import { createHash, createPrivateKey, createPublicKey, generateKeyPairSync, sign, verify } from 'node:crypto';
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { basename, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { committedWorkspaces, manifestOf } from './plugins/lib/cargo.mjs';
import { findPlugins, prefixOf, readRules } from './plugins/lib/manifests.mjs';

/** A SemVer version without build metadata, such as 0.1.0 or 0.2.0-rc.1. */
const VERSION = /^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?$/;

/** The heading of the changelog section that collects the next release's changes. */
const UNRELEASED_HEADING = '## Unreleased';

/**
 * Checks a version argument.
 * @param {string | undefined} version
 * @returns {string}
 */
export function checkedVersion(version) {
  const isVersion = version !== undefined && VERSION.test(version);
  if (!isVersion) {
    throw new Error(`${JSON.stringify(version)} is not a version such as 0.1.0`);
  }
  return version;
}

/** The version line of [workspace.package] in Cargo.toml. */
const CARGO_VERSION = /^\[workspace\.package\][^[]*?^version = "([^"]*)"/m;

/**
 * The version of the Cargo workspace.
 * @param {string} manifest
 * @returns {string}
 */
export function cargoVersion(manifest) {
  const version = CARGO_VERSION.exec(manifest)?.[1];
  if (version === undefined) {
    throw new Error('Cargo.toml has no version in [workspace.package]');
  }
  return version;
}

/**
 * Sets the version of the Cargo workspace in Cargo.toml.
 * @param {string} manifest
 * @param {string} version
 * @returns {string}
 */
export function withCargoVersion(manifest, version) {
  cargoVersion(manifest);
  return manifest.replace(CARGO_VERSION, (match) => match.replace(/version = "[^"]*"$/, `version = "${version}"`));
}

/**
 * Sets the version of a package.json, keeping its layout.
 * @param {string} manifest
 * @param {string} version
 * @returns {string}
 */
export function withPackageVersion(manifest, version) {
  const field = /^ {2}"version": "[^"]*"/m;
  if (!field.test(manifest)) {
    throw new Error('package.json has no top-level version');
  }
  return manifest.replace(field, `  "version": "${version}"`);
}

/**
 * Turns the Unreleased section into the release's section and starts a new,
 * empty Unreleased section above it.
 * @param {string} changelog
 * @param {string} version
 * @param {string} date in the form YYYY-MM-DD
 * @returns {string}
 */
export function withReleasedChangelog(changelog, version, date) {
  const lines = changelog.split('\n');
  const start = lines.indexOf(UNRELEASED_HEADING);
  if (start === -1) {
    throw new Error(`CHANGELOG.md has no "${UNRELEASED_HEADING}" section`);
  }
  const alreadyReleased = lines.some((line) => line.startsWith(`## ${version} `));
  if (alreadyReleased) {
    throw new Error(`CHANGELOG.md already has a section for ${version}`);
  }
  const body = sectionBody(lines, start);
  const hasChanges = body.some((line) => line.startsWith('- '));
  if (!hasChanges) {
    throw new Error('the Unreleased section of CHANGELOG.md lists no changes');
  }
  lines.splice(start, 1, UNRELEASED_HEADING, '', `## ${version} - ${date}`);
  return lines.join('\n');
}

/**
 * The lines of a section, from after its heading to the next section.
 * @param {string[]} lines
 * @param {number} heading
 * @returns {string[]}
 */
function sectionBody(lines, heading) {
  const rest = lines.slice(heading + 1);
  const next = rest.findIndex((line) => line.startsWith('## '));
  return next === -1 ? rest : rest.slice(0, next);
}

/** The kinds of change of a changelog section, in Keep a Changelog's order. */
const CHANGE_KINDS = ['Added', 'Changed', 'Deprecated', 'Removed', 'Fixed', 'Security'];

/**
 * A plugin's changelog, `plugins/<id>/CHANGELOG.md`, whose unreleased
 * changes a release moves into the root's changelog.
 * @typedef {object} PluginChangelog
 * @property {string} name the plugin's name, under which its changes appear
 * @property {string} path for messages
 * @property {string} text
 */

/**
 * The Unreleased section of a changelog: where it starts and ends among the
 * lines, and its changes by their kinds, without blank lines.
 * @param {string[]} lines
 * @param {string} path for messages
 * @returns {{ changes: Map<string, string[]>, end: number, start: number }}
 */
function unreleasedSection(lines, path) {
  const start = lines.indexOf(UNRELEASED_HEADING);
  if (start === -1) {
    throw new Error(`${path} has no "${UNRELEASED_HEADING}" section`);
  }
  const body = sectionBody(lines, start);
  /** @type {Map<string, string[]>} */
  const changes = new Map();
  /** @type {string[] | null} */
  let current = null;
  for (const line of body.filter((candidate) => candidate.trim() !== '')) {
    const kind = /^### (.+)$/u.exec(line)?.[1];
    if (kind === undefined) {
      if (current === null) {
        throw new Error(`${path} lists ${JSON.stringify(line)} outside a kind of change such as "### Added"`);
      }
      current.push(line);
    } else if (CHANGE_KINDS.includes(kind)) {
      current = changes.get(kind) ?? [];
      changes.set(kind, current);
    } else {
      throw new Error(
        `${path} has the kind of change ${JSON.stringify(kind)}, which is not one of ${CHANGE_KINDS.join(', ')}`,
      );
    }
  }
  return { changes, end: start + 1 + body.length, start };
}

/**
 * The root's changelog with each plugin's unreleased changes in its
 * Unreleased section, under their kinds of change and the plugin's name, and
 * the plugins' changelogs with their Unreleased sections emptied (ADR-0035,
 * decision 11). Without a plugin's change, nothing changes.
 * @param {string} changelog the root's
 * @param {PluginChangelog[]} plugins in the order of the plugins
 * @returns {{ changelog: string, plugins: string[] }}
 */
export function withPluginChanges(changelog, plugins) {
  const unreleased = plugins.map((plugin) => {
    const lines = plugin.text.split('\n');
    return { lines, plugin, section: unreleasedSection(lines, plugin.path) };
  });
  const hasPluginChanges = unreleased.some(({ section }) => section.changes.size > 0);
  if (!hasPluginChanges) {
    return { changelog, plugins: plugins.map((plugin) => plugin.text) };
  }
  const lines = changelog.split('\n');
  const root = unreleasedSection(lines, 'CHANGELOG.md');
  /** @type {string[]} */
  const merged = [UNRELEASED_HEADING, ''];
  for (const kind of CHANGE_KINDS) {
    const own = root.changes.get(kind) ?? [];
    const fromPlugins = unreleased.flatMap(({ plugin, section }) => {
      const changes = section.changes.get(kind) ?? [];
      return changes.length > 0 ? [[`#### ${plugin.name}`, '', ...changes]] : [];
    });
    const parts = own.length > 0 ? [own, ...fromPlugins] : fromPlugins;
    if (parts.length > 0) {
      merged.push(`### ${kind}`, '', ...parts.flatMap((part) => [...part, '']));
    }
  }
  const emptied = unreleased.map(({ lines: pluginLines, section }) =>
    [...pluginLines.slice(0, section.start + 1), '', ...pluginLines.slice(section.end)].join('\n'),
  );
  return {
    changelog: [...lines.slice(0, root.start), ...merged, ...lines.slice(root.end)].join('\n'),
    plugins: emptied,
  };
}

/**
 * The notes of a release: its changelog section without the heading.
 * @param {string} changelog
 * @param {string} version
 * @returns {string}
 */
export function releaseNotes(changelog, version) {
  const lines = changelog.split('\n');
  const heading = lines.findIndex((line) => line.startsWith(`## ${version} - `));
  if (heading === -1) {
    throw new Error(`CHANGELOG.md has no section for ${version}`);
  }
  return `${sectionBody(lines, heading).join('\n').trim()}\n`;
}

/**
 * The SHA256SUMS file for the given files, in the format `sha256sum --check` reads.
 * @param {{ name: string, content: Buffer }[]} files
 * @returns {string}
 */
export function checksums(files) {
  const sorted = [...files].sort((left, right) => left.name.localeCompare(right.name));
  return sorted.map((file) => `${createHash('sha256').update(file.content).digest('hex')}  ${file.name}\n`).join('');
}

/**
 * A new signing key pair as PEM text.
 * @returns {{ privateKey: string, publicKey: string }}
 */
export function newKeyPair() {
  return generateKeyPairSync('ed25519', {
    privateKeyEncoding: { format: 'pem', type: 'pkcs8' },
    publicKeyEncoding: { format: 'pem', type: 'spki' },
  });
}

/**
 * The base64 Ed25519 signature of `content`.
 * @param {Buffer} content
 * @param {string} privateKeyPem
 * @returns {string}
 */
export function signature(content, privateKeyPem) {
  return sign(null, content, createPrivateKey(privateKeyPem)).toString('base64');
}

/**
 * Whether `signatureText` is a valid signature of `content`.
 * @param {Buffer} content
 * @param {string} signatureText
 * @param {string} publicKeyPem
 * @returns {boolean}
 */
export function isValidSignature(content, signatureText, publicKeyPem) {
  return verify(null, content, createPublicKey(publicKeyPem), Buffer.from(signatureText.trim(), 'base64'));
}

/**
 * The SHA-256 of a public key's SubjectPublicKeyInfo in hexadecimal, as
 * `mannie-worker self-check --json` reports it.
 * @param {string} publicKeyPem
 * @returns {string}
 */
export function keyFingerprint(publicKeyPem) {
  const spki = createPublicKey(publicKeyPem).export({ format: 'der', type: 'spki' });
  return createHash('sha256').update(spki).digest('hex');
}

/** @param {string} path */
function readText(path) {
  return readFileSync(path, 'utf8');
}

/**
 * The changelogs of the plugins that releases ship, in the plugins' order,
 * each with the plugin's name from its English catalog. A plugin without a
 * changelog has no changes to move.
 * @param {string} repository
 * @returns {PluginChangelog[]}
 */
export function pluginChangelogs(repository) {
  const found = findPlugins(repository, readRules(repository));
  if (found.problems.length > 0) {
    throw new Error(found.problems.join('\n'));
  }
  return found.plugins
    .filter((plugin) => !plugin.validation)
    .flatMap((plugin) => {
      const path = `${plugin.folder}/CHANGELOG.md`;
      if (!existsSync(join(repository, path))) {
        return [];
      }
      /** @type {Record<string, unknown>} */
      const catalog = JSON.parse(readText(join(repository, plugin.folder, 'messages', 'en.json')));
      const key = `${prefixOf(plugin.id)}_plugin_name`;
      const name = catalog[key];
      if (typeof name !== 'string') {
        throw new Error(
          `${plugin.folder}/messages/en.json has no ${key}, the plugin's name in its changelog's changes`,
        );
      }
      return [{ name, path, text: readText(join(repository, path)) }];
    });
}

/**
 * Runs one command of the script.
 * @param {string[]} argumentList
 * @param {string} repository
 */
function run(argumentList, repository) {
  const [command, ...rest] = argumentList;
  switch (command) {
    case 'version': {
      console.log(cargoVersion(readText(join(repository, 'Cargo.toml'))));
      return;
    }
    case 'prepare': {
      const version = checkedVersion(rest[0]);
      const date = new Date().toISOString().slice(0, 10);
      // The changelogs first, so that a release without changes stops before
      // it writes anything. Each plugin's unreleased changes move into the
      // root's section under the plugin's name (ADR-0035, decision 11).
      const changelog = join(repository, 'CHANGELOG.md');
      const plugins = pluginChangelogs(repository);
      const folded = withPluginChanges(readText(changelog), plugins);
      const released = withReleasedChangelog(folded.changelog, version, date);
      // One version for every artifact (ADR-0018): the root's workspace and
      // each plugin's carry it (ADR-0035, decision 5).
      for (const workspace of committedWorkspaces(repository)) {
        const cargo = join(repository, manifestOf(workspace));
        writeFileSync(cargo, withCargoVersion(readText(cargo), version));
      }
      const desktop = join(repository, 'apps', 'desktop', 'package.json');
      writeFileSync(desktop, withPackageVersion(readText(desktop), version));
      writeFileSync(changelog, released);
      for (const [index, plugin] of plugins.entries()) {
        writeFileSync(join(repository, plugin.path), folded.plugins[index] ?? plugin.text);
      }
      console.log(`release: set the version to ${version} and dated its changelog section ${date}`);
      return;
    }
    case 'notes': {
      process.stdout.write(releaseNotes(readText(join(repository, 'CHANGELOG.md')), checkedVersion(rest[0])));
      return;
    }
    case 'checksums': {
      const folder = rest[0];
      if (folder === undefined) {
        throw new Error('name the folder of the release files');
      }
      const names = readdirSync(folder).filter((name) => {
        const isOwnOutput = name === 'SHA256SUMS' || name.endsWith('.sig');
        const isFile = statSync(join(folder, name)).isFile();
        return isFile && !isOwnOutput;
      });
      const files = names.map((name) => ({ content: readFileSync(join(folder, name)), name }));
      writeFileSync(join(folder, 'SHA256SUMS'), checksums(files));
      console.log(`release: wrote the checksums of ${String(files.length)} files to ${join(folder, 'SHA256SUMS')}`);
      return;
    }
    case 'keygen': {
      const folder = rest[0];
      if (folder === undefined) {
        throw new Error('name the folder for the new key pair');
      }
      const pair = newKeyPair();
      writeFileSync(join(folder, 'mannie-release.key'), pair.privateKey, { flag: 'wx', mode: 0o600 });
      writeFileSync(join(folder, 'mannie-release.pub'), pair.publicKey, { flag: 'wx' });
      console.log(
        `release: wrote mannie-release.key and mannie-release.pub to ${folder}. Store the private key as the CI secret MANNIE_RELEASE_SIGNING_KEY, keep a copy offline, and delete the file`,
      );
      return;
    }
    case 'sign': {
      const privateKey = process.env.MANNIE_RELEASE_SIGNING_KEY ?? '';
      if (privateKey === '') {
        throw new Error('MANNIE_RELEASE_SIGNING_KEY holds no signing key');
      }
      for (const file of rest) {
        writeFileSync(`${file}.sig`, `${signature(readFileSync(file), privateKey)}\n`);
        console.log(`release: signed ${basename(file)}`);
      }
      return;
    }
    case 'verify': {
      const [publicKeyFile, ...files] = rest;
      if (publicKeyFile === undefined) {
        throw new Error('name the public key file');
      }
      const publicKey = readText(publicKeyFile);
      const invalid = files.filter((file) => !isValidSignature(readFileSync(file), readText(`${file}.sig`), publicKey));
      for (const file of invalid) {
        console.error(`error: the signature of ${file} is not valid`);
      }
      if (invalid.length > 0) {
        process.exitCode = 1;
      } else {
        console.log(`release: the signatures of ${String(files.length)} files are valid`);
      }
      return;
    }
    case 'fingerprint': {
      const publicKeyFile = rest[0];
      if (publicKeyFile === undefined) {
        throw new Error('name the public key file');
      }
      console.log(keyFingerprint(readText(publicKeyFile)));
      return;
    }
    default: {
      throw new Error(
        `unknown command ${JSON.stringify(command)}: use version, prepare, notes, checksums, keygen, sign, verify or fingerprint`,
      );
    }
  }
}

const isMain = process.argv[1] === fileURLToPath(import.meta.url);
if (isMain) {
  try {
    run(process.argv.slice(2), join(fileURLToPath(import.meta.url), '..', '..'));
  } catch (error) {
    console.error(`error: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}
