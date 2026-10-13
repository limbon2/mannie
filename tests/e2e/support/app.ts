// The app under test: the validation build of the desktop app (`task e2e`),
// which uses its own bundle identifier, so the tests never touch the owner's
// data.
import { mkdirSync, rmSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { fileURLToPath } from 'node:url';

/** The bundle identifier of validation builds (src-tauri/tauri.validation.conf.json). */
export const VALIDATION_IDENTIFIER = 'io.github.limbon2.mannie.validation';

/** The repository's root folder. */
export const repository = fileURLToPath(new URL('../../..', import.meta.url));

/** The validation build of the desktop app. */
export function applicationPath(): string {
  const binary = process.platform === 'win32' ? 'mannie-desktop.exe' : 'mannie-desktop';
  return join(repository, 'target', 'debug', binary);
}

/** The data folder of validation builds: the database, the logs and the webview's storage. */
export function dataDirectory(): string {
  if (process.platform === 'win32') {
    const localAppData = process.env.LOCALAPPDATA ?? join(homedir(), 'AppData', 'Local');
    return join(localAppData, VALIDATION_IDENTIFIER);
  }
  const dataHome = process.env.XDG_DATA_HOME ?? join(homedir(), '.local', 'share');
  return join(dataHome, VALIDATION_IDENTIFIER);
}

/** Deletes the data folder, so the app starts as on a new machine. */
export function resetDataDirectory(): void {
  // The webview's processes can hold the folder for a moment after the app ends.
  rmSync(dataDirectory(), { force: true, maxRetries: 20, recursive: true, retryDelay: 250 });
}

/** Leaves a database that a future version of Mannie wrote, with a schema version this one does not know. */
export function plantDatabaseFromTheFuture(): void {
  mkdirSync(dataDirectory(), { recursive: true });
  const database = new DatabaseSync(join(dataDirectory(), 'mannie.sqlite'));
  database.exec('PRAGMA journal_mode = WAL; CREATE TABLE future_table (id INTEGER); PRAGMA user_version = 999;');
  database.close();
}
