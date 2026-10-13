// The UI shell's slots (docs/architecture/extension-points.md, UI shell
// slots): what a feature registers so that the shell shows it. The shell
// renders whatever is registered and never imports a feature directly.
import type { Component } from 'svelte';

/** A message function from the catalog, called when the text is shown (ADR-0019). */
export type Message = () => string;

/** A command in the command palette. */
export interface ShellCommand {
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

/** A section of the settings view. */
export interface SettingsSection {
  /** The section's content. */
  readonly component: Component;
  /** One sentence about what the section changes. */
  readonly description: Message;
  /** Unique among settings sections. */
  readonly id: string;
  /** Sections are sorted by this number, lowest first. */
  readonly order: number;
  /** The section's heading. */
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
}
