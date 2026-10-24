// The slot catalog of the TypeScript plugin SDK, `@mannie/plugin-sdk/slots`
// (ADR-0034, decision 11; docs/architecture/extension-points.md, UI shell
// slots): every slot of the shell with the type of its entries, as one
// record. A new slot is added here and in the shell's registry
// (`state/shell-slots.svelte.ts`), which the type checks hold to this
// record, plus the component that shows its entries.
import type {
  ContextKind,
  ConversationControl,
  ImageSourceKind,
  MentionKind,
  NavigationEntry,
  Overlay,
  PartRenderer,
  SettingsSection,
  ShellCommand,
  ShellView,
  SidebarPanel,
  SidePanelKind,
  TaskKind,
  ToolKind,
} from '#lib/types/shell-slots.js';

// What the model got of a sent mention, which a mention kind words; the
// attachments of a conversation and the items of a context provider, which a
// context kind words and opens (ADR-0046).
export type {
  Attachment,
  AttachmentRef,
  ContextItemView,
  ContextLocation,
  MentionSummary,
} from '#lib/generated/bindings.js';
export type {
  ContextKind,
  ContextOpen,
  ConversationControl,
  ConversationControlPlace,
  ConversationControlProps,
  ConversationDraft,
  DraftControlProps,
  ImageSourceButtonProps,
  ImageSourceKind,
  ImageTarget,
  MentionKind,
  Message,
  NavigationEntry,
  Overlay,
  PartRenderer,
  PartRendererProps,
  PendingImage,
  ReadyImage,
  SettingsSection,
  ShellCommand,
  ShellView,
  SidebarPanel,
  SidePanelKind,
  SidePanelProps,
  TaskKind,
  ToolKind,
  ToolKindCall,
  ToolKindDetailsProps,
} from '#lib/types/shell-slots.js';

/** Every slot of the shell by its name, with the type of its entries. */
export interface SlotCatalog {
  readonly commands: ShellCommand;
  readonly contextKinds: ContextKind;
  readonly conversationControls: ConversationControl;
  readonly imageSources: ImageSourceKind;
  readonly mentionKinds: MentionKind;
  readonly navigation: NavigationEntry;
  readonly overlays: Overlay;
  readonly partRenderers: PartRenderer;
  readonly settingsSections: SettingsSection;
  readonly sidebarPanels: SidebarPanel;
  readonly sidePanels: SidePanelKind;
  readonly taskKinds: TaskKind;
  readonly toolKinds: ToolKind;
  readonly views: ShellView;
}

/** A slot as a plugin registers in it: each entry belongs to the plugin. */
export interface PluginSlot<T> {
  /** Adds an entry of the plugin, which shows only while the plugin is on. Registering an ID twice throws. */
  readonly register: (entry: T) => void;
}

/** Every slot of the shell as one plugin registers in it. */
export type PluginSlots = { readonly [Name in keyof SlotCatalog]: PluginSlot<SlotCatalog[Name]> };
