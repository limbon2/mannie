// @ts-check
// Consistency checks for the documentation that generic linters cannot make.
// `task docs:check` runs this script after Prettier, markdownlint, cspell and
// link validation. The rules come from docs/README.md (conventions),
// docs/adr/README.md (the ADR index) and docs/features/README.md (feature
// documents). Errors fail the check; warnings are printed only.

import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, posix, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

const FEATURE_STATUSES = ['Idea', 'Shaped', 'Accepted', 'Building', 'Shipped', 'Dropped'];
const SCHEDULED_FEATURE_STATUSES = ['Accepted', 'Building'];
const ADR_STATUS_PREFIXES = ['Accepted', 'Proposed', 'Superseded by ADR-', 'Deprecated'];
const NUMBERED_HEADING_FOLDERS = ['docs/product/', 'docs/architecture/', 'docs/engineering/', 'docs/roadmap/'];
const VERIFIED_DATE_DOCUMENT = 'docs/architecture/technology-stack.md';
const VERIFICATION_WARNING_DAYS = 90;
const MILESTONES_DOCUMENT = 'docs/roadmap/milestones.md';
const ADR_INDEX_DOCUMENT = 'docs/adr/README.md';
const GLOSSARY_DOCUMENT = 'docs/glossary.md';
const EM_DASH = '—';

/** @type {string[]} */
const errors = [];
/** @type {string[]} */
const warnings = [];

/**
 * @param {string} file
 * @param {number} line
 * @param {string} message
 */
function reportError(file, line, message) {
  errors.push(`${file}:${line}: ${message}`);
}

/**
 * @param {string} file
 * @param {number} line
 * @param {string} message
 */
function reportWarning(file, line, message) {
  warnings.push(`${file}:${line}: ${message}`);
}

/**
 * @typedef {object} Doc
 * @property {string} path Repository-relative path with forward slashes.
 * @property {string[]} lines
 * @property {boolean[]} inCode True for lines inside fenced code blocks, fences included.
 */

/**
 * @param {string} relativePath
 * @returns {Doc}
 */
