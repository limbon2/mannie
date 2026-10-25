// The plugin's entries in the shell (ADR-0030, decision 11; ADR-0045,
// ADR-0046, ADR-0050, ADR-0052): the Notebooks entry of the activity bar
// after Chat with its view and its group of the sidebar, the dialogs and the
// viewer, the kinds of context and of mentions, the cards of its tools, the
// command and the settings section show only while the plugin is on, and the
// notebooks follow the plugin from its registration.
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { describe, expect, it, vi } from 'vitest';

import plugin from '../../ui/index.js';

const library = vi.hoisted(() => ({ find: vi.fn(), findSource: vi.fn(), follow: vi.fn() }));

// The notebooks follow the plugin's channel, which these tests do not open.
vi.mock('../../ui/state/notebooks.svelte.js', () => ({ notebookLibrary: library }));
vi.mock('@mannie/plugin-sdk/navigation', () => ({
  openSettings: vi.fn(),
  openView: vi.fn(),
  viewParameters: () => new URLSearchParams(),
}));

/** A call of one of the plugin's tools, as its card reads it. */
function toolCall(argumentsOf: Record<string, unknown>, data: Record<string, unknown> | null) {
  return { arguments: argumentsOf, data, failure: null, status: 'completed' as const };
}

describe('the plugin', () => {
  it('registers its entries, which show only while it is on', () => {
    const context = new TestPluginContext('notebooks');

    plugin.register(context);
    const whileOff = [context.shell.navigation.entries, context.shell.views.entries];
    context.turnOn();

    expect(whileOff).toEqual([[], []]);
    expect(context.shell.navigation.entries.map((entry) => [entry.id, entry.order, entry.view])).toEqual([
      ['notebooks', 10, 'notebooks'],
    ]);
    expect(context.shell.views.entries.map((view) => view.id)).toEqual(['notebooks']);
    expect(context.shell.sidebarPanels.entries.map((panel) => [panel.id, panel.activity])).toEqual([
      ['notebooks.notebooks', 'notebooks'],
    ]);
    // The viewer is a dialog of the overlay, and "@" attaches notebooks, so
    // the plugin has neither a side panel nor a button in the message box.
    expect(context.shell.sidePanels.entries).toEqual([]);
    expect(context.shell.conversationControls.entries).toEqual([]);
    expect(context.shell.overlays.entries.map((overlay) => overlay.id)).toEqual(['notebooks.dialogs']);
    expect(context.shell.commands.entries.map((command) => command.id)).toEqual(['notebooks.new-notebook']);
    expect(context.shell.settingsSections.entries.map((section) => section.id)).toEqual(['notebooks.settings']);
    expect(library.follow).toHaveBeenCalledWith(context);
  });

  it('words its kind of context and opens its passages', () => {
    const context = new TestPluginContext('notebooks', true);

    plugin.register(context);
    const kind = context.shell.contextKinds.find('notebooks');

    expect(kind?.noun()).toBe('Notebook');
    expect(kind?.gathering()).toBe('Searching notebooks…');
    expect(kind?.found(['Physics'], 2)).toBe('Searched Physics: 2 passages');
    expect(kind?.failure('notebook_not_found')).toBe('The notebook was deleted.');
    expect(kind?.open?.label()).toBe('Open source');
  });

  it('words the notebooks that "@" lists and attaches', () => {
    const context = new TestPluginContext('notebooks', true);

    plugin.register(context);
    const kind = context.shell.mentionKinds.find('notebooks');

    expect(kind?.group()).toBe('Notebooks');
    expect(kind?.variantName('attachment')).toBe('Notebook');
    expect(kind?.variantName('file')).toBeNull();
    expect(kind?.summary({ code: 'attached', values: {} })).toBe('Attached to this chat');
    expect(kind?.refusal('notebook_not_found')).toBe('The notebook was deleted.');
    expect(kind?.warning('anything')).toBeNull();
  });

  it('describes the calls of its tools', () => {
    const context = new TestPluginContext('notebooks', true);
    library.findSource.mockReturnValue({ name: 'guide.pdf' });

    plugin.register(context);
    const kind = (tool: string) => context.shell.toolKinds.find(tool);
    const search = kind('search_notebooks');
    const read = kind('read_notebook_source');

    expect(kind('list_notebook_sources')?.summary(toolCall({}, null))).toBe(
      'Listed the sources of the attached notebooks',
    );
    expect(search?.summary(toolCall({ query: 'ssh proxy' }, null))).toBe('Searched the notebooks for "ssh proxy"');
    expect(search?.summary(toolCall({ query: 'ssh proxy' }, { found: 3, query: 'ssh proxy' }))).toBe(
      'Found 3 passages for "ssh proxy"',
    );
    expect(read?.summary(toolCall({ source: 's1' }, null))).toBe('Read guide.pdf');
    expect(read?.summary(toolCall({ source: 's1' }, { firstPage: 3, lastPage: 4, source: 'guide.pdf' }))).toBe(
      'Read guide.pdf, pages 3 to 4',
    );
    expect(read?.summary(toolCall({ source: 's1' }, { firstPage: 3, lastPage: 3, source: 'guide.pdf' }))).toBe(
      'Read guide.pdf, page 3',
    );
    expect(read?.summary(toolCall({ source: 's2' }, { heading: 'Lenses', source: 'notes.md' }))).toBe(
      'Read notes.md, Lenses',
    );
    expect(read?.failure('page_not_found')).toBe('The source has no text from that page on.');
    expect(search?.failure('no_embedding_profile')).not.toBeNull();
    expect(search?.failure('unknown_code')).toBeNull();
  });

  it('names its view after the notebook it shows', () => {
    const context = new TestPluginContext('notebooks', true);
    library.find.mockReturnValue(undefined);

    plugin.register(context);
    const view = context.shell.views.entries.at(0);

    expect(view?.title()).toBe('Notebooks');
    expect(view?.titleIsContent?.()).toBe(false);
  });

  it('words its start failure and asks before it stops while sources are indexed', () => {
    expect(plugin.failure?.('storage_failed')).toContain('could not read its notebooks');
    expect(plugin.offQuestion?.([])).toBeNull();
    expect(plugin.offQuestion?.([{ code: 'indexing_sources', count: 2 }])).toContain('2 sources');
  });
});
