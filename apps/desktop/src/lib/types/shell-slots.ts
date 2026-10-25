// The UI shell's slots (docs/architecture/extension-points.md, UI shell
// slots; ADR-0020): what a feature registers so that the shell shows it. The
// shell renders whatever is registered and never imports a feature directly.
import type { Component } from 'svelte';

import type {
  Attachment,
  AttachmentRef,
  BackgroundCommandView,
  ConversationId,
  ConversationSummary,
  MentionSummary,
  PartView,
  ToolCallStatus,
  TurnView,
} from '#lib/generated/bindings.js';

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

/**
 * An entry of the activity bar (ADR-0045): selecting it shows its sidebar
 * panels and its view at the address that the view last had under it.
 */
export interface NavigationEntry {
  /** An icon from @lucide/svelte. */
  readonly icon: Component;
  /** Unique among navigation entries; sidebar panels name it as their `activity`. */
  readonly id: string;
  /** The entry's label, which its button shows as a tooltip and names itself with. */
  readonly label: Message;
  /** Entries are sorted by this number, lowest first, and the first one's panels take those that name none. */
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

/** A panel of the sidebar, shown while the activity bar selects its navigation entry (ADR-0045, decision 3). */
export interface SidebarPanel {
  /**
   * The ID of the navigation entry that the panel belongs to. Without it, the
   * panel belongs to the entry with the lowest order, which is Chat's.
   */
  readonly activity?: string;
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
  /**
   * The conversation's background commands, by number (ADR-0041). A view
   * of a version before them passes none.
   */
  readonly backgroundCommands?: readonly BackgroundCommandView[];
  /** The conversation that the control belongs to. */
  readonly conversation: ConversationSummary;
  /**
   * Where the focus returns when a side panel or a dialog that the control
   * opened closes (ADR-0042, decision 5): the menu's button for a control of
   * the conversation's menu, whose own entry is gone once the menu closes.
   * Without it, the control passes its own button.
   */
  readonly opener?: HTMLElement | null;
  /**
   * The turns that the conversation view has loaded, oldest first, as the
   * transcript shows them. The control only reads them.
   */
  readonly turns: readonly TurnView[];
}

/**
 * Where a conversation control shows (ADR-0042): `toolbar`, a compact button
 * or menu in the conversation's toolbar; `menu`, entries of the
 * conversation's menu at the toolbar's end, built from the dropdown menu's
 * items and submenus; `message-box`, a compact button in the row below the
 * message box's text.
 */
export type ConversationControlPlace = 'menu' | 'message-box' | 'toolbar';

/**
 * The conversation that the first message of the new conversation pane
 * starts, as the controls of its message box change it (ADR-0046, decision
 * 6). The core asks each attachment's provider again when the message is
 * sent.
 */
export interface ConversationDraft {
  /** Adds an attachment with the label that its plugin knows, or gives one that the draft holds that label. */
  readonly attach: (attachment: Attachment) => void;
  /** What the conversation starts with attached, in the order the owner attached them. */
  readonly attachments: readonly Attachment[];
  /** Removes an attachment; one that the draft does not hold changes nothing. */
  readonly detach: (attachment: AttachmentRef) => void;
  /** The project that the conversation starts in, or `null` for a general conversation. */
  readonly projectId: string | null;
}

/** What the control of a new conversation's message box receives. */
export interface DraftControlProps {
  /** The conversation that the first message starts. */
  readonly draft: ConversationDraft;
}

/**
 * A control of a conversation (ADR-0026, ADR-0042): a feature's button,
 * menu or menu entries, in the place that it names. It renders nothing when
 * it has nothing to offer, and it never takes the focus by itself.
 */
export interface ConversationControl {
  /** The control. */
  readonly component: Component<ConversationControlProps>;
  /**
   * The control of a new conversation's message box, for a control of the
   * message box: it changes what the first message starts, such as its
   * attachments (ADR-0046, decision 6). Without it, the control shows only
   * in conversations.
   */
  readonly draft?: Component<DraftControlProps>;
  /** Unique among conversation controls. */
  readonly id: string;
  /** Controls are sorted by this number, lowest first. */
  readonly order: number;
  /** Where the control shows; `toolbar` when it names no place. */
  readonly place?: ConversationControlPlace;
  /**
   * How many of a menu control's things work, such as its running commands,
   * which the menu's button adds up in a badge, so that running work shows
   * while the menu is closed (ADR-0042, decision 3).
   */
  readonly running?: (props: ConversationControlProps) => number;
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

/**
 * A kind of mention (ADR-0032, decision 4): how the "@" list, the chips of
 * the message box and the transcript name the mentions of one provider.
 * Without an entry for a provider, such as while its plugin is off, they
 * show generic texts.
 */
export interface MentionKind {
  /** The heading of the provider's group in the list. */
  readonly group: Message;
  /** An icon from @lucide/svelte for an entry of a variant, such as `file` or `folder`. */
  readonly icon: (variant: string) => Component;
  /** The provider's ID. */
  readonly id: string;
  /** The list's groups are sorted by this number, lowest first. */
  readonly order: number;
  /** The sentence of one of the provider's refusal or failure codes, or `null` for a code it does not know. */
  readonly refusal: (code: string) => string | null;
  /** What the model got of a sent mention, or `null` for a code it does not know. */
  readonly summary: (summary: MentionSummary) => string | null;
  /** What an entry of a variant is, for screen readers, such as "File", or `null` for a variant it does not know. */
  readonly variantName: (variant: string) => string | null;
  /** The words of a warning code, such as "may hold secrets", or `null` for a code it does not know. */
  readonly warning: (code: string) => string | null;
}

/** How a kind of context opens one of its items where it comes from (ADR-0046, decision 5; ADR-0049, decision 4). */
export interface ContextOpen {
  /** The button's label, such as "Open source". */
  readonly label: Message;
  /**
   * Shows the item with this key where it comes from, such as a passage in
   * its source. The key comes from a stored excerpt or from a cite link that
   * the model wrote, so the plugin checks it.
   */
  readonly run: (key: string) => void;
}

/**
 * A kind of context (ADR-0046, decision 6): how the chips of the message box,
 * the parts of the transcript, the status line and the citations name the
 * attachments and items of one context provider, and how its items open.
 * Without an entry for a provider, such as while its plugin is off, they show
 * generic texts, and its items open nowhere.
 */
export interface ContextKind {
  /** The sentence of one of the provider's failure codes, or `null` for a code it does not know. */
  readonly failure: (code: string) => string | null;
  /** The row of a part with items, from its attachments' labels and the number of items, such as "Searched Physics: 5 passages". */
  readonly found: (labels: readonly string[], count: number) => string;
  /** The status line of a reply while the provider gathers, such as "Searching notebooks…". */
  readonly gathering: Message;
  /** An icon from @lucide/svelte for the provider's attachments and parts. */
  readonly icon: Component;
  /** The provider's ID. */
  readonly id: string;
  /** What one of the provider's attachments is, for screen readers, such as "Notebook". */
  readonly noun: Message;
  /** How an item opens, or nothing when the provider's items open nowhere. */
  readonly open?: ContextOpen;
}

/** What a tool kind reads of one call of its plugin's tool (ADR-0039, decision 1). */
export interface ToolKindCall {
  /** The arguments that the model wrote, without the call's activity, or an empty object while they are cut or not an object. */
  readonly arguments: Readonly<Record<string, unknown>>;
  /** The data that the plugin returned for the call's card, a JSON object that the core checked, or `null` without any. */
  readonly data: Readonly<Record<string, unknown>> | null;
  /** The plugin's code of how the call failed, or `null`. */
  readonly failure: string | null;
  /** Where the call is in its life. */
  readonly status: ToolCallStatus;
}

/** What the details of a tool kind receive. */
export interface ToolKindDetailsProps {
  /** The call. */
  readonly call: ToolKindCall;
  /** The conversation the call belongs to. */
  readonly conversationId: ConversationId;
}

/**
 * How the transcript shows the calls of one of a plugin's tools (ADR-0039,
 * decision 1): their icon, their line, the sentence of a failure code and
 * details below the core's accesses. The core uses an entry only for calls of
 * the plugin that registered it, and keeps the accesses, the approval and
 * the outputs that did not fit its own. Without an entry, such as while its
 * plugin is off, a call shows the core's generic texts. An entry with
 * `everyTool` describes every tool of its plugin that has no entry of its
 * own, such as the tools that the plugin publishes while it runs (ADR-0043,
 * decision 9).
 */
export interface ToolKind {
  /** What the call's data shows, below its accesses. It never inserts raw HTML. */
  readonly details?: Component<ToolKindDetailsProps>;
  /** Whether the entry describes every tool of its plugin without an entry of its own; its `id` then names the entry only. */
  readonly everyTool?: boolean;
  /** The sentence of one of the plugin's failure codes, or `null` for a code it does not know. */
  readonly failure: (code: string) => string | null;
  /** An icon from @lucide/svelte for the tool's calls. */
  readonly icon: Component;
  /** The tool's name, as the plugin's tool spec gives it. */
  readonly id: string;
  /** The line of a call, such as "Committed a1b2c3d Fix the parser", or `null` for the core's. */
  readonly summary: (call: ToolKindCall) => string | null;
}

/** An image that a source prepared, as the message box shows and sends it (ADR-0033, decision 7). */
export interface ReadyImage {
  /** Lets the source drop the image, when the owner removes it from the message. */
  readonly discard: () => void;
  /** The image's height in pixels. */
  readonly height: number;
  /** The key that the message's reference names. */
  readonly key: string;
  /** The thumbnail, as a `data:` URL. */
  readonly thumbnailUrl: string;
  /** The image's width in pixels. */
  readonly width: number;
}

/** An image of the message that its source prepares. */
export interface PendingImage {
  /** The image could not be prepared: its chip leaves, and the sentence shows below the message box. */
  readonly fail: (sentence: string) => void;
  /** The image is ready to send. */
  readonly ready: (image: ReadyImage) => void;
}

/** The message that the owner writes, as an image source adds images to it. */
export interface ImageTarget {
  /**
   * Starts an image of the source, which shows as a chip with its name while
   * the source prepares it, or returns `null` when the message holds as many
   * images as it can, which the message box then says.
   */
  readonly begin: (source: string, name: string) => PendingImage | null;
  /** Says below the message box why an image could not be added. */
  readonly refuse: (sentence: string) => void;
}

/** What the button of an image source receives. */
export interface ImageSourceButtonProps {
  /** Whether the message box takes no images now. */
  readonly disabled: boolean;
  /** The message that the owner writes. */
  readonly target: ImageTarget;
}

/**
 * A source of images for the message box (ADR-0033, decision 7): its button,
 * the files it takes from a paste or a drop, and the words of its refusal
 * codes. Without an entry, such as while its plugin is off, the message box
 * offers no images from the source, and pasting and dropping files do nothing.
 */
export interface ImageSourceKind {
  /** The button before the text, such as one that opens the system's file dialog. */
  readonly button: Component<ImageSourceButtonProps>;
  /** The source's ID, as the core knows it. */
  readonly id: string;
  /** Buttons are sorted by this number, lowest first, and the first source that takes a file gets it. */
  readonly order: number;
  /** The sentence of one of the source's refusal codes, or `null` for a code it does not know. */
  readonly refusal: (code: string) => string | null;
  /** Adds files of a paste or a drop that it takes to the message. */
  readonly take: (files: readonly File[], target: ImageTarget) => void;
  /** Whether the source takes a file of a paste or a drop. */
  readonly takes: (file: File) => boolean;
}
