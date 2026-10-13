// A database written by a newer version of Mannie (ADR-0009): the core refuses
// it and leaves it untouched, and the window says what failed and what to do.
// wdio.conf.ts plants the database before this spec's session starts.
import { join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';

import { dataDirectory } from '../support/app.ts';
import { axeViolations, waitForApp } from '../support/page.ts';

describe('a database from a newer version of Mannie', () => {
  before(async () => {
    await waitForApp();
  });

  it('shows what failed and what the owner can do', async () => {
    await expect($('h1')).toHaveText('Mannie could not start');
    await expect($('main')).toHaveText(expect.stringContaining('schema version 999'));
    await expect($('main')).toHaveText(expect.stringContaining('Install the newer version of Mannie'));
    expect(await axeViolations()).toEqual([]);
  });

  it('leaves the database untouched', () => {
    const database = new DatabaseSync(join(dataDirectory(), 'mannie.sqlite'), { readOnly: true });
    const version = database.prepare('PRAGMA user_version').get();
    database.close();

    expect(version).toEqual({ user_version: 999 });
  });
});
