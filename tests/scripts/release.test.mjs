// @ts-check
import assert from 'node:assert/strict';
import { createHash, createPrivateKey, createPublicKey, generateKeyPairSync, sign, verify } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import { describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  cargoVersion,
  checkedVersion,
  checksums,
  isValidSignature,
  keyFingerprint,
  manifestProblems,
  MAX_INSTALLER_BYTES,
  minisignPublicKey,
  minisignSignature,
  newKeyPair,
  pluginChangelogs,
  releaseNotes,
  signature,
  updateManifest,
  updaterKey,
  verifiedTrustedComment,
  withCargoVersion,
  withPackageVersion,
  withPluginChanges,
  withReleasedChangelog,
} from '../../scripts/release.mjs';

const CHANGELOG = `# Changelog

Intro.

## Unreleased

### Added

- The worker service.

## 0.1.0 - 2026-11-01

### Added

- Chat.
`;

describe('checkedVersion', () => {
  it('accepts SemVer versions', () => {
    assert.equal(checkedVersion('0.1.0'), '0.1.0');
    assert.equal(checkedVersion('1.20.3-rc.1'), '1.20.3-rc.1');
  });

  it('refuses anything else', () => {
    for (const version of [undefined, '', 'v0.1.0', '0.1', '01.2.3', '0.1.0+build']) {
      assert.throws(() => checkedVersion(version), /is not a version/, String(version));
    }
  });
});

describe('cargoVersion', () => {
  it('reads the workspace package version', () => {
    assert.equal(cargoVersion('[workspace.package]\nversion = "0.3.1"\nedition = "2024"\n'), '0.3.1');
  });
});

describe('withCargoVersion', () => {
  it('changes only the workspace package version', () => {
    const manifest =
      '[workspace]\nresolver = "3"\n\n[workspace.package]\nversion = "0.0.0"\nedition = "2024"\n\n[workspace.dependencies]\nclap = { version = "4.6.7" }\n';

    const changed = withCargoVersion(manifest, '0.2.0');

    assert.equal(changed, manifest.replace('version = "0.0.0"', 'version = "0.2.0"'));
  });

  it('refuses a manifest without a workspace version', () => {
    assert.throws(() => withCargoVersion('[workspace]\n', '0.2.0'), /no version/);
  });
});

describe('withPackageVersion', () => {
  it('changes the top-level version and keeps the layout', () => {
    const manifest =
      '{\n  "name": "mannie-desktop",\n  "version": "0.0.0",\n  "dependencies": {\n    "x": "1.0.0"\n  }\n}\n';

    assert.equal(withPackageVersion(manifest, '0.2.0'), manifest.replace('"0.0.0"', '"0.2.0"'));
  });
});

describe('withReleasedChangelog', () => {
  it('dates the Unreleased section and opens a new one', () => {
    const released = withReleasedChangelog(CHANGELOG, '0.2.0', '2026-12-01');

    assert.match(
      released,
      /## Unreleased\n\n## 0\.2\.0 - 2026-12-01\n\n### Added\n\n- The worker service\.\n\n## 0\.1\.0/,
    );
  });

  it('refuses a release without changes', () => {
    const empty = CHANGELOG.replace('### Added\n\n- The worker service.\n\n', '');

    assert.throws(() => withReleasedChangelog(empty, '0.2.0', '2026-12-01'), /lists no changes/);
  });

  it('refuses a version that already has a section', () => {
    assert.throws(() => withReleasedChangelog(CHANGELOG, '0.1.0', '2026-12-01'), /already has a section/);
  });
});

/**
 * A plugin's changelog with the given Unreleased section.
 * @param {string} unreleased
 */
function pluginChangelog(unreleased) {
  return `# Changelog of the plugin\n\nIntro.\n\n## Unreleased\n${unreleased}`;
}

