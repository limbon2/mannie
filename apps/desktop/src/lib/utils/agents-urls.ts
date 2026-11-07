// Where the Agents page lives (docs/features/custom-agents.md):
// `?view=agents&agent=<key>` names the agent that the editor shows.
import { page } from '$app/state';

import { activities } from '#lib/state/activities.svelte.js';

/** The ID of the Agents page. */
export const AGENTS_VIEW = 'agents';

/** The search parameter that names the shown agent. */
const AGENT_PARAMETER = 'agent';

/** Shows the Agents page, with an agent or with none, and selects its entry of the activity bar. */
export async function openAgents(key: string | null = null): Promise<void> {
  await activities.openView(AGENTS_VIEW, key === null ? {} : { [AGENT_PARAMETER]: key });
}

/** The agent that the Agents page shows, if it shows one. */
export function shownAgentKey(): string | null {
  return page.url.searchParams.get(AGENT_PARAMETER);
}
