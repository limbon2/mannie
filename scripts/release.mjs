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
//   node scripts/release.mjs updater-key <public key>  print a public key as the desktop app's updater reads it
//   node scripts/release.mjs manifest <folder> <version> <download URL>
//                                                      sign the desktop installers in a folder with
//                                                      MANNIE_RELEASE_SIGNING_KEY and write latest.json
//   node scripts/release.mjs verify-manifest <public key> <folder>
//                                                      check latest.json against the installers in a folder
//
// Signatures are Ed25519 over the whole file, stored base64-encoded beside it
// as <file>.sig. The private key is a PKCS #8 PEM that lives only in the CI
// secret MANNIE_RELEASE_SIGNING_KEY; the public key is an SPKI PEM. The
// fingerprint is the SHA-256 of the key's SubjectPublicKeyInfo, which the
// worker's `self-check --json` reports for the key built into it.
//
// The desktop app's updater reads minisign signatures instead (ADR-0063). The
// same key signs the BLAKE2b-512 hash of an installer, and a second signature
// covers the first one and a trusted comment, which names the file and its
// version. latest.json, the update manifest, holds the URL and the signature
// of each installer of a release, and the app reads the newest release's.

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

/** The file of a release that the desktop app's updater reads (ADR-0063). */
export const UPDATE_MANIFEST = 'latest.json';

/**
 * The desktop installers of a release: how each one's file name ends after
 * `mannie-<version>`, and its key in the update manifest, which is the
 * updater's `<system>-<architecture>-<installer>`.
 */
const INSTALLERS = [
  { ending: '-windows-x86_64-setup.exe', platform: 'windows-x86_64-nsis' },
  { ending: '-linux-amd64.deb', platform: 'linux-x86_64-deb' },
  { ending: '-linux-x86_64.rpm', platform: 'linux-x86_64-rpm' },
];

const MEBIBYTE = 1024 * 1024;

/**
 * The largest installer that a desktop app downloads as an update, which is
 * `MAX_PACKAGE_BYTES` of apps/desktop/core/src/updates.rs. A larger one
 * would update no installed app, so a release with one stops before it is
 * published.
 */
export const MAX_INSTALLER_BYTES = 256 * MEBIBYTE;

/**
 * @param {string} name an installer's file name
 * @returns {string} what is wrong with an installer above the limit
 */
function tooLarge(name) {
  const limit = String(MAX_INSTALLER_BYTES / MEBIBYTE);
  return `${name} is larger than the ${limit} MiB that the desktop app downloads as an update`;
}

/** What a minisign signature's trusted comment line starts with. */
const TRUSTED_COMMENT_PREFIX = 'trusted comment: ';

/**
 * The ID that minisign keys and signatures carry so that a verifier can tell
 * keys apart: the first eight bytes of the key's fingerprint.
 * @param {string} publicKeyPem
 * @returns {Buffer}
 */
function minisignKeyId(publicKeyPem) {
  return Buffer.from(keyFingerprint(publicKeyPem), 'hex').subarray(0, 8);
}

/**
 * A public key in minisign's text form: a comment line, then the base64 of
 * "Ed", the key ID and the 32 bytes of the key.
 * @param {string} publicKeyPem
 * @returns {string}
 */
export function minisignPublicKey(publicKeyPem) {
  const jwk = createPublicKey(publicKeyPem).export({ format: 'jwk' });
  const rawKey = jwk.crv === 'Ed25519' ? jwk.x : undefined;
  if (rawKey === undefined) {
    throw new Error('the public key is not an Ed25519 key');
  }
  const key = Buffer.concat([Buffer.from('Ed'), minisignKeyId(publicKeyPem), Buffer.from(rawKey, 'base64url')]);
  return `untrusted comment: Mannie's release key\n${key.toString('base64')}\n`;
}

/**
 * A public key as `plugins.updater.pubkey` of tauri.conf.json holds it: the
 * base64 of its minisign text.
 * @param {string} publicKeyPem
 * @returns {string}
 */
