// The MCP server of the end-to-end tests: the example program of the
// plugin's test-only crate, `mannie-mcp-testing`, which this file builds with
// cargo, because a `[[programs]]` entry of the manifest would ship it in
// releases. The app starts it itself as a local server, by the absolute path
// that the spec types into the server's dialog.
import { execFileSync } from 'node:child_process';
import { join, resolve } from 'node:path';

import { repository } from '#e2e/app.ts';

/**
 * How long a hook that builds the server may take. On a new checkout, cargo
 * first builds rmcp and its dependencies for the server.
 */
export const BUILD_TIMEOUT_MILLIS = 600_000;

/** The variable whose value the server writes to its error output and its tool `read_variable` returns. */
export const SECRET_VARIABLE = 'MCP_TEST_TOKEN';

/** The text of the server's resource `test://readme`. */
export const README_TEXT = 'Read me first: this server only tests.';

/** The example program in the shared target folder. */
export function serverProgram(): string {
  const binary = process.platform === 'win32' ? 'mcp-test-server.exe' : 'mcp-test-server';
  const targetDirectory = resolve(repository, process.env.CARGO_TARGET_DIR ?? 'target');
  return join(targetDirectory, 'debug', 'examples', binary);
}

/** Builds the example program; cargo builds nothing when it is current. */
export function buildServer(): void {
  execFileSync(
    'cargo',
    [
      'build',
      '--locked',
      '--manifest-path',
      join(repository, 'plugins', 'mcp', 'Cargo.toml'),
      '-p',
      'mannie-mcp-testing',
      '--example',
      'mcp-test-server',
    ],
    { stdio: 'inherit' },
  );
}
