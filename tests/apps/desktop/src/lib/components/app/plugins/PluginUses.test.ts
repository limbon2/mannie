// What a plugin may use, below its description on the Plugins page
// (docs/features/plugins.md; ADR-0034, decision 13), rendered on the server
// side: collapsed at first, with the list hidden; open, the services in words with their names and
// the sensitive ones marked, the entries in the credential store, the
// commands and channels that its window may call, its programs, the
// installed programs that it runs, the libraries that only it may use and
// its system access outside Mannie's services in words with their names;
// and a line for a plugin that uses nothing.
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import PluginUses from '../../../../../../../../apps/desktop/src/lib/components/app/plugins/PluginUses.svelte';
import type { PluginUses as Uses } from '../../../../../../../../apps/desktop/src/lib/generated/bindings.js';

/** What the Workers plugin's manifest lets it use. */
const WORKERS_USES: Uses = {
  channels: ['subscribe_workers'],
  commands: ['pair_worker', 'rename_worker', 'remove_worker'],
  crates: ['protox', 'tonic', 'tonic-prost', 'tonic-prost-build'],
  installedPrograms: [],
  native: ['sockets'],
  programs: ['mannie-worker'],
  secrets: ['link/master-key'],
  services: [
    { major: 1, name: 'credentials', sensitive: true },
    { major: 1, name: 'executor-host', sensitive: false },
    { major: 1, name: 'secrets', sensitive: true },
    { major: 1, name: 'store', sensitive: false },
  ],
};

/** What a plugin that runs the installed git may use. */
const GIT_USES: Uses = {
  channels: [],
  commands: ['repository_overview'],
  crates: [],
  installedPrograms: ['git'],
  native: [],
  programs: [],
  secrets: [],
  services: [{ major: 1, name: 'processes', sensitive: true }],
};

function shown(uses: Uses, open: boolean): string {
  return render(PluginUses, { props: { open, uses } }).body;
}

function count(text: string, part: string): number {
  return text.split(part).length - 1;
}

/** The list's element, which carries `hidden` while the list is collapsed. */
const HIDDEN_LIST = /data-plugin-uses=""[^>]* hidden=""/;

describe('PluginUses', () => {
  it('starts collapsed, with the button that shows the list and the list hidden', () => {
    const body = shown(WORKERS_USES, false);

    expect(body).toContain('What it may use');
    expect(body).toContain('aria-expanded="false"');
    expect(body).toMatch(HIDDEN_LIST);
  });

  it('lists the services in words and by name, marks the sensitive ones, and lists the entries', () => {
    const body = shown(WORKERS_USES, true);

    expect(body).toContain('aria-expanded="true"');
    expect(body).not.toMatch(HIDDEN_LIST);
    for (const name of ['credentials', 'executor-host', 'secrets', 'store']) {
      expect(body).toContain(`data-plugin-service="${name}"`);
      expect(body).toContain(`>${name}</code>`);
    }
    // The names are the plugin's, so they show as data and not as text of
    // the catalog.
    expect(count(body, 'data-content')).toBe(15);
    expect(body).toContain('Provider credentials, lent to its own tasks');
    expect(count(body, 'data-plugin-service-sensitive')).toBe(2);
    expect(body).toContain('data-plugin-secret="link/master-key"');
    for (const command of ['pair_worker', 'rename_worker', 'remove_worker']) {
      expect(body).toContain(`data-plugin-command="${command}"`);
    }
    expect(body).toContain('data-plugin-channel="subscribe_workers"');
    expect(body).toContain('Commands that its window may call');
    expect(body).toContain('data-plugin-program="mannie-worker"');
    for (const name of ['protox', 'tonic', 'tonic-prost', 'tonic-prost-build']) {
      expect(body).toContain(`data-plugin-crate="${name}"`);
    }
    expect(body).toContain('data-plugin-native="sockets"');
    expect(body).toContain('Opens network connections itself');
    expect(body).not.toContain('Installed programs that it runs');
  });

  it('lists the installed programs that a plugin runs, with the sensitive service that runs them', () => {
    const body = shown(GIT_USES, true);

    expect(body).toContain('Installed programs that it runs');
    expect(body).toContain('data-plugin-installed-program="git"');
    expect(body).toContain('>git</code>');
    expect(body).toContain('Installed programs that its manifest names, under your approvals');
    expect(body).toContain('data-plugin-service="processes"');
    expect(count(body, 'data-plugin-service-sensitive')).toBe(1);
  });

  it('says so for a plugin that uses nothing', () => {
    const body = shown(
      {
        channels: [],
        commands: [],
        crates: [],
        installedPrograms: [],
        native: [],
        programs: [],
        secrets: [],
        services: [],
      },
      true,
    );

    expect(body).toContain('Nothing beyond its own parts.');
    expect(body).not.toContain('<dl');
  });
});
