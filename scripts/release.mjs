// @ts-check
// The release steps that run in Node, so they work the same on every runner
// (docs/engineering/releasing.md):
//
//   node scripts/release.mjs version                   print the workspace version
//   node scripts/release.mjs prepare <version>         set the version and date the changelog section
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
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { basename, join } from 'node:path';
import { fileURLToPath } from 'node:url';

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
      const cargo = join(repository, 'Cargo.toml');
      writeFileSync(cargo, withCargoVersion(readText(cargo), version));
      const desktop = join(repository, 'apps', 'desktop', 'package.json');
      writeFileSync(desktop, withPackageVersion(readText(desktop), version));
      const changelog = join(repository, 'CHANGELOG.md');
      writeFileSync(changelog, withReleasedChangelog(readText(changelog), version, date));
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
