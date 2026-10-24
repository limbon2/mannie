// The kind of context of the plugin's provider (ADR-0046, decision 6): how the
// chips of the message box, the parts of the transcript, the status line and
// the citations name notebooks and their passages, and how a passage opens in
// its source's side panel.
import BookOpenIcon from '@mannie/plugin-sdk/icons/book-open';
import type { ContextKind } from '@mannie/plugin-sdk/slots';

import { m } from '../generated/messages.js';
import { PLUGIN_ID } from '../generated/plugin.js';

import { contextFailureMessage, contextFoundMessage } from './notebooks-messages.js';
import { openPassage } from './notebooks-places.js';

/** The kind of context of the provider `notebooks`, whose ID is the plugin's. */
export const NOTEBOOKS_CONTEXT: ContextKind = {
  failure: contextFailureMessage,
  found: contextFoundMessage,
  gathering: m.notebooks_context_gathering,
  icon: BookOpenIcon,
  id: PLUGIN_ID,
  noun: m.notebooks_context_noun,
  open: {
    label: m.notebooks_open_source,
    run: (key) => openPassage(key),
  },
};
