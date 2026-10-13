// @ts-check
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, describe, it } from 'node:test';

import {
  END_MARK,
  generatePseudoLocaleProject,
  PSEUDO_LOCALE,
  pseudoLocalize,
  pseudoLocalizeCatalog,
  START_MARK,
} from '../../scripts/pseudo-locale.mjs';

describe('pseudoLocalize', () => {
  it('marks, accents and lengthens a message', () => {
    const result = pseudoLocalize('Settings');

    assert.ok(result.startsWith(START_MARK));
    assert.ok(result.endsWith(END_MARK));
    assert.ok(result.includes('Šéţţîñĝš'), result);
    // 8 letters grow by 40%: 4 padding characters.
    assert.equal(result, `${START_MARK}Šéţţîñĝš····${END_MARK}`);
  });

  it('keeps variables and escaped braces', () => {
    const result = pseudoLocalize('Hello {name}, use \\{braces\\} and {count}');

    assert.ok(result.includes('{name}'), result);
    assert.ok(result.includes('{count}'), result);
    assert.ok(result.includes('\\{'), result);
    assert.ok(result.includes('\\}'), result);
  });

  it('keeps digits and punctuation', () => {
    assert.equal(pseudoLocalize('Ctrl ,').replace(/[·⟦⟧]/g, ''), 'Çţŕļ ,');
  });
});

describe('pseudoLocalizeCatalog', () => {
  it('transforms every message and keeps the schema', () => {
    const catalog = {
      $schema: 'https://inlang.com/schema/inlang-message-format',
      plain: 'Open',
      variants: [{ declarations: ['input count'], match: { 'count=one': 'One item', 'count=other': '{count} items' } }],
    };

    const result = pseudoLocalizeCatalog(catalog);

    assert.equal(result.$schema, catalog.$schema);
    assert.match(String(result.plain), /^⟦Öþéñ·+⟧$/);
    const [variant] = /** @type {{ match: Record<string, string> }[]} */ (result.variants);
    assert.match(variant.match['count=one'], /^⟦Öñé îţéɱ·+⟧$/);
    assert.ok(variant.match['count=other'].includes('{count}'));
  });
});

describe('generatePseudoLocaleProject', () => {
  const app = mkdtempSync(join(tmpdir(), 'mannie-pseudo-'));
  after(() => rmSync(app, { force: true, recursive: true }));

  it('writes a project with the pseudo-locale added', () => {
    mkdirSync(join(app, 'project.inlang'));
    mkdirSync(join(app, 'messages'));
    writeFileSync(
      join(app, 'project.inlang', 'settings.json'),
      JSON.stringify({
        baseLocale: 'en',
        locales: ['en', 'de'],
        modules: ['./node_modules/@inlang/plugin-message-format/dist/index.js'],
      }),
    );
    writeFileSync(join(app, 'project.inlang', 'paraglide.config.js'), 'export default {};\n');
    writeFileSync(join(app, 'messages', 'en.json'), JSON.stringify({ greeting: 'Hello' }));
    writeFileSync(join(app, 'messages', 'de.json'), JSON.stringify({ greeting: 'Hallo' }));

    generatePseudoLocaleProject(app);

    const output = join(app, '.pseudo-locale');
    const settings = JSON.parse(readFileSync(join(output, 'project.inlang', 'settings.json'), 'utf8'));
    assert.deepEqual(settings.locales, ['en', 'de', PSEUDO_LOCALE]);
    assert.deepEqual(settings.modules, ['../node_modules/@inlang/plugin-message-format/dist/index.js']);
    assert.deepEqual(JSON.parse(readFileSync(join(output, 'messages', 'de.json'), 'utf8')), { greeting: 'Hallo' });
    const pseudo = JSON.parse(readFileSync(join(output, 'messages', `${PSEUDO_LOCALE}.json`), 'utf8'));
    assert.match(pseudo.greeting, /^⟦Ĥéļļö·+⟧$/);
  });
});
