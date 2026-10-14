// The UI shell's slots (docs/architecture/extension-points.md, UI shell
// slots; ADR-0020): what a feature registers so that the shell shows it. The
// shell renders whatever is registered and never imports a feature directly.
import type { Component } from 'svelte';

import type { ConversationId, PartView } from '#lib/generated/bindings.js';

/** A message function from the catalog, called when the text is shown (ADR-0019). */
export type Message = () => string;

/** A command in the command palette. */
export interface ShellCommand {
  /**
   * Whether the command applies now, such as a command for the open
   * conversation. The palette asks when it opens; without it, the command
   * is always available.
   */
  readonly available?: () => boolean;
  /** The group the palette lists it under. */
  readonly group: Message;
  /** Unique among commands. */
  readonly id: string;
  /** What the command does, as a label. */
  readonly label: Message;
  /** Commands of a group are sorted by this number, lowest first. */
  readonly order: number;
  /** Runs the command. */
  readonly run: () => void;
}

/** An entry in the sidebar that opens a view. */
export interface NavigationEntry {
  /** An icon from @lucide/svelte. */
  readonly icon: Component;
  /** Unique among navigation entries. */
  readonly id: string;
  /** The entry's label. */
  readonly label: Message;
  /** Entries are sorted by this number, lowest first. */
  readonly order: number;
  /** The view the entry opens. */
  readonly view: string;
}

/**
 * A section of the settings. Each section is a page of its own, which the
 * settings sidebar lists by its icon and title.
 */
export interface SettingsSection {
  /** The section's content. */
  readonly component: Component;
  /** One sentence about what the section changes. */
  readonly description: Message;
  /** An icon from @lucide/svelte, which the settings sidebar shows beside the title. */
  readonly icon: Component;
  /** Unique among settings sections; the `section` search parameter of the settings selects it. */
  readonly id: string;
  /** Sections are sorted by this number, lowest first. */
  readonly order: number;
  /** The section's heading, and its entry in the settings sidebar. */
  readonly title: Message;
}

/** A view of the main area. */
export interface ShellView {
  /** The view's content. */
  readonly component: Component;
  /** Unique among views; the `view` search parameter selects it. */
  readonly id: string;
  /** The view's heading. */
  readonly title: Message;
  /**
   * Whether the heading is content, such as a conversation's title, rather
   * than text from the catalog. The catalog check of the end-to-end tests
   * skips it (ADR-0019).
   */
  readonly titleIsContent?: () => boolean;
}

/** A panel of the sidebar, shown after the navigation entries. */
export interface SidebarPanel {
  /** The panel's content. */
  readonly component: Component;
  /** Unique among sidebar panels. */
  readonly id: string;
  /** Panels are sorted by this number, lowest first. */
  readonly order: number;
}

/**
 * A component that the shell mounts once for the life of the window, for
 * dialogs that belong to no view. It renders nothing until it has something
 * to show.
 */
export interface Overlay {
  /** The overlay. */
  readonly component: Component;
  /** Unique among overlays. */
  readonly id: string;
}

/** What a part renderer receives. */
export interface PartRendererProps {
  /** The conversation the part belongs to. */
  readonly conversationId: ConversationId;
  /** Whether the part belongs to the conversation's latest turn. */
  readonly latest: boolean;
  /** The part. */
  readonly part: PartView;
}

/** The component that renders one kind of transcript part. */
export interface PartRenderer {
  /** The renderer. It never inserts raw HTML (ADR-0020). */
  readonly component: Component<PartRendererProps>;
  /** The part kind it renders, such as `text`; unique among renderers. */
  readonly id: string;
}