describe('withPluginChanges', () => {
  it("moves each plugin's changes under their kinds and the plugin's name, and empties the plugins' sections", () => {
    const workers = pluginChangelog('\n### Fixed\n\n- A worker reconnects.\n\n### Added\n\n- Worker groups.\n');
    const images = pluginChangelog('\n### Added\n\n- Images from the clipboard.\n');

    const folded = withPluginChanges(CHANGELOG, [
      { name: 'Workers', path: 'plugins/workers/CHANGELOG.md', text: workers },
      { name: 'Images', path: 'plugins/images/CHANGELOG.md', text: images },
    ]);

    assert.equal(
      folded.changelog,
      [
        '# Changelog',
        '',
        'Intro.',
        '',
        '## Unreleased',
        '',
        '### Added',
        '',
        '- The worker service.',
        '',
        '#### Workers',
        '',
        '- Worker groups.',
        '',
        '#### Images',
        '',
        '- Images from the clipboard.',
        '',
        '### Fixed',
        '',
        '#### Workers',
        '',
        '- A worker reconnects.',
        '',
        '## 0.1.0 - 2026-11-01',
        '',
        '### Added',
        '',
        '- Chat.',
        '',
      ].join('\n'),
    );
    assert.deepEqual(folded.plugins, [pluginChangelog(''), pluginChangelog('')]);
    assert.match(
      withReleasedChangelog(folded.changelog, '0.2.0', '2026-12-01'),
      /## 0\.2\.0 - 2026-12-01\n\n### Added/u,
    );
  });

  it('changes nothing when no plugin has changes', () => {
    const plugins = [{ name: 'Workers', path: 'plugins/workers/CHANGELOG.md', text: pluginChangelog('\n') }];

    assert.deepEqual(withPluginChanges(CHANGELOG, plugins), { changelog: CHANGELOG, plugins: [plugins[0]?.text] });
  });

  it('refuses a plugin changelog without Unreleased, with an unknown kind or with a change outside a kind', () => {
    const plugin = (/** @type {string} */ text) => [{ name: 'Workers', path: 'plugins/workers/CHANGELOG.md', text }];

    assert.throws(
      () => withPluginChanges(CHANGELOG, plugin('# Changelog\n')),
      /plugins\/workers\/CHANGELOG\.md has no "## Unreleased"/u,
    );
    assert.throws(
      () => withPluginChanges(CHANGELOG, plugin(pluginChangelog('\n### Notes\n\n- A note.\n'))),
      /"Notes", which is not one of/u,
    );
    assert.throws(
      () => withPluginChanges(CHANGELOG, plugin(pluginChangelog('\n- A loose change.\n'))),
      /outside a kind of change/u,
    );
  });
});

