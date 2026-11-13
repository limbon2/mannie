import { definePlugin } from '@mannie/plugin-sdk';
import GlobeIcon from '@mannie/plugin-sdk/icons/globe';

import BrowserControl from './components/BrowserControl.svelte';
import BrowserPanel from './components/BrowserPanel.svelte';
import BrowserSettingsSection from './components/BrowserSettingsSection.svelte';
import BrowserToolDetails from './components/BrowserToolDetails.svelte';
import { m } from './generated/messages.js';
import { BROWSER_SETTINGS_SECTION } from './state/browser-settings.svelte.js';
import { browserErrorMessage, browserStartFailureMessage } from './utils/browser-messages.js';
import { BROWSER_PANEL } from './utils/browser-panel.js';

export default definePlugin({
  failure: browserStartFailureMessage,
  offQuestion: m.browser_off_question,
  register({ slots }) {
    slots.sidePanels.register({
      component: BrowserPanel,
      defaultLabel: m.browser_panel_title,
      hideHeading: true,
      icon: GlobeIcon,
      id: BROWSER_PANEL,
    });
    slots.conversationControls.register({ component: BrowserControl, id: 'browser.control', order: 45 });
    slots.settingsSections.register({
      component: BrowserSettingsSection,
      description: m.browser_settings_description,
      icon: GlobeIcon,
      id: BROWSER_SETTINGS_SECTION,
      order: 75,
      title: m.browser_panel_title,
    });
    const tools = {
      browser_interact: m.browser_tool_interact,
      browser_navigate: m.browser_tool_navigate,
      browser_observe: m.browser_tool_observe,
      browser_request_control: m.browser_tool_request_control,
      browser_tabs: m.browser_tool_tabs,
    };
    for (const [id, summary] of Object.entries(tools)) {
      slots.toolKinds.register({
        details: BrowserToolDetails,
        failure: browserErrorMessage,
        icon: GlobeIcon,
        id,
        summary: () => summary(),
      });
    }
  },
});