function readDoc(relativePath) {
  const text = readFileSync(join(root, relativePath), 'utf8');
  const lines = text.split(/\r?\n/);
  const inCode = [];
  let fenced = false;
  for (const line of lines) {
    const isFence = /^\s*(```|~~~)/.test(line);
    if (isFence) {
      inCode.push(true);
      fenced = !fenced;
    } else {
      inCode.push(fenced);
    }
  }
  return { path: relativePath, lines, inCode };
}

/**
 * Lists Markdown files below a repository-relative folder.
 * @param {string} folder
 * @returns {string[]}
 */
function markdownFilesBelow(folder) {
  /** @type {string[]} */
  const found = [];
  const absolute = join(root, folder);
  if (!existsSync(absolute)) {
    return found;
  }
  for (const entry of readdirSync(absolute)) {
    const entryPath = join(absolute, entry);
    const relativePath = relative(root, entryPath).split(sep).join('/');
    if (statSync(entryPath).isDirectory()) {
      found.push(...markdownFilesBelow(relativePath));
    } else if (entry.endsWith('.md')) {
      found.push(relativePath);
    }
  }
  return found.sort();
}

const rootDocuments = readdirSync(root)
  .filter((entry) => entry.endsWith('.md'))
  .sort();
const docsDocuments = markdownFilesBelow('docs');
const allDocs = [...rootDocuments, ...docsDocuments].map(readDoc);
const docsByPath = new Map(allDocs.map((doc) => [doc.path, doc]));

/**
 * @param {string} path
 * @returns {Doc}
 */
function requireDoc(path) {
  const doc = docsByPath.get(path);
  if (!doc) {
    throw new Error(`Required document is missing: ${path}`);
  }
  return doc;
}

/**
 * Splits a Markdown table row into trimmed cells.
 * @param {string} line
 * @returns {string[]}
 */
function tableCells(line) {
  return line
    .trim()
    .replace(/^\|/, '')
    .replace(/\|$/, '')
    .split('|')
    .map((cell) => cell.trim());
}

/**
 * True when the line exists and holds only whitespace.
 * @param {string | undefined} line
 * @returns {boolean}
 */
function isBlank(line) {
  if (line === undefined) {
    return false;
  }
  return line.trim() === '';
}

/**
 * True when the line exists and is a row of a Markdown table.
 * @param {string | undefined} line
 * @returns {boolean}
 */
function isTableRow(line) {
  if (line === undefined) {
    return false;
  }
  return line.trim().startsWith('|');
}

/**
 * Reads the status table that follows the document title.
 * @param {Doc} doc
 * @returns {Map<string, { value: string, line: number }> | undefined}
 */
function statusTable(doc) {
  let index = doc.lines.findIndex((line) => line.trim() !== '');
  const hasText = index !== -1;
  const hasTitle = hasText && doc.lines[index].startsWith('# ');
  if (!hasTitle) {
    reportError(doc.path, index + 1, 'the document must start with a "# " title');
    return undefined;
  }
  index += 1;
  while (isBlank(doc.lines[index])) {
    index += 1;
  }
  /** @type {Map<string, { value: string, line: number }>} */
  const rows = new Map();
  let rowNumber = 0;
  while (isTableRow(doc.lines[index])) {
    // The first row is an empty header and the second one its separator.
    if (rowNumber >= 2) {
      const [key, ...rest] = tableCells(doc.lines[index]);
      rows.set(key, { value: rest.join(' | '), line: index + 1 });
    }
    rowNumber += 1;
    index += 1;
  }
  if (rows.size === 0) {
    reportError(doc.path, index + 1, 'a status table must follow the title');
    return undefined;
  }
  return rows;
}

/**
 * @param {Doc} doc
 * @param {Map<string, { value: string, line: number }>} table
 * @param {string[]} keys
 */
function requireKeys(doc, table, keys) {
  for (const key of keys) {
    if (!table.has(key)) {
      reportError(doc.path, 1, `the status table needs a "${key}" row`);
    }
  }
}

const today = new Date();
const todayIso = [
  today.getFullYear(),
  String(today.getMonth() + 1).padStart(2, '0'),
  String(today.getDate()).padStart(2, '0'),
].join('-');

/**
 * @param {Doc} doc
 * @param {{ value: string, line: number } | undefined} cell
 */
function checkDate(doc, cell) {
  if (!cell) {
    return;
  }
  if (!/^\d{4}-\d{2}-\d{2}$/.test(cell.value)) {
    reportError(doc.path, cell.line, `"${cell.value}" is not a YYYY-MM-DD date`);
  } else if (cell.value > todayIso) {
    reportError(doc.path, cell.line, `${cell.value} is in the future`);
  }
}

/**
 * True when the line at `index` is a "## " heading outside a code block.
 * @param {Doc} doc
 * @param {number} index
 * @returns {boolean}
 */
function isSecondLevelHeading(doc, index) {
  const isHeading = doc.lines[index].startsWith('## ');
  return isHeading && !doc.inCode[index];
}

/**
 * Returns the text of a "## <title>" section, without its heading.
 * @param {Doc} doc
 * @param {string} title
 * @returns {{ text: string, line: number } | undefined}
 */
function section(doc, title) {
  const headings = doc.lines.flatMap((_line, index) => (isSecondLevelHeading(doc, index) ? [index] : []));
  const start = headings.find((index) => doc.lines[index].replace(/^##\s+(\d+\.\s+)?/, '') === title);
  if (start === undefined) {
    return undefined;
  }
  const end = headings.find((index) => index > start) ?? doc.lines.length;
  return { text: doc.lines.slice(start + 1, end).join('\n'), line: start + 1 };
}

/**
 * Resolves the Markdown link targets of a line against the document's folder.
 * @param {Doc} doc
 * @param {string} line
 * @returns {string[]}
 */
function linkTargets(doc, line) {
  /** @type {string[]} */
  const targets = [];
  for (const match of line.matchAll(/\]\(([^)\s#]+)(#[^)]*)?\)/g)) {
    const target = match[1];
    if (/^[a-z]+:/i.test(target)) {
      continue;
    }
    targets.push(posix.normalize(posix.join(posix.dirname(doc.path), target)));
  }
  return targets;
}

// Status tables ---------------------------------------------------------------

const adrFilePattern = /^docs\/adr\/(\d{4})-[a-z0-9-]+\.md$/;
const featureFolderReadmePattern = /^docs\/features\/[a-z0-9-]+\/README\.md$/;
const featurePartPattern = /^docs\/features\/[a-z0-9-]+\/[a-z0-9-]+\.md$/;
const singleFeaturePattern = /^docs\/features\/[a-z0-9-]+\.md$/;
const templates = new Set(['docs/adr/0000-template.md', 'docs/features/template.md']);

/** @type {Map<string, { doc: Doc, table: Map<string, { value: string, line: number }> }>} */
const adrs = new Map();
/** @type {Map<string, { doc: Doc, table: Map<string, { value: string, line: number }>, kind: 'single' | 'folder' | 'part' }>} */
const features = new Map();

for (const doc of allDocs.filter((candidate) => candidate.path.startsWith('docs/'))) {
  const table = statusTable(doc);
  if (!table || templates.has(doc.path)) {
    continue;
  }
  const adrNumber = adrFilePattern.exec(doc.path)?.[1];
  const isTemplateNumber = adrNumber === '0000';
  const isAdr = adrNumber !== undefined && !isTemplateNumber;
  if (isAdr) {
    requireKeys(doc, table, ['Status', 'Date', 'Deciders', 'Related']);
    checkDate(doc, table.get('Date'));
    const title = doc.lines.find((line) => line.startsWith('# ')) ?? '';
    if (!title.startsWith(`# ADR-${adrNumber}: `)) {
      reportError(doc.path, 1, `the title must start with "# ADR-${adrNumber}: "`);
    }
    const status = table.get('Status')?.value ?? '';
    if (!ADR_STATUS_PREFIXES.some((prefix) => status.startsWith(prefix))) {
      reportError(doc.path, table.get('Status')?.line ?? 1, `unknown ADR status "${status}"`);
    }
    adrs.set(adrNumber, { doc, table });
    continue;
  }
  const isPart = featurePartPattern.test(doc.path) && !featureFolderReadmePattern.test(doc.path);
  const isFolderFeature = featureFolderReadmePattern.test(doc.path);
  const isSingleFeature = singleFeaturePattern.test(doc.path) && doc.path !== 'docs/features/README.md';
  if (isPart || isFolderFeature || isSingleFeature) {
    const keys = ['Status', 'Milestone', 'Last updated', 'Related'];
    requireKeys(doc, table, isPart ? [...keys, 'Part of'] : keys);
    checkDate(doc, table.get('Last updated'));
    const status = table.get('Status');
    if (status && !FEATURE_STATUSES.includes(status.value)) {
      reportError(doc.path, status.line, `feature status must be one of ${FEATURE_STATUSES.join(', ')}`);
    }
    /** @type {'single' | 'folder' | 'part'} */
    let kind = 'single';
    if (isPart) {
      kind = 'part';
    } else if (isFolderFeature) {
      kind = 'folder';
    }
    features.set(doc.path, { doc, table, kind });
    continue;
  }
  requireKeys(doc, table, ['Status', 'Last updated', 'Related']);
  checkDate(doc, table.get('Last updated'));
}

