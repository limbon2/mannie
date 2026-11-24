// Preserve the complete performance reports in the job log and summary.
// GitHub's artifact storage quota does not apply to either destination.
import { appendFileSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const REPORTS = [
  { heading: 'Performance measurements', path: join('target', 'perf', 'results.json') },
  { heading: 'Terminal performance measurements', path: join('target', 'perf', 'terminal.json') },
];

function readOptional(file) {
  try {
    return readFileSync(file, 'utf8');
  } catch (error) {
    if (error.code === 'ENOENT') {
      return undefined;
    }
    throw error;
  }
}

const sections = [];
for (const report of REPORTS) {
  const contents = readOptional(report.path);
  if (contents !== undefined) {
    sections.push(
      `## ${report.heading}\n\n\`\`\`json\n${JSON.stringify(JSON.parse(contents), undefined, 2)}\n\`\`\`\n`,
    );
  }
}

const summary =
  sections.length === 0
    ? '## Performance measurements\n\nThe performance run did not produce a report.\n'
    : sections.join('\n');
process.stdout.write(summary);
if (process.env.GITHUB_STEP_SUMMARY) {
  appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${summary}\n`);
}