export function updaterKey(publicKeyPem) {
  return Buffer.from(minisignPublicKey(publicKeyPem)).toString('base64');
}

/**
 * What a signature says about its file, which its second signature covers.
 * The updater refuses an installer whose signed version is not the version
 * that the manifest announces, so an older signed installer cannot pass as a
 * newer release.
 * @typedef {object} SignedFile
 * @property {string} file the file's name
 * @property {number} timestamp when it was signed, in seconds since the Unix epoch
 * @property {string} version the release's version
 */

/**
 * The minisign signature of `content` in its text form: a comment line, the
 * base64 of "ED", the key ID and the signature of the content's BLAKE2b-512
 * hash, the trusted comment, and the base64 of the signature of the first
 * signature followed by the trusted comment.
 * @param {Buffer} content
 * @param {string} privateKeyPem
 * @param {SignedFile} signed
 * @returns {string}
 */
export function minisignSignature(content, privateKeyPem, signed) {
  const privateKey = createPrivateKey(privateKeyPem);
  const publicKeyPem = createPublicKey(privateKey).export({ format: 'pem', type: 'spki' }).toString();
  const trustedComment = `timestamp:${String(signed.timestamp)}\tfile:${signed.file}\tversion:${signed.version}`;
  // A line break would end the comment early, and a tab would start a field.
  const isOneLineOfThreeFields = !/[\n\r]/u.test(trustedComment) && trustedComment.split('\t').length === 3;
  if (!isOneLineOfThreeFields) {
    throw new Error(`${JSON.stringify(signed.file)} or ${JSON.stringify(signed.version)} has a tab or a line break`);
  }
  const hash = createHash('blake2b512').update(content).digest();
  const fileSignature = sign(null, hash, privateKey);
  const commentSignature = sign(null, Buffer.concat([fileSignature, Buffer.from(trustedComment)]), privateKey);
  return [
    "untrusted comment: signature from Mannie's release key",
    Buffer.concat([Buffer.from('ED'), minisignKeyId(publicKeyPem), fileSignature]).toString('base64'),
    `${TRUSTED_COMMENT_PREFIX}${trustedComment}`,
    commentSignature.toString('base64'),
    '',
  ].join('\n');
}

/**
 * The fields of a minisign signature's trusted comment, when the signature
 * is a valid one of `content` by the key, and null otherwise. It checks what
 * the updater checks: the key ID, the signature of the content's hash and
 * the signature of the trusted comment.
 * @param {Buffer} content
 * @param {string} signatureText
 * @param {string} publicKeyPem
 * @returns {Map<string, string> | null}
 */
export function verifiedTrustedComment(content, signatureText, publicKeyPem) {
  const [, signatureLine = '', commentLine = '', commentSignatureLine = ''] = signatureText.split(/\r?\n/u);
  const signed = Buffer.from(signatureLine, 'base64');
  const isOfThisKey =
    signed.length === 74 &&
    signed.subarray(0, 2).toString('latin1') === 'ED' &&
    signed.subarray(2, 10).equals(minisignKeyId(publicKeyPem));
  if (!isOfThisKey || !commentLine.startsWith(TRUSTED_COMMENT_PREFIX)) {
    return null;
  }
  const publicKey = createPublicKey(publicKeyPem);
  const fileSignature = signed.subarray(10);
  const trustedComment = commentLine.slice(TRUSTED_COMMENT_PREFIX.length);
  const hash = createHash('blake2b512').update(content).digest();
  const signedComment = Buffer.concat([fileSignature, Buffer.from(trustedComment)]);
  const isValid =
    verify(null, hash, publicKey, fileSignature) &&
    verify(null, signedComment, publicKey, Buffer.from(commentSignatureLine, 'base64'));
  if (!isValid) {
    return null;
  }
  /** @type {Map<string, string>} */
  const fields = new Map();
  for (const field of trustedComment.split('\t')) {
    const separator = field.indexOf(':');
    if (separator !== -1) {
      fields.set(field.slice(0, separator), field.slice(separator + 1));
    }
  }
  return fields;
}