// Identifiers defined by the roadmap -----------------------------------------------

const milestonesDoc = requireDoc(MILESTONES_DOCUMENT);
/** @type {Map<string, Set<string>>} */
const defined = new Map([
  ['M', new Set()],
  ['V', new Set()],
  ['D', new Set()],
  ['R', new Set()],
]);
/** @type {Map<string, string>} Text of each validation row, by identifier. */
const validationRows = new Map();

milestonesDoc.lines.forEach((line, index) => {
  if (milestonesDoc.inCode[index]) {
    return;
  }
  const milestone = /^###\s+(M\d+):/.exec(line);
  if (milestone) {
    defined.get('M')?.add(milestone[1]);
  }
  const row = /^\|\s*([VDR])(\d+)\s*\|/.exec(line);
  if (row) {
    const id = `${row[1]}${row[2]}`;
    const kindSet = defined.get(row[1]);
    if (kindSet?.has(id)) {
      reportError(milestonesDoc.path, index + 1, `${id} is defined twice`);
    }
    kindSet?.add(id);
    if (row[1] === 'V') {
      validationRows.set(id, line);
    }
  }
});

// References ---------------------------------------------------------------------

for (const doc of allDocs) {
  doc.lines.forEach((line, index) => {
    for (const match of line.matchAll(/\bADR-(\d{4})\b/g)) {
      if (!adrs.has(match[1])) {
        reportError(doc.path, index + 1, `ADR-${match[1]} does not exist`);
      }
    }
    for (const match of line.matchAll(/\b([MVDR])(\d+)\b/g)) {
      const id = `${match[1]}${match[2]}`;
      if (!defined.get(match[1])?.has(id)) {
        reportError(doc.path, index + 1, `${id} is not defined in ${MILESTONES_DOCUMENT}`);
      }
    }
    if (line.includes(EM_DASH)) {
      reportError(doc.path, index + 1, 'use a colon, comma or parentheses instead of an em-dash');
    }
    const mentionsSubagent = /subagent/i.test(line);
    const isGlossary = doc.path === GLOSSARY_DOCUMENT;
    if (mentionsSubagent && !isGlossary) {
      reportError(doc.path, index + 1, 'say "child agent" (see the glossary)');
    }
  });
}

