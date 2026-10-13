// @ts-check
// Writes the third-party notices of the desktop app's npm packages for a
// release (ADR-0017, docs/engineering/releasing.md): every package of the
// desktop app with its license and the license and notice files it ships.
// cargo-about writes the notices of the Rust crates (about.toml, about.hbs).
//
// The release workflow pipes `pnpm licenses list --json --long --filter
// mannie-desktop` into this script and saves its output.

import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { text } from 'node:stream/consumers';
import { fileURLToPath } from 'node:url';

import { parsedReport } from './check-licenses.mjs';

/** File names that hold a license or a notice, such as LICENSE, LICENSE-MIT or NOTICE.md. */
const NOTICE_FILE = /^(?:licen[cs]e|copying|notice)(?:[.-][\w.-]*)?$/i;

/** Code that only has such a name, such as license-checker.js. */
const CODE_FILE = /\.(?:[cm]?[jt]s|json|map)$/i;

/**
 * @typedef {{ name: string, versions: string[], paths: string[], license: string, homepage?: string }} LongLicensedPackage
 * @typedef {Record<string, LongLicensedPackage[]>} LongLicenseReport
 */

/**
 * The license and notice files in a package folder, as name and text.
 * @param {string} folder
 * @returns {{ name: string, text: string }[]}
 */
export function noticeFiles(folder) {
  if (!existsSync(folder)) {
    return [];
  }
  return readdirSync(folder, { withFileTypes: true })
    .filter((entry) => {
      const isNotice = NOTICE_FILE.test(entry.name);
      const isCode = CODE_FILE.test(entry.name);
      return entry.isFile() && isNotice && !isCode;
    })
    .map((entry) => entry.name)
    .sort()
    .map((name) => ({ name, text: readFileSync(join(folder, name), 'utf8').trim() }));
}

/**
 * The notices document.
 * @param {LongLicenseReport} report
 * @param {(folder: string) => { name: string, text: string }[]} readNotices
 * @returns {string}
 */
export function notices(report, readNotices = noticeFiles) {
  const packages = Object.values(report)
    .flat()
    .sort((left, right) => left.name.localeCompare(right.name));
  const sections = packages.map((licensedPackage) => {
    const files = licensedPackage.paths.flatMap((path) => readNotices(path));
    const unique = [...new Map(files.map((file) => [file.text, file])).values()];
    const heading = `## ${licensedPackage.name} ${licensedPackage.versions.join(', ')}`;
    const facts = [`License: ${licensedPackage.license}`];
    if (licensedPackage.homepage !== undefined) {
      facts.push(`Homepage: ${licensedPackage.homepage}`);
    }
    const texts =
      unique.length === 0
        ? ['The package includes no license file; its license is the one named above.']
        : unique.map((file) => `${file.name}:\n\n\`\`\`text\n${file.text}\n\`\`\``);
    return [heading, facts.join('\n\n'), ...texts].join('\n\n');
  });
  const introduction =
    '# Third-party notices: npm packages\n\nThe Mannie desktop app is built from these npm packages. Each section names a package, its license and the license and notice files it includes.';
  return `${[introduction, ...sections].join('\n\n')}\n`;
}

const isMain = process.argv[1] === fileURLToPath(import.meta.url);
if (isMain) {
  // The long report has the same shape as the short one, with paths added.
  const report = /** @type {LongLicenseReport} */ (/** @type {unknown} */ (parsedReport(await text(process.stdin))));
  process.stdout.write(notices(report));
}
