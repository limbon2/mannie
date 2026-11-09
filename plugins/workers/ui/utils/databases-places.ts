// Where Databases shows things (plugins/workers/docs/databases.md, the
// view): its view, with the database, the tab and the table that it shows,
// which it opens through the SDK's navigation, so the addresses stay the
// shell's. The sidebar lists each database's tables, and a table opens on
// the tab that shows it.
import { type NavigateOptions, openView, viewParameters } from '@mannie/plugin-sdk/navigation';

import type { DatabaseTab } from '../types/databases.js';

/** The feature's view, which is also the ID of its entry of the activity bar. */
export const DATABASES_VIEW = 'workers.databases';

/** The parameter of the view that names the database it shows. */
export const DATABASE_PARAMETER = 'database';

/** The parameter of the view that names the tab it shows. */
export const TAB_PARAMETER = 'tab';

/** The parameter of the view that names the table of the Structure and Data tabs. */
export const TABLE_PARAMETER = 'table';

/** The feature's ID, as the plugin's manifest names it. */
export const DATABASES_FEATURE = 'databases';

/** The tabs of a database's page, in their order. */
export const DATABASE_TABS: readonly DatabaseTab[] = ['data', 'structure', 'sql'];

/** The database whose page the view shows, or `null`; read in a component, it follows the address. */
export function shownDatabaseKey(): string | null {
  return viewParameters().get(DATABASE_PARAMETER);
}

/** The tab that the view shows, the Data tab when the address names none it knows. */
export function shownTab(): DatabaseTab {
  const tab = viewParameters().get(TAB_PARAMETER);
  return DATABASE_TABS.find((known) => known === tab) ?? 'data';
}

/** The tab on which a table from the sidebar opens: the Structure tab while it shows, else the Data tab. */
export function tableTab(): DatabaseTab {
  return shownTab() === 'structure' ? 'structure' : 'data';
}

/** The table of the Structure and Data tabs, or `null` for the first one. */
export function shownTable(): string | null {
  return viewParameters().get(TABLE_PARAMETER);
}

/** Shows the view without a database. */
export function openDatabases(): Promise<void> {
  return openView(DATABASES_VIEW);
}

/** Shows a database's page on a tab, with a table for the Structure and Data tabs. */
export function openDatabase(
  key: string,
  tab: DatabaseTab = 'data',
  table: string | null = null,
  options?: NavigateOptions,
): Promise<void> {
  const parameters: Record<string, string> = { [DATABASE_PARAMETER]: key, [TAB_PARAMETER]: tab };
  if (table !== null) {
    parameters[TABLE_PARAMETER] = table;
  }
  return openView(DATABASES_VIEW, parameters, options);
}