/**
 * A release as its update manifest describes it.
 * @typedef {object} ManifestRelease
 * @property {string} downloadUrl the address that the release's files download from, without a file's name
 * @property {string} notes the release's changelog section
 * @property {string} privateKeyPem the release key
 * @property {Date} publishedAt when the manifest is written
 * @property {string} version
 */

/**
 * The update manifest of a release: its version, notes and date, and for
 * each desktop installer among `files` the address it downloads from and its
 * minisign signature, as the updater reads them. A release may leave a
 * platform out, and the desktop apps of that platform then find no package
 * in it. An installer above the size that the app downloads is refused.
 * @param {{ name: string, content: Buffer }[]} files
 * @param {ManifestRelease} release
 * @returns {string}
 */
export function updateManifest(files, release) {
  const version = checkedVersion(release.version);
  const downloadUrl = release.downloadUrl.replace(/\/+$/u, '');
  if (!downloadUrl.startsWith('https://')) {
    throw new Error(`${JSON.stringify(release.downloadUrl)} is not an HTTPS address`);
  }
  const timestamp = Math.floor(release.publishedAt.getTime() / 1000);
  /** @type {Record<string, { signature: string, url: string }>} */
  const platforms = {};
  for (const installer of INSTALLERS) {
    const name = `mannie-${version}${installer.ending}`;
    const file = files.find((candidate) => candidate.name === name);
    if (file !== undefined) {
      if (file.content.length > MAX_INSTALLER_BYTES) {
        throw new Error(tooLarge(name));
      }
      const signed = minisignSignature(file.content, release.privateKeyPem, { file: name, timestamp, version });
      platforms[installer.platform] = {
        signature: Buffer.from(signed).toString('base64'),
        url: `${downloadUrl}/${name}`,
      };
    }
  }
  if (Object.keys(platforms).length === 0) {
    throw new Error(`the release files have no desktop installer of ${version}`);
  }
  // The date has whole seconds, as the signatures' timestamps have.
  const publishedAt = new Date(timestamp * 1000).toISOString().replace('.000Z', 'Z');
  return `${JSON.stringify({ notes: release.notes, platforms, pub_date: publishedAt, version }, null, 2)}\n`;
}

/**
 * @param {unknown} value
 * @returns {string} the value when it is text, and no text otherwise
 */
function textOf(value) {
  return typeof value === 'string' ? value : '';
}

/**
 * @param {unknown} value
 * @returns {Record<string, unknown>} the value when it is an object with keys, and an empty one otherwise
 */
function recordOf(value) {
  const isObject = typeof value === 'object' && value !== null;
  return isObject && !Array.isArray(value) ? /** @type {Record<string, unknown>} */ (value) : {};
}

/**
 * What is wrong with an update manifest, for the release files it belongs
 * to: every installer it names must be among `files` under the manifest's
 * version, no larger than the app downloads, at an HTTPS address that ends
 * with the file's name, with a valid signature whose trusted comment names
 * that file and that version.
 * @param {string} manifestText
 * @param {string} publicKeyPem
 * @param {{ name: string, content: Buffer }[]} files
 * @returns {string[]}
 */