// ADR index and validations --------------------------------------------------------

const adrIndex = requireDoc(ADR_INDEX_DOCUMENT);
/** @type {Map<string, { file: string, status: string, line: number }>} */
const indexed = new Map();
adrIndex.lines.forEach((line, index) => {
  const match = /^\|\s*\[(\d{4})\]\(([^)]+)\)\s*\|/.exec(line);
  if (!match) {
    return;
  }
  const cells = tableCells(line);
  indexed.set(match[1], { file: match[2], status: cells[2] ?? '', line: index + 1 });
});

for (const [number, adr] of adrs) {
  const entry = indexed.get(number);
  const fileName = posix.basename(adr.doc.path);
  const status = adr.table.get('Status')?.value ?? '';
  if (!entry) {
    reportError(ADR_INDEX_DOCUMENT, 1, `ADR-${number} is missing from the index`);
  } else {
    if (entry.file !== fileName) {
      reportError(ADR_INDEX_DOCUMENT, entry.line, `ADR-${number} must link to ${fileName}`);
    }
    if (entry.status !== status) {
      reportError(
        ADR_INDEX_DOCUMENT,
        entry.line,
        `ADR-${number} status "${entry.status}" differs from its file ("${status}")`,
      );
    }
  }
  // The validation rule applies to every Proposed ADR, indexed or not.
  const validation = section(adr.doc, 'Validation');
  if (status.startsWith('Proposed')) {
    if (!validation) {
      reportError(adr.doc.path, 1, 'a Proposed ADR needs a "## Validation" section');
    } else if (!/\b[VM]\d+\b/.test(validation.text)) {
      reportError(
        adr.doc.path,
        validation.line,
        'the Validation section must name the validation (V) or milestone (M) that settles it',
      );
    }
  }
}
for (const [number, entry] of indexed) {
  if (!adrs.has(number)) {
    reportError(ADR_INDEX_DOCUMENT, entry.line, `ADR-${number} is indexed but has no file`);
  }
}

for (const [id, row] of validationRows) {
  const adrNumbers = [...row.matchAll(/\bADR-(\d{4})\b/g)].map((match) => match[1]);
  if (adrNumbers.length === 0) {
    reportError(MILESTONES_DOCUMENT, 1, `${id} must name the ADR it settles`);
  }
  for (const number of adrNumbers) {
    const adr = adrs.get(number);
    if (!adr) {
      continue;
    }
    const validation = section(adr.doc, 'Validation');
    const namesValidation = validation !== undefined && new RegExp(`\\b${id}\\b`).test(validation.text);
    if (!namesValidation) {
      reportError(adr.doc.path, validation?.line ?? 1, `the Validation section must name ${id}`);
    }
  }
}

