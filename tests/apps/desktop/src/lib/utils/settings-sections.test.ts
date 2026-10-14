// The pages of the settings (docs/architecture/extension-points.md, settings
// sections): which section a URL shows, and how the settings open at one.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { Slot } from '../../../../../../apps/desktop/src/lib/state/shell-slots.svelte.js';
import type { SettingsSection } from '../../../../../../apps/desktop/src/lib/types/shell-slots.js';
import {
  activeSettingsSection,
  openSettings,
  SECTION_PARAMETER,
  settingsSectionQuery,
} from '../../../../../../apps/desktop/src/lib/utils/settings-sections.js';

const navigation = vi.hoisted(() => ({ goto: vi.fn() }));

vi.mock('$app/navigation', () => ({ goto: navigation.goto }));
vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));

function section(id: string, order: number): SettingsSection {
  // The components are never rendered here, so any component-shaped value will do.
  const component = () => ({});
  return { component, description: () => id, icon: component, id, order, title: () => id };
}

function url(search: string): URL {
  return new URL(`https://tauri.localhost/settings${search}`);
}

beforeEach(() => {
  navigation.goto.mockReset();
});

describe('activeSettingsSection', () => {
  it('shows nothing when no section is registered', () => {
    const sections = new Slot<SettingsSection>('settings sections');

    expect(activeSettingsSection(sections, url(''))).toEqual({ kind: 'none' });
  });

  it('shows the section with the lowest order when the URL names none', () => {
    const sections = new Slot<SettingsSection>('settings sections');
    sections.register(section('chat.providers', 10));
    const interfaceSection = section('shell.interface', 0);
    sections.register(interfaceSection);

    expect(activeSettingsSection(sections, url(''))).toEqual({ kind: 'section', section: interfaceSection });
  });

  it('shows the section the URL names', () => {
    const sections = new Slot<SettingsSection>('settings sections');
    sections.register(section('shell.interface', 0));
    const providers = section('chat.providers', 10);
    sections.register(providers);

    expect(activeSettingsSection(sections, url(`?${settingsSectionQuery('chat.providers')}`))).toEqual({
      kind: 'section',
      section: providers,
    });
  });

  it('reports a section that no feature provides', () => {
    const sections = new Slot<SettingsSection>('settings sections');
    sections.register(section('shell.interface', 0));

    expect(activeSettingsSection(sections, url(`?${SECTION_PARAMETER}=removed`))).toEqual({
      kind: 'missing',
      requested: 'removed',
    });
  });
});

describe('openSettings', () => {
  it('opens the first section without an ID, and the named section with one', async () => {
    await openSettings();
    await openSettings('chat.providers');

    expect(navigation.goto).toHaveBeenNthCalledWith(1, '/settings');
    expect(navigation.goto).toHaveBeenNthCalledWith(2, '/settings?section=chat.providers');
  });
});
