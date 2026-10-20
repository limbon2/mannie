// Preserve the complete performance report in the job log and summary.
// GitHub's artifact storage quota does not apply to either destination.
import { appendFileSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const report = join('target', 'perf', 'results.json');
let contents;
try {
  contents = readFileSync(report, 'utf8');
} catch (error) {
  if (error.code !== 'ENOENT') {
    throw error;
  }
}

const summary =
  contents === undefined
    ? '## Performance measurements\n\nThe performance run did not produce a report.\n'
    : `## Performance measurements\n\n\`\`\`json\n${JSON.stringify(JSON.parse(contents), undefined, 2)}\n\`\`\`\n`;
process.stdout.write(summary);
if (process.env.GITHUB_STEP_SUMMARY) {
  appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${summary}\n`);
}
