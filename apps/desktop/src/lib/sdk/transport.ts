// The transport of the TypeScript plugin SDK, `@mannie/plugin-sdk/transport`
// (ADR-0034, decision 9): it carries a plugin's calls to the host. Only a
// plugin's generated bindings import it, which ESLint checks, so plugin code
// never names the route of its calls.
export { pluginTransport, type PluginTransport } from '#lib/generated/plugin-transport.js';
