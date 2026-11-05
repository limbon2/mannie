// @ts-check
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { inlangSettings, messageProblems, messageSources } from '../../../scripts/plugins/lib/messages.mjs';

const SOURCES = messageSources([
  { folder: 'plugins/workers', id: 'workers', manifest: { features: { databases: {}, 'query-history': {} } } },
  { folder: 'plugins/log-viewer', id: 'log-viewer', manifest: {} },
]);

/** The project's locales. */
const SETTINGS = { baseLocale: 'en', locales: ['en'] };

/**
 * Catalogs by source and locale.
 * @param {Record<string, Record<string, Record<string, unknown>>>} bySource
 */
function catalogs(bySource) {
  return new Map(Object.entries(bySource).map(([name, byLocale]) => [name, new Map(Object.entries(byLocale))]));
}

/** The catalogs of the sources, as they should be. */
const VALID = {
  core: { en: { $schema: 'schema', chat_send: 'Send' } },
  'log-viewer': {
    en: { log_viewer_plugin_description: 'Shows the logs.', log_viewer_plugin_name: 'Log viewer' },
  },
  workers: {
    en: {
      workers_feature_databases_description: 'Opens SQLite files.',
      workers_feature_databases_name: 'Databases',
      workers_feature_query_history_description: 'Keeps the queries.',
      workers_feature_query_history_name: 'Query history',
      workers_pair: 'Pair',
      workers_plugin_description: 'Pairs workers.',
      workers_plugin_name: 'Workers',
    },
  },
};

describe('messageSources', () => {
  it("lists the core's catalogs, then each plugin's with its prefix", () => {
    assert.deepEqual(SOURCES, [
      { features: [], folder: 'apps/desktop/messages', name: 'core', prefix: null },
      {
        features: ['databases', 'query-history'],
        folder: 'plugins/workers/messages',
        name: 'workers',
        prefix: 'workers',
      },
      { features: [], folder: 'plugins/log-viewer/messages', name: 'log-viewer', prefix: 'log_viewer' },
    ]);
  });
});

describe('inlangSettings', () => {
  it("lists every source's path and moves the modules to the project's folder", () => {
    const settings = inlangSettings(
      {
        baseLocale: 'en',
        modules: ['./node_modules/@inlang/plugin-message-format/dist/index.js', 'https://example.org/plugin.js'],
        'plugin.inlang.messageFormat': { pathPattern: './messages/{locale}.json' },
      },
      SOURCES,
      'apps/desktop/.generated',
    );

    assert.deepEqual(settings.modules, [
      '../node_modules/@inlang/plugin-message-format/dist/index.js',
      'https://example.org/plugin.js',
    ]);
    assert.deepEqual(settings['plugin.inlang.messageFormat'], {
      pathPattern: [
        '../messages/{locale}.json',
        '../../../plugins/workers/messages/{locale}.json',
        '../../../plugins/log-viewer/messages/{locale}.json',
      ],
    });
  });
});

describe('messageProblems', () => {
  it('accepts catalogs that keep to their prefixes', () => {
    assert.deepEqual(messageProblems(SOURCES, catalogs(VALID), SETTINGS), []);
  });

  it("refuses a plugin's key outside its prefix and a core key under a plugin's prefix", () => {
    const planted = {
      ...VALID,
      core: { en: { chat_send: 'Send', workers_pair_title: 'Pair a worker' } },
      'log-viewer': { en: { ...VALID['log-viewer'].en, log_open: 'Open' } },
    };

    assert.deepEqual(messageProblems(SOURCES, catalogs(planted), SETTINGS), [
      'apps/desktop/messages/en.json: workers_pair_title is under the prefix of the plugin workers, so it belongs in its catalog',
      "plugins/log-viewer/messages/en.json: log_open is outside the plugin's prefix log_viewer_",
    ]);
  });

  it('refuses a key in two sources, a missing name and an unregistered locale', () => {
    const planted = {
      ...VALID,
      'log-viewer': {
        de: { log_viewer_plugin_name: 'Protokolle' },
        en: { log_viewer_plugin_description: 'Shows the logs.', workers_pair: 'Pair' },
      },
    };

    assert.deepEqual(messageProblems(SOURCES, catalogs(planted), SETTINGS), [
      'plugins/log-viewer/messages/de.json: the core registers no locale de in project.inlang/settings.json',
      "plugins/log-viewer/messages/en.json: workers_pair is outside the plugin's prefix log_viewer_",
      'plugins/log-viewer/messages/en.json: workers_pair is also in the catalog of workers',
      'plugins/log-viewer/messages/en.json: the plugin needs the message log_viewer_plugin_name, which names it on the Plugins page',
    ]);
  });

  it("refuses a feature without its name or description, with underscores in place of the ID's hyphens", () => {
    const {
      workers_feature_databases_name: _name,
      workers_feature_query_history_description: _description,
      ...rest
    } = VALID.workers.en;
    const planted = { ...VALID, workers: { en: rest } };

    assert.deepEqual(messageProblems(SOURCES, catalogs(planted), SETTINGS), [
      'plugins/workers/messages/en.json: the feature databases needs the message workers_feature_databases_name, which names it on the Plugins page',
      'plugins/workers/messages/en.json: the feature query-history needs the message workers_feature_query_history_description, which names it on the Plugins page',
    ]);
  });
});
