import { describe, expect, it } from 'vitest';

import { Slot } from '../../../../../../apps/desktop/src/lib/state/shell-slots.svelte.js';
import type { ShellView } from '../../../../../../apps/desktop/src/lib/types/shell-slots.js';
import { activeView, VIEW_PARAMETER } from '../../../../../../apps/desktop/src/lib/utils/views.js';

function view(id: string): ShellView {
  // The component is never rendered here, so any component-shaped value will do.
  return { component: () => ({}), id, title: () => id };
}

function url(search: string): URL {
  return new URL(`https://tauri.localhost/${search}`);
}

function everyPluginOn(): boolean {
  return true;
}

describe('activeView', () => {
  it('shows nothing when no view is registered', () => {
    const views = new Slot<ShellView>('views', everyPluginOn);

    expect(activeView(views, url(''))).toEqual({ kind: 'none' });
  });

  it('shows the first registered view when the URL names none', () => {
    const views = new Slot<ShellView>('views', everyPluginOn);
    const chat = view('chat');
    views.register(chat);
    views.register(view('workers'));

    expect(activeView(views, url(''))).toEqual({ kind: 'view', view: chat });
  });

  it('shows the view the URL names', () => {
    const views = new Slot<ShellView>('views', everyPluginOn);
    views.register(view('chat'));
    const workers = view('workers');
    views.register(workers);

    expect(activeView(views, url(`?${VIEW_PARAMETER}=workers`))).toEqual({ kind: 'view', view: workers });
  });

  it('reports a view that no feature provides', () => {
    const views = new Slot<ShellView>('views', everyPluginOn);
    views.register(view('chat'));

    expect(activeView(views, url(`?${VIEW_PARAMETER}=removed`))).toEqual({ kind: 'missing', requested: 'removed' });
  });

  it('says that the plugin of the view the URL names is off', () => {
    const views = new Slot<ShellView>('views', () => false);
    views.register(view('chat'));
    views.register(view('jira'), 'jira');

    expect(activeView(views, url(`?${VIEW_PARAMETER}=jira`))).toEqual({ kind: 'plugin_off', plugin: 'jira' });
  });
});
