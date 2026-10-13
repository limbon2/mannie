// @ts-check
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, describe, it } from 'node:test';

import { noticeFiles, notices } from '../../scripts/third-party-notices.mjs';

describe('noticeFiles', () => {
  const folder = mkdtempSync(join(tmpdir(), 'mannie-notices-'));
  after(() => {
    rmSync(folder, { force: true, recursive: true });
  });

  it('reads the license and notice files of a package', () => {
    writeFileSync(join(folder, 'LICENSE.md'), 'MIT License\n');
    writeFileSync(join(folder, 'NOTICE'), 'Copyright notice\n');
    writeFileSync(join(folder, 'README.md'), 'Not a license\n');
    writeFileSync(join(folder, 'license-checker.js'), 'not a license either');

    assert.deepEqual(noticeFiles(folder), [
      { name: 'LICENSE.md', text: 'MIT License' },
      { name: 'NOTICE', text: 'Copyright notice' },
    ]);
  });

  it('finds nothing in a missing folder', () => {
    assert.deepEqual(noticeFiles(join(folder, 'missing')), []);
  });
});

describe('notices', () => {
  it('lists the packages by name with their licenses and texts', () => {
    const texts = new Map([
      ['/b', [{ name: 'LICENSE', text: 'B license' }]],
      ['/a', []],
    ]);

    const document = notices(
      {
        ISC: [
          { homepage: 'https://example.org/b', license: 'ISC', name: 'b-package', paths: ['/b'], versions: ['2.0.0'] },
        ],
        MIT: [{ license: 'MIT', name: 'a-package', paths: ['/a'], versions: ['1.0.0', '1.1.0'] }],
      },
      (folder) => texts.get(folder) ?? [],
    );

    assert.ok(document.startsWith('# Third-party notices: npm packages\n'));
    const aSection = document.indexOf('## a-package 1.0.0, 1.1.0');
    const bSection = document.indexOf('## b-package 2.0.0');
    assert.ok(aSection > 0 && bSection > aSection, document);
    assert.match(document, /License: MIT\n\nThe package includes no license file/);
    assert.match(
      document,
      /License: ISC\n\nHomepage: https:\/\/example\.org\/b\n\nLICENSE:\n\n```text\nB license\n```/,
    );
  });
});
