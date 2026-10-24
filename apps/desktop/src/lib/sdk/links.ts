// Links of the TypeScript plugin SDK, `@mannie/plugin-sdk/links` (ADR-0043,
// decision 9): a plugin's UI opens a link in the system browser through the
// core's own command, which checks it as it checks the links of model output
// (ADR-0021), such as the page where the owner signs in to a tool server.
// The plugin shows where the link leads before the owner opens it.
import { commands } from '#lib/generated/bindings.js';
import type { LinkFailure } from '#lib/state/link-confirmation.svelte.js';
import { call } from '#lib/utils/calls.js';

export type { LinkFailure } from '#lib/state/link-confirmation.svelte.js';
export { linkFailureMessage } from '#lib/utils/link-messages.js';

/** Opens a link in the system browser after the core's checks, and returns why it did not open, or `null`. */
export async function openLink(url: string): Promise<LinkFailure | null> {
  const outcome = await call(() => commands.openLink({ url }));
  return outcome.ok ? null : outcome.failure;
}