describe('pluginChangelogs', () => {
  it("reads the changelogs of the plugins that ship, with their names from their catalogs, and not the fixture's", () => {
    const repository = fileURLToPath(new URL('../../', import.meta.url));

    const changelogs = pluginChangelogs(repository);

    assert.deepEqual(
      changelogs.map(({ name, path }) => ({ name, path })),
      [
        { name: 'Workers', path: 'plugins/workers/CHANGELOG.md' },
        { name: 'Images', path: 'plugins/images/CHANGELOG.md' },
        { name: 'SSH', path: 'plugins/ssh/CHANGELOG.md' },
        { name: 'Git', path: 'plugins/git/CHANGELOG.md' },
        { name: 'MCP', path: 'plugins/mcp/CHANGELOG.md' },
        { name: 'Notebooks', path: 'plugins/notebooks/CHANGELOG.md' },
        { name: 'Terminal', path: 'plugins/terminal/CHANGELOG.md' },
      ],
    );
    for (const changelog of changelogs) {
      assert.match(changelog.text, /^## Unreleased$/mu);
    }
  });
});

describe('releaseNotes', () => {
  it('returns the section of the version without its heading', () => {
    assert.equal(releaseNotes(CHANGELOG, '0.1.0'), '### Added\n\n- Chat.\n');
  });

  it('refuses a version without a section', () => {
    assert.throws(() => releaseNotes(CHANGELOG, '0.9.0'), /no section for 0\.9\.0/);
  });
});

describe('checksums', () => {
  it('writes sorted lines that sha256sum --check reads', () => {
    const text = checksums([
      { content: Buffer.from('b'), name: 'b.zip' },
      { content: Buffer.from(''), name: 'a.tar.gz' },
    ]);

    assert.equal(
      text,
      'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  a.tar.gz\n' +
        '3e23e8160039594a33894f6564e1b1348bbd7a0088d42c4acb73eeaed59c009d  b.zip\n',
    );
  });
});

describe('signatures', () => {
  it('verify with the matching public key only', () => {
    const pair = newKeyPair();
    const other = newKeyPair();
    const content = Buffer.from('mannie-worker');

    const signed = signature(content, pair.privateKey);

    assert.equal(isValidSignature(content, signed, pair.publicKey), true);
    assert.equal(isValidSignature(Buffer.from('mannie-worker!'), signed, pair.publicKey), false);
    assert.equal(isValidSignature(content, signed, other.publicKey), false);
  });
});

describe('keyFingerprint', () => {
  it('is the SHA-256 of the key info, the same for every copy of a key and different for another key', () => {
    const pair = newKeyPair();
    const other = newKeyPair();

    const fingerprint = keyFingerprint(pair.publicKey);

    assert.match(fingerprint, /^[0-9a-f]{64}$/u);
    assert.equal(keyFingerprint(`${pair.publicKey}\n`), fingerprint);
    assert.notEqual(keyFingerprint(other.publicKey), fingerprint);
  });
});

/**
 * A key pair that is the same on every run, for what a test compares with a
 * stored copy. The PKCS #8 form of an Ed25519 key is a fixed start and the
 * key's 32 bytes. Such a key signs test data only.
 * @param {number} fill the value of each of the key's bytes
 * @returns {{ privateKey: string, publicKey: string }}
 */
function fixedKeyPair(fill) {
  const start = Buffer.from('302e020100300506032b657004220420', 'hex');
  const privateKey = createPrivateKey({
    format: 'der',
    key: Buffer.concat([start, Buffer.alloc(32, fill)]),
    type: 'pkcs8',
  });
  return {
    privateKey: privateKey.export({ format: 'pem', type: 'pkcs8' }).toString(),
    publicKey: createPublicKey(privateKey).export({ format: 'pem', type: 'spki' }).toString(),
  };
}

/**
 * A minisign signature in the legacy format, which signs the content itself
 * where today's format signs its BLAKE2b-512 hash. The release script writes
 * none, so this one is built from the parts of the script's signature.
 * @param {Buffer} content
 * @param {string} privateKeyPem
 * @param {string} signatureText the script's signature of the same content
 * @returns {string}
 */
function legacySignature(content, privateKeyPem, signatureText) {
  const [untrustedComment = '', signatureLine = '', commentLine = ''] = signatureText.split('\n');
  const keyId = Buffer.from(signatureLine, 'base64').subarray(2, 10);
  const privateKey = createPrivateKey(privateKeyPem);
  const fileSignature = sign(null, content, privateKey);
  const trustedComment = commentLine.slice('trusted comment: '.length);
  const commentSignature = sign(null, Buffer.concat([fileSignature, Buffer.from(trustedComment)]), privateKey);
  return [
    untrustedComment,
    Buffer.concat([Buffer.from('Ed'), keyId, fileSignature]).toString('base64'),
    commentLine,
    commentSignature.toString('base64'),
    '',
  ].join('\n');
}

/**
 * @param {string} publicKeyPem an Ed25519 key
 * @returns {Buffer} the key's 32 bytes, which end its SubjectPublicKeyInfo
 */
function rawPublicKey(publicKeyPem) {
  return createPublicKey(publicKeyPem).export({ format: 'der', type: 'spki' }).subarray(-32);
}

/**
 * @param {string} publicKeyPem
 * @returns {Buffer} the ID that minisign keys and signatures carry for the key
 */
function keyId(publicKeyPem) {
  return Buffer.from(keyFingerprint(publicKeyPem), 'hex').subarray(0, 8);
}

describe('minisignPublicKey', () => {
  it('writes "Ed", the key ID and the key in base64 under a comment line', () => {
    const pair = fixedKeyPair(7);

    const lines = minisignPublicKey(pair.publicKey).split('\n');
    const key = Buffer.from(lines[1] ?? '', 'base64');

    assert.deepEqual([lines[0], lines[2]], ["untrusted comment: Mannie's release key", '']);
    assert.equal(lines.length, 3);
    assert.equal(key.subarray(0, 2).toString('latin1'), 'Ed');
    assert.deepEqual(key.subarray(2, 10), keyId(pair.publicKey));
    assert.deepEqual(key.subarray(10), rawPublicKey(pair.publicKey));
  });

  it('refuses a key that is not an Ed25519 key', () => {
    const other = generateKeyPairSync('ed448').publicKey.export({ format: 'pem', type: 'spki' }).toString();

    assert.throws(() => minisignPublicKey(other), /is not an Ed25519 key/u);
  });
});

/** The public key of the release key, which the release workflow signs with. */
const RELEASE_KEY = fileURLToPath(new URL('../../release/mannie-release.pub', import.meta.url));

/** The configuration of the desktop app, which holds the key that its updater verifies with. */
const DESKTOP_CONFIGURATION = fileURLToPath(new URL('../../apps/desktop/src-tauri/tauri.conf.json', import.meta.url));

/** The module of the desktop core that stops a download above its size limit. */
const DESKTOP_UPDATES = fileURLToPath(new URL('../../apps/desktop/core/src/updates.rs', import.meta.url));

describe('updaterKey', () => {
  it('is the base64 of the minisign text of the key', () => {
    const pair = fixedKeyPair(7);

    assert.equal(Buffer.from(updaterKey(pair.publicKey), 'base64').toString(), minisignPublicKey(pair.publicKey));
  });

  // The release workflow signs the installers with the release key, and an
  // installed app verifies them with the key of its configuration.
  it('of the release key is the key in the configuration of the desktop app', () => {
    const configuration = JSON.parse(readFileSync(DESKTOP_CONFIGURATION, 'utf8'));

    assert.equal(configuration.plugins.updater.pubkey, updaterKey(readFileSync(RELEASE_KEY, 'utf8')));
  });
});

/** What the tests sign in place of an installer. */
const INSTALLER = Buffer.from('What the tests sign in place of an installer.\n');

/** What the signatures of the tests say about it. */
const SIGNED = { file: 'mannie-1.2.3-linux-amd64.deb', timestamp: 1_794_000_000, version: '1.2.3' };

describe('minisignSignature', () => {
  it('is the same for the same content, key and comment', () => {
    const pair = fixedKeyPair(7);

    assert.equal(
      minisignSignature(INSTALLER, pair.privateKey, SIGNED),
      minisignSignature(INSTALLER, pair.privateKey, SIGNED),
    );
  });

  it('writes a comment line, the signature after "ED" and the key ID, the trusted comment and its signature', () => {
    const pair = fixedKeyPair(7);

    const lines = minisignSignature(INSTALLER, pair.privateKey, SIGNED).split('\n');
    const signed = Buffer.from(lines[1] ?? '', 'base64');

    assert.equal(lines.length, 5);
    assert.equal(lines[0], "untrusted comment: signature from Mannie's release key");
    assert.equal(signed.length, 74);
    assert.equal(signed.subarray(0, 2).toString('latin1'), 'ED');
    assert.deepEqual(signed.subarray(2, 10), keyId(pair.publicKey));
    assert.equal(lines[2], 'trusted comment: timestamp:1794000000\tfile:mannie-1.2.3-linux-amd64.deb\tversion:1.2.3');
    assert.equal(Buffer.from(lines[3] ?? '', 'base64').length, 64);
    assert.equal(lines[4], '');
  });

  it('signs the BLAKE2b-512 hash of the content, and the signature together with the trusted comment', () => {
    const pair = fixedKeyPair(7);
    const publicKey = createPublicKey(pair.publicKey);

    const lines = minisignSignature(INSTALLER, pair.privateKey, SIGNED).split('\n');
    const fileSignature = Buffer.from(lines[1] ?? '', 'base64').subarray(10);
    const trustedComment = (lines[2] ?? '').slice('trusted comment: '.length);
    const commentSignature = Buffer.from(lines[3] ?? '', 'base64');

    assert.ok(verify(null, createHash('blake2b512').update(INSTALLER).digest(), publicKey, fileSignature));
    assert.ok(verify(null, Buffer.concat([fileSignature, Buffer.from(trustedComment)]), publicKey, commentSignature));
  });

  it('differs for another file name, version or time of signing', () => {
    const pair = fixedKeyPair(7);
    const signed = minisignSignature(INSTALLER, pair.privateKey, SIGNED);

    for (const other of [
      { ...SIGNED, file: 'mannie-1.2.3-linux-x86_64.rpm' },
      { ...SIGNED, timestamp: SIGNED.timestamp + 1 },
      { ...SIGNED, version: '1.2.4' },
    ]) {
      assert.notEqual(minisignSignature(INSTALLER, pair.privateKey, other), signed);
    }
  });

  it('refuses a file name or a version with a tab or a line break', () => {
    const pair = fixedKeyPair(7);

    for (const signed of [
      { ...SIGNED, file: 'mannie\t1.2.3.deb' },
      { ...SIGNED, file: 'mannie\n1.2.3.deb' },
      { ...SIGNED, version: '1.2.3\r' },
      { ...SIGNED, version: '1.2.3\tfile:another.deb' },
    ]) {
      assert.throws(() => minisignSignature(INSTALLER, pair.privateKey, signed), /has a tab or a line break/u);
    }
  });
});

describe('verifiedTrustedComment', () => {
  it('reads the fields of the trusted comment of a valid signature', () => {
    const pair = fixedKeyPair(7);
    const signed = minisignSignature(INSTALLER, pair.privateKey, SIGNED);

    assert.deepEqual(
      verifiedTrustedComment(INSTALLER, signed, pair.publicKey),
      new Map([
        ['timestamp', '1794000000'],
        ['file', 'mannie-1.2.3-linux-amd64.deb'],
        ['version', '1.2.3'],
      ]),
    );
  });

  it('reads a signature whose lines end as on Windows', () => {
    const pair = fixedKeyPair(7);
    const signed = minisignSignature(INSTALLER, pair.privateKey, SIGNED).replaceAll('\n', '\r\n');

    assert.equal(verifiedTrustedComment(INSTALLER, signed, pair.publicKey)?.get('version'), '1.2.3');
  });

  it('knows no comment of changed content', () => {
    const pair = fixedKeyPair(7);
    const signed = minisignSignature(INSTALLER, pair.privateKey, SIGNED);
    const changed = Buffer.concat([INSTALLER, Buffer.from('!')]);

    assert.equal(verifiedTrustedComment(changed, signed, pair.publicKey), null);
  });

  it('knows no comment of a signature by another key', () => {
    const pair = fixedKeyPair(7);
    const signed = minisignSignature(INSTALLER, fixedKeyPair(9).privateKey, SIGNED);

    assert.equal(verifiedTrustedComment(INSTALLER, signed, pair.publicKey), null);
  });

  it("knows no comment of another key's signature that carries the ID of the key", () => {
    const pair = fixedKeyPair(7);
    const lines = minisignSignature(INSTALLER, fixedKeyPair(9).privateKey, SIGNED).split('\n');
    const signed = Buffer.from(lines[1] ?? '', 'base64');
    keyId(pair.publicKey).copy(signed, 2);
    lines[1] = signed.toString('base64');

    assert.equal(verifiedTrustedComment(INSTALLER, lines.join('\n'), pair.publicKey), null);
  });

  it('knows no comment that changed after signing', () => {
    const pair = fixedKeyPair(7);
    const signed = minisignSignature(INSTALLER, pair.privateKey, SIGNED).replace('version:1.2.3', 'version:1.2.4');

    assert.equal(verifiedTrustedComment(INSTALLER, signed, pair.publicKey), null);
  });

  it('knows no comment of a signature in the legacy format', () => {
    const pair = fixedKeyPair(7);
    const signed = minisignSignature(INSTALLER, pair.privateKey, SIGNED);
    const legacy = legacySignature(INSTALLER, pair.privateKey, signed);

    assert.equal(verifiedTrustedComment(INSTALLER, legacy, pair.publicKey), null);
  });

  it('knows no comment of text that is no signature', () => {
    const pair = fixedKeyPair(7);
    const withoutComment = minisignSignature(INSTALLER, pair.privateKey, SIGNED).split('\n').slice(0, 2).join('\n');

    for (const text of ['', 'not a signature', withoutComment]) {
      assert.equal(verifiedTrustedComment(INSTALLER, text, pair.publicKey), null);
    }
  });
});

/** Where the files of the tests' release download from. */
const DOWNLOADS = 'https://example.com/mannie/releases/download/v1.2.3';

/** When the tests' release is published, in the whole seconds that its signatures and its manifest name. */
const PUBLISHED_SECONDS = Date.UTC(2026, 10, 10, 8, 30, 15) / 1000;

/** The desktop installers of the tests' release with their platforms, in the manifest's order. */
const DESKTOP_INSTALLERS = [
  { name: 'mannie-1.2.3-windows-x86_64-setup.exe', platform: 'windows-x86_64-nsis' },
  { name: 'mannie-1.2.3-linux-amd64.deb', platform: 'linux-x86_64-deb' },
  { name: 'mannie-1.2.3-linux-x86_64.rpm', platform: 'linux-x86_64-rpm' },
];

/**
 * The files of a release of version 1.2.3: its desktop installers, and two
 * files that its manifest leaves out.
 * @returns {{ name: string, content: Buffer }[]}
 */
function releaseFiles() {
  return [
    ...DESKTOP_INSTALLERS.map(({ name }) => ({ content: Buffer.from(`the content of ${name}`), name })),
    { content: Buffer.from('a worker'), name: 'mannie-worker-1.2.3-linux-x86_64.tar.gz' },
    { content: Buffer.from('a package of the release before'), name: 'mannie-1.2.2-linux-amd64.deb' },
  ];
}

/**
 * @param {string} name
 * @returns {Buffer} the content of one of the release files
 */
function contentOf(name) {
  const file = releaseFiles().find((candidate) => candidate.name === name);
  assert.ok(file, `the release files have no ${name}`);
  return file.content;
}

/**
 * @param {string} privateKeyPem
 * @returns {import('../../scripts/release.mjs').ManifestRelease} the tests' release
 */
function release(privateKeyPem) {
  return {
    downloadUrl: DOWNLOADS,
    notes: '### Added\n\n- Updates.',
    privateKeyPem,
    publishedAt: new Date('2026-11-10T08:30:15.750Z'),
    version: '1.2.3',
  };
}

/**
 * @param {Buffer} content
 * @param {string} privateKeyPem
 * @param {{ file: string, version: string }} signed
 * @returns {string} a signature as a manifest holds it
 */
function manifestSignature(content, privateKeyPem, signed) {
  const text = minisignSignature(content, privateKeyPem, { ...signed, timestamp: PUBLISHED_SECONDS });
  return Buffer.from(text).toString('base64');
}

describe('updateManifest', () => {
  it('names the version, the notes and the date in whole seconds', () => {
    const pair = fixedKeyPair(7);

    const text = updateManifest(releaseFiles(), release(pair.privateKey));
    const manifest = JSON.parse(text);

    assert.deepEqual(Object.keys(manifest), ['notes', 'platforms', 'pub_date', 'version']);
    assert.equal(manifest.notes, '### Added\n\n- Updates.');
    assert.equal(manifest.pub_date, '2026-11-10T08:30:15Z');
    assert.equal(manifest.version, '1.2.3');
    assert.ok(text.endsWith('\n}\n'));
  });

  it('names each desktop installer under its platform, with its address and its signature', () => {
    const pair = fixedKeyPair(7);

    const manifest = JSON.parse(updateManifest(releaseFiles(), release(pair.privateKey)));

    assert.deepEqual(
      Object.keys(manifest.platforms),
      DESKTOP_INSTALLERS.map(({ platform }) => platform),
    );
    for (const { name, platform } of DESKTOP_INSTALLERS) {
      const entry = manifest.platforms[platform];
      const signed = Buffer.from(entry.signature, 'base64').toString();
      assert.deepEqual(Object.keys(entry), ['signature', 'url']);
      assert.equal(entry.url, `${DOWNLOADS}/${name}`);
      assert.deepEqual(
        verifiedTrustedComment(contentOf(name), signed, pair.publicKey),
        new Map([
          ['timestamp', String(PUBLISHED_SECONDS)],
          ['file', name],
          ['version', '1.2.3'],
        ]),
      );
    }
  });

  it('is the same for the same release', () => {
    const pair = fixedKeyPair(7);

    assert.equal(
      updateManifest(releaseFiles(), release(pair.privateKey)),
      updateManifest(releaseFiles(), release(pair.privateKey)),
    );
  });

  it('leaves out a platform whose installer the release lacks', () => {
    const pair = fixedKeyPair(7);
    const files = releaseFiles().filter((file) => !file.name.endsWith('.exe'));

    const manifest = JSON.parse(updateManifest(files, release(pair.privateKey)));

    assert.deepEqual(Object.keys(manifest.platforms), ['linux-x86_64-deb', 'linux-x86_64-rpm']);
  });

  it('refuses a release without a desktop installer of its version', () => {
    const pair = fixedKeyPair(7);
    const files = releaseFiles().filter((file) => !file.name.startsWith('mannie-1.2.3-'));

    assert.throws(() => updateManifest(files, release(pair.privateKey)), /no desktop installer of 1\.2\.3/u);
  });

  it('refuses a download address that is not an HTTPS address', () => {
    const pair = fixedKeyPair(7);
    const overHttp = { ...release(pair.privateKey), downloadUrl: 'http://example.com/mannie/releases/download/v1.2.3' };

    assert.throws(() => updateManifest(releaseFiles(), overHttp), /is not an HTTPS address/u);
  });

  it('joins a download address that ends in a slash and a file name with one slash', () => {
    const pair = fixedKeyPair(7);
    const withSlash = { ...release(pair.privateKey), downloadUrl: `${DOWNLOADS}/` };

    const manifest = JSON.parse(updateManifest(releaseFiles(), withSlash));

    assert.equal(manifest.platforms['linux-x86_64-deb'].url, `${DOWNLOADS}/mannie-1.2.3-linux-amd64.deb`);
  });

  it('refuses an installer above the size that the desktop app downloads', () => {
    const pair = fixedKeyPair(7);
    const files = [{ content: Buffer.alloc(MAX_INSTALLER_BYTES + 1), name: 'mannie-1.2.3-linux-amd64.deb' }];

    assert.throws(
      () => updateManifest(files, release(pair.privateKey)),
      /mannie-1\.2\.3-linux-amd64\.deb is larger than the 256 MiB that the desktop app downloads/u,
    );
  });

  // The script refuses what the app would stop, so that a release never offers
  // an update that no installation can download. The limit is a constant in
  // each language, and this test keeps the two equal.
  it('has the size limit of the desktop app', () => {
    const source = readFileSync(DESKTOP_UPDATES, 'utf8');
    const limit = /const MAX_PACKAGE_BYTES: u64 = (?<factors>\d+(?: \* \d+)*);/u.exec(source);

    assert.ok(limit?.groups, 'updates.rs declares MAX_PACKAGE_BYTES as a product of integers');
    const factors = limit.groups.factors.split(' * ').map(Number);
    const bytes = factors.reduce((product, factor) => product * factor, 1);
    assert.equal(MAX_INSTALLER_BYTES, bytes);
  });
});

/**
 * @param {string} privateKeyPem
 * @returns {any} the manifest of the release files as an object, which a test changes before it checks it
 */
function parsedManifest(privateKeyPem) {
  return JSON.parse(updateManifest(releaseFiles(), release(privateKeyPem)));
}

describe('manifestProblems', () => {
  it('finds nothing wrong with the manifest that the script wrote for the files', () => {
    const pair = fixedKeyPair(7);

    const manifest = updateManifest(releaseFiles(), release(pair.privateKey));

    assert.deepEqual(manifestProblems(manifest, pair.publicKey, releaseFiles()), []);
  });

  it('says when the manifest is not JSON', () => {
    const pair = fixedKeyPair(7);

    assert.deepEqual(manifestProblems('not JSON', pair.publicKey, releaseFiles()), ['latest.json is not JSON']);
  });

  it('says when the manifest names no version', () => {
    const pair = fixedKeyPair(7);

    for (const manifest of ['{}', '[]', 'null', '"1.2.3"', '{"version":"v1.2.3"}', '{"version":7}']) {
      assert.deepEqual(manifestProblems(manifest, pair.publicKey, releaseFiles()), [
        'latest.json names no version such as 0.1.0',
      ]);
    }
  });

  it('says when the manifest names no installer', () => {
    const pair = fixedKeyPair(7);

    for (const platforms of [undefined, {}, [], 'linux-x86_64-deb']) {
      const manifest = JSON.stringify({ platforms, version: '1.2.3' });
      assert.deepEqual(manifestProblems(manifest, pair.publicKey, releaseFiles()), ['latest.json names no installer']);
    }
  });

  it('says when a platform is not one of the desktop app', () => {
    const pair = fixedKeyPair(7);
    const manifest = parsedManifest(pair.privateKey);
    manifest.platforms['darwin-aarch64-app'] = { signature: '', url: `${DOWNLOADS}/mannie-1.2.3-macos.app.tar.gz` };

    assert.deepEqual(manifestProblems(JSON.stringify(manifest), pair.publicKey, releaseFiles()), [
      'darwin-aarch64-app is not a platform of the desktop app',
    ]);
  });

  it('says when an address is not an HTTPS address of the installer', () => {
    const pair = fixedKeyPair(7);
    const name = 'mannie-1.2.3-linux-amd64.deb';

    for (const url of [
      `http://example.com/${name}`,
      `${DOWNLOADS}/mannie-1.2.2-linux-amd64.deb`,
      `${DOWNLOADS}/${name}?of=another`,
      undefined,
    ]) {
      const manifest = parsedManifest(pair.privateKey);
      manifest.platforms['linux-x86_64-deb'].url = url;
      assert.deepEqual(manifestProblems(JSON.stringify(manifest), pair.publicKey, releaseFiles()), [
        `linux-x86_64-deb: the address is not an HTTPS address of ${name}`,
      ]);
    }
  });

  it('says when the release files lack an installer that the manifest names', () => {
    const pair = fixedKeyPair(7);
    const manifest = updateManifest(releaseFiles(), release(pair.privateKey));
    const files = releaseFiles().filter((file) => !file.name.endsWith('.rpm'));

    assert.deepEqual(manifestProblems(manifest, pair.publicKey, files), [
      'linux-x86_64-rpm: the release files have no mannie-1.2.3-linux-x86_64.rpm',
    ]);
  });

  it('says when a signature is not a valid one by the key', () => {
    const pair = fixedKeyPair(7);
    const name = 'mannie-1.2.3-linux-amd64.deb';
    const byAnotherKey = manifestSignature(contentOf(name), fixedKeyPair(9).privateKey, {
      file: name,
      version: '1.2.3',
    });

    for (const signatureText of [byAnotherKey, 'not a signature', '', undefined]) {
      const manifest = parsedManifest(pair.privateKey);
      manifest.platforms['linux-x86_64-deb'].signature = signatureText;
      assert.deepEqual(manifestProblems(JSON.stringify(manifest), pair.publicKey, releaseFiles()), [
        `linux-x86_64-deb: the signature of ${name} is not valid`,
      ]);
    }
  });

  it('says when an installer changed after it was signed', () => {
    const pair = fixedKeyPair(7);
    const manifest = updateManifest(releaseFiles(), release(pair.privateKey));
    const files = releaseFiles().map((file) =>
      file.name.endsWith('.exe') ? { ...file, content: Buffer.from('another installer') } : file,
    );

    assert.deepEqual(manifestProblems(manifest, pair.publicKey, files), [
      'windows-x86_64-nsis: the signature of mannie-1.2.3-windows-x86_64-setup.exe is not valid',
    ]);
  });

  it('says when a signature names another file or another version', () => {
    const pair = fixedKeyPair(7);
    const name = 'mannie-1.2.3-linux-amd64.deb';

    for (const signed of [
      { file: 'mannie-1.2.3-linux-x86_64.rpm', version: '1.2.3' },
      { file: name, version: '1.2.2' },
    ]) {
      const manifest = parsedManifest(pair.privateKey);
      manifest.platforms['linux-x86_64-deb'].signature = manifestSignature(contentOf(name), pair.privateKey, signed);
      assert.deepEqual(manifestProblems(JSON.stringify(manifest), pair.publicKey, releaseFiles()), [
        `linux-x86_64-deb: the signature is not one of ${name} in version 1.2.3`,
      ]);
    }
  });

  // A signature stays valid for its installer under any file name, so the
  // version that it names is what tells an older release from a newer one.
  it('says when the installers of an older release are announced as a newer version', () => {
    const pair = fixedKeyPair(7);
    const manifest = parsedManifest(pair.privateKey);
    manifest.version = '1.2.4';
    for (const entry of Object.values(manifest.platforms)) {
      entry.url = entry.url.replace('mannie-1.2.3-', 'mannie-1.2.4-');
    }
    const files = releaseFiles().map((file) => ({
      ...file,
      name: file.name.replace('mannie-1.2.3-', 'mannie-1.2.4-'),
    }));

    assert.deepEqual(manifestProblems(JSON.stringify(manifest), pair.publicKey, files), [
      'windows-x86_64-nsis: the signature is not one of mannie-1.2.4-windows-x86_64-setup.exe in version 1.2.4',
      'linux-x86_64-deb: the signature is not one of mannie-1.2.4-linux-amd64.deb in version 1.2.4',
      'linux-x86_64-rpm: the signature is not one of mannie-1.2.4-linux-x86_64.rpm in version 1.2.4',
    ]);
  });

  it('says when an installer is above the size that the desktop app downloads', () => {
    const pair = fixedKeyPair(7);
    const name = 'mannie-1.2.3-linux-amd64.deb';
    const content = Buffer.alloc(MAX_INSTALLER_BYTES + 1);
    const signature = manifestSignature(content, pair.privateKey, { file: name, version: '1.2.3' });
    const manifest = JSON.stringify({
      platforms: { 'linux-x86_64-deb': { signature, url: `${DOWNLOADS}/${name}` } },
      version: '1.2.3',
    });

    assert.deepEqual(manifestProblems(manifest, pair.publicKey, [{ content, name }]), [
      `linux-x86_64-deb: ${name} is larger than the 256 MiB that the desktop app downloads as an update`,
    ]);
  });

  it('names every problem of a manifest', () => {
    const pair = fixedKeyPair(7);
    const manifest = parsedManifest(pair.privateKey);
    manifest.platforms['linux-x86_64-rpm'].url = 'http://example.com/mannie-1.2.3-linux-x86_64.rpm';
    manifest.platforms['darwin-aarch64-app'] = manifest.platforms['linux-x86_64-deb'];
    const files = releaseFiles().filter((file) => !file.name.endsWith('.rpm'));

    assert.deepEqual(manifestProblems(JSON.stringify(manifest), pair.publicKey, files), [
      'linux-x86_64-rpm: the address is not an HTTPS address of mannie-1.2.3-linux-x86_64.rpm',
      'linux-x86_64-rpm: the release files have no mannie-1.2.3-linux-x86_64.rpm',
      'darwin-aarch64-app is not a platform of the desktop app',
    ]);
  });
});

/**
 * The signed release that the tests of the desktop core read
 * (apps/desktop/core/src/updates/release/tests.rs): an installer, its
 * signature as the update manifest holds it, the same in the legacy format,
 * and the key as tauri.conf.json holds it.
 */
const SIGNED_RELEASE = fileURLToPath(new URL('../fixtures/updates/release.json', import.meta.url));

/** @returns {string} the text of the signed release, the same on every run */
function signedRelease() {
  const pair = fixedKeyPair(7);
  const version = '1.2.3';
  const file = `mannie-${version}-windows-x86_64-setup.exe`;
  const installer = 'A stand-in for an installer of Mannie.\n';
  const content = Buffer.from(installer);
  const signed = minisignSignature(content, pair.privateKey, { file, timestamp: 1_794_000_000, version });
  const release = {
    file,
    installer,
    legacySignature: Buffer.from(legacySignature(content, pair.privateKey, signed)).toString('base64'),
    publicKey: updaterKey(pair.publicKey),
    signature: Buffer.from(signed).toString('base64'),
    version,
  };
  return `${JSON.stringify(release, null, 2)}\n`;
}

describe('the signed release that the tests of the desktop core read', () => {
  // The desktop core verifies this file with the crate that the updater
  // verifies with, so a change to how the script signs shows there too.
  // MANNIE_UPDATE_EXPECTED=1 writes the file instead of comparing; review the
  // result before committing it.
  it('is what the release script signs', () => {
    const expected = signedRelease();
    if (process.env.MANNIE_UPDATE_EXPECTED !== undefined) {
      mkdirSync(dirname(SIGNED_RELEASE), { recursive: true });
      writeFileSync(SIGNED_RELEASE, expected);
      return;
    }

    assert.equal(
      readFileSync(SIGNED_RELEASE, 'utf8'),
      expected,
      `the release script signs a release differently; review it and update ${SIGNED_RELEASE}`,
    );
  });
});