// Feature documents ----------------------------------------------------------------

const roadmapTargets = new Set(milestonesDoc.lines.flatMap((line) => linkTargets(milestonesDoc, line)));

for (const [path, feature] of features) {
  const status = feature.table.get('Status')?.value ?? '';
  const milestone = feature.table.get('Milestone');
  if (milestone) {
    const isUnscheduled = milestone.value === 'Unscheduled';
    const isDefinedMilestone = defined.get('M')?.has(milestone.value) === true;
    if (!isUnscheduled && !isDefinedMilestone) {
      reportError(path, milestone.line, `milestone must be "Unscheduled" or a defined milestone`);
    }
  }
  if (feature.kind === 'part') {
    const partOf = feature.table.get('Part of');
    const targets = partOf ? linkTargets(feature.doc, partOf.value) : [];
    const readme = posix.join(posix.dirname(path), 'README.md');
    if (partOf && !targets.includes(readme)) {
      reportError(path, partOf.line, `"Part of" must link to README.md of its feature`);
    }
    continue;
  }
  if (SCHEDULED_FEATURE_STATUSES.includes(status)) {
    const folder = posix.dirname(path);
    const listed = roadmapTargets.has(path) || (feature.kind === 'folder' && roadmapTargets.has(folder));
    if (!listed) {
      reportError(path, 1, `an ${status} feature must be linked from ${MILESTONES_DOCUMENT}`);
    }
  }
  if (feature.kind === 'folder') {
    const folder = posix.dirname(path);
    const listedParts = new Set(feature.doc.lines.flatMap((line) => linkTargets(feature.doc, line)));
    for (const part of features.keys()) {
      const isPart = features.get(part)?.kind === 'part';
      const isInFolder = posix.dirname(part) === folder;
      if (isPart && isInFolder && !listedParts.has(part)) {
        reportError(path, 1, `the Parts section must link to ${posix.basename(part)}`);
      }
    }
  }
}

// Numbered headings ------------------------------------------------------------------

for (const doc of allDocs) {
  if (!NUMBERED_HEADING_FOLDERS.some((folder) => doc.path.startsWith(folder))) {
    continue;
  }
  doc.lines.forEach((line, index) => {
    const isNumbered = /^## \d+\. \S/.test(line);
    if (isSecondLevelHeading(doc, index) && !isNumbered) {
      reportError(doc.path, index + 1, 'second-level headings are numbered ("## 1. Title")');
    }
  });
}

// Verification dates -------------------------------------------------------------------

const stackDoc = requireDoc(VERIFIED_DATE_DOCUMENT);
const verifiedDates = stackDoc.lines.flatMap((line, index) =>
  [...line.matchAll(/Verified (\d{4}-\d{2}-\d{2})/g)].map((match) => ({
    date: match[1],
    line: index + 1,
  })),
);
if (verifiedDates.length === 0) {
  reportError(stackDoc.path, 1, 'third-party facts need a "Verified YYYY-MM-DD" date');
}
for (const { date, line } of verifiedDates) {
  const ageDays = (today.getTime() - new Date(`${date}T00:00:00`).getTime()) / 86_400_000;
  if (ageDays > VERIFICATION_WARNING_DAYS) {
    reportWarning(stackDoc.path, line, `facts verified on ${date} are over ${VERIFICATION_WARNING_DAYS} days old`);
  }
}

// Report ---------------------------------------------------------------------------------

for (const warning of warnings) {
  console.warn(`warning: ${warning}`);
}
for (const message of errors) {
  console.error(`error: ${message}`);
}
if (errors.length > 0) {
  console.error(`check-docs: ${errors.length} error(s) in ${allDocs.length} documents`);
  process.exitCode = 1;
} else {
  console.log(`check-docs: ${allDocs.length} documents are consistent`);
}
