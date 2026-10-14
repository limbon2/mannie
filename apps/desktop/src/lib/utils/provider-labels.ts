// Labels of the provider settings (docs/features/chat/providers.md): the API
// interfaces, the servers and the reasoning efforts, from the catalog.
import type { ApiInterface, ReasoningEffort, ServerKind } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';

/** Every reasoning effort, in the order the settings offer them. */
export const REASONING_EFFORTS: readonly ReasoningEffort[] = [
  'server_default',
  'none',
  'minimal',
  'low',
  'medium',
  'high',
  'xhigh',
  'max',
];

/** The label of an API interface. */
export function interfaceLabel(api: ApiInterface): string {
  switch (api) {
    case 'chat_completions': {
      return m.chat_providers_interface_chat_completions();
    }
    case 'responses': {
      return m.chat_providers_interface_responses();
    }
  }
}

/** The label of a reasoning effort. */
export function effortLabel(effort: ReasoningEffort): string {
  switch (effort) {
    case 'high': {
      return m.chat_providers_effort_high();
    }
    case 'low': {
      return m.chat_providers_effort_low();
    }
    case 'max': {
      return m.chat_providers_effort_max();
    }
    case 'medium': {
      return m.chat_providers_effort_medium();
    }
    case 'minimal': {
      return m.chat_providers_effort_minimal();
    }
    case 'none': {
      return m.chat_providers_effort_none();
    }
    case 'server_default': {
      return m.chat_providers_effort_server_default();
    }
    case 'xhigh': {
      return m.chat_providers_effort_xhigh();
    }
  }
}

/** The name of a server that the provider test recognized. */
export function serverLabel(server: ServerKind): string {
  switch (server) {
    case 'lm_studio': {
      return m.chat_providers_server_lm_studio();
    }
    case 'ollama': {
      return m.chat_providers_server_ollama();
    }
    case 'open_ai': {
      return m.chat_providers_server_open_ai();
    }
    case 'open_router': {
      return m.chat_providers_server_open_router();
    }
    case 'other': {
      return m.chat_providers_server_other();
    }
    case 'sglang': {
      return m.chat_providers_server_sglang();
    }
    case 'vllm': {
      return m.chat_providers_server_vllm();
    }
  }
}
