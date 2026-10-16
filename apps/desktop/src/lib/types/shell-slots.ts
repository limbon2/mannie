// The UI shell's slots (docs/architecture/extension-points.md, UI shell
// slots; ADR-0020): what a feature registers so that the shell shows it. The
// shell renders whatever is registered and never imports a feature directly.
import type { Component } from 'svelte';

import type { ConversationId, ConversationSummary, PartView, TurnView } from '#lib/generated/bindings.js';

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

/** What the component of a side panel kind receives. */
export interface SidePanelProps {
  /** What the panel shows, as the address names it, such as a conversation's ID. */
  readonly argument: string;
  /** Closes the panel, as when what it shows no longer exists. */
  readonly close: () => void;
  /**
   * Names the panel's tab once the panel knows what it shows. A label that
   * is content, such as a conversation's title, is marked so, and the
   * catalog check of the end-to-end tests skips it (ADR-0019).
   */
  readonly setLabel: (label: string, isContent: boolean) => void;
}

/**
 * A kind of side panel (ADR-0025): content that the shell shows beside the
 * main view, in a tab of the side panel area. A feature opens a panel of its
 * kind through the shell's side panels, with an argument.
 */
export interface SidePanelKind {
  /** The panel's content. */
  readonly component: Component<SidePanelProps>;
  /** The tab's label until the panel names itself. */
  readonly defaultLabel: Message;
  /** An icon from @lucide/svelte, which the tab shows before its label. */
  readonly icon: Component;
  /** Unique among side panel kinds; the address names it, so it has no colon. */
  readonly id: string;
}

/** What a conversation control receives (ADR-0026). */
export interface ConversationControlProps {
  /** The conversation whose toolbar shows the control. */
  readonly conversation: ConversationSummary;
  /**
   * The turns that the conversation view has loaded, oldest first, as the
   * transcript shows them. The control only reads them.
   */
  readonly turns: readonly TurnView[];
}

/**
 * A control in a conversation's toolbar (ADR-0026): a feature's compact
 * button or menu, before the context meter. It renders nothing when it has
 * nothing to offer, and it never takes the focus by itself.
 */
export interface ConversationControl {
  /** The control. */
  readonly component: Component<ConversationControlProps>;
  /** Unique among conversation controls. */
  readonly id: string;
  /** Controls are sorted by this number, lowest first. */
  readonly order: number;
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

/**
 * A kind of task that a plugin's executor runs (ADR-0031, decision 7): how
 * the core's task card, call note, read-only footer and status line name its
 * tasks and their target. Without an entry for a kind, such as while its
 * plugin is off, they show generic texts that name the target.
 */
export interface TaskKind {
  /** The note of a task that the reply let go, which may still run on its target. */
  readonly detached: Message;
  /** An icon from @lucide/svelte for the kind's tasks. */
  readonly icon: Component;
  /** The executor's kind: the ID of its plugin. */
  readonly id: string;
  /** The footer of a task conversation of the kind, which the owner cannot write to. */
  readonly readOnly: Message;
  /** The sentence of one of the executor's refusal codes, with the target's name, or `null` for a code it does not know. */
  readonly refusal: (code: string, target: string | null) => string | null;
  /** The words before the target's name, for screen readers, such as "Worker:". */
  readonly targetPrefix: Message;
  /** The executor's tool, whose calls the call note shows before they have a task. */
  readonly tool: string;
  /** The note of a call of the tool whose activity the model has not written yet. */
  readonly untitledCall: Message;
  /** The status line of a reply that waits only for tasks of the kind. */
  readonly waiting: (count: number) => string;
}