export function manifestProblems(manifestText, publicKeyPem, files) {
  /** @type {Record<string, unknown>} */
  let manifest;
  try {
    manifest = recordOf(JSON.parse(manifestText));
  } catch {
    return [`${UPDATE_MANIFEST} is not JSON`];
  }
  const version = textOf(manifest.version);
  const platforms = Object.entries(recordOf(manifest.platforms));
  if (!VERSION.test(version)) {
    return [`${UPDATE_MANIFEST} names no version such as 0.1.0`];
  }
  if (platforms.length === 0) {
    return [`${UPDATE_MANIFEST} names no installer`];
  }
  /** @type {string[]} */
  const problems = [];
  for (const [platform, value] of platforms) {
    const installer = INSTALLERS.find((candidate) => candidate.platform === platform);
    if (installer === undefined) {
      problems.push(`${platform} is not a platform of the desktop app`);
      continue;
    }
    const entry = recordOf(value);
    const name = `mannie-${version}${installer.ending}`;
    const url = textOf(entry.url);
    const isDownloadOfFile = url.startsWith('https://') && url.endsWith(`/${name}`);
    if (!isDownloadOfFile) {
      problems.push(`${platform}: the address is not an HTTPS address of ${name}`);
    }
    const file = files.find((candidate) => candidate.name === name);
    if (file === undefined) {
      problems.push(`${platform}: the release files have no ${name}`);
      continue;
    }
    if (file.content.length > MAX_INSTALLER_BYTES) {
      problems.push(`${platform}: ${tooLarge(name)}`);
    }
    const signatureText = Buffer.from(textOf(entry.signature), 'base64').toString();
    const comment = verifiedTrustedComment(file.content, signatureText, publicKeyPem);
    const isOfThisRelease = comment?.get('file') === name && comment.get('version') === version;
    if (comment === null) {
      problems.push(`${platform}: the signature of ${name} is not valid`);
    } else if (!isOfThisRelease) {
      problems.push(`${platform}: the signature is not one of ${name} in version ${version}`);
    }
  }
  return problems;
}

/**
 * The files of a folder with their content.
 * @param {string} folder
 * @returns {{ name: string, content: Buffer }[]}
 */
function filesIn(folder) {
  return readdirSync(folder)
    .filter((name) => statSync(join(folder, name)).isFile())
    .map((name) => ({ content: readFileSync(join(folder, name)), name }));
}

/**
 * The release key from the environment.
 * @returns {string}
 */
function signingKey() {
  const privateKey = process.env.MANNIE_RELEASE_SIGNING_KEY ?? '';
  if (privateKey === '') {
    throw new Error('MANNIE_RELEASE_SIGNING_KEY holds no signing key');
  }
  return privateKey;
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
      const files = filesIn(folder).filter((file) => {
        const isOwnOutput = file.name === 'SHA256SUMS' || file.name.endsWith('.sig');
        return !isOwnOutput;
      });
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
      const privateKey = signingKey();
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
    case 'updater-key': {
      const publicKeyFile = rest[0];
      if (publicKeyFile === undefined) {
        throw new Error('name the public key file');
      }
      console.log(updaterKey(readText(publicKeyFile)));
      return;
    }
    case 'manifest': {
      const [folder, versionArgument, downloadUrl] = rest;
      const lacksArgument = folder === undefined || downloadUrl === undefined;
      if (lacksArgument) {
        throw new Error(
          'name the folder of the release files, the version and the address that the files download from',
        );
      }
      const version = checkedVersion(versionArgument);
      const manifest = updateManifest(filesIn(folder), {
        downloadUrl,
        notes: releaseNotes(readText(join(repository, 'CHANGELOG.md')), version),
        privateKeyPem: signingKey(),
        publishedAt: new Date(),
        version,
      });
      writeFileSync(join(folder, UPDATE_MANIFEST), manifest);
      console.log(`release: signed the desktop installers of ${version} and wrote ${join(folder, UPDATE_MANIFEST)}`);
      return;
    }
    case 'verify-manifest': {
      const [publicKeyFile, folder] = rest;
      const lacksArgument = publicKeyFile === undefined || folder === undefined;
      if (lacksArgument) {
        throw new Error('name the public key file and the folder of the release files');
      }
      const manifest = readText(join(folder, UPDATE_MANIFEST));
      const problems = manifestProblems(manifest, readText(publicKeyFile), filesIn(folder));
      for (const problem of problems) {
        console.error(`error: ${problem}`);
      }
      if (problems.length > 0) {
        process.exitCode = 1;
      } else {
        console.log(`release: ${UPDATE_MANIFEST} names installers of the folder with valid signatures`);
      }
      return;
    }
    default: {
      throw new Error(
        `unknown command ${JSON.stringify(command)}: use version, prepare, notes, checksums, keygen, sign, verify, fingerprint, updater-key, manifest or verify-manifest`,
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
