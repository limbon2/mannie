// Messages for links that could not be checked or opened (ADR-0019): the
// catalog's message for each code the core reports.
import { m } from '#lib/paraglide/messages.js';
import type { LinkFailure } from '#lib/state/link-confirmation.svelte.js';

/** What to tell the owner about a link that could not be checked or opened. */
export function linkFailureMessage(failure: LinkFailure): string {
  if (failure.kind === 'ipc') {
    return m.shell_error_ipc_failed();
  }
  const { error } = failure;
  switch (error.code) {
    case 'invalid_url': {
      return m.markdown_link_invalid();
    }
    case 'open_failed': {
      return m.markdown_link_open_failed();
    }
    case 'too_long': {
      return m.markdown_link_too_long({ maxBytes: new Intl.NumberFormat().format(error.maxBytes) });
    }
    case 'unsupported_scheme': {
      return m.markdown_link_unsupported_scheme({ scheme: error.scheme });
    }
  }
}
