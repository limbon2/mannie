// Messages for the plugin's codes (ADR-0019): the core words a call that got
// no answer and the host's refusals, and the plugin its own errors.
import { hostRefusalMessage, ipcFailureMessage } from '@mannie/plugin-sdk/messages';

import type { DesignerError } from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import type { DesignerFailure } from '../types/designer.js';

/** The sentence of one of the plugin's errors. */
function errorMessage(error: DesignerError): string {
  switch (error.code) {
    case 'name_taken': {
      return m.designer_error_name_taken();
    }
    case 'storage_failed': {
      return m.designer_error_storage_failed();
    }
  }
}

/** What to tell the owner about a call of the plugin that failed, and what they can do. */
export function designerFailureMessage(failure: DesignerFailure): string {
  if (failure.kind === 'ipc') {
    return ipcFailureMessage();
  }
  const { error } = failure;
  if (error.origin === 'host') {
    return hostRefusalMessage(error, m.designer_plugin_name());
  }
  return errorMessage(error);
}
