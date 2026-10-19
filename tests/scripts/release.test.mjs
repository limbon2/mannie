// @ts-check
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  cargoVersion,
  checkedVersion,
  checksums,
  isValidSignature,
  keyFingerprint,
  newKeyPair,
  pluginChangelogs,
  releaseNotes,
  signature,
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
