// What the owner does with MCP servers (plugins/mcp/docs/README.md,
// settings): adding and editing a server, turning it on and off, restarting
// it, signing in and out, switching its tools, reading its log, removing it,
// and adding servers from a pasted configuration. The dialogs are mounted
// once, as the plugin's overlay, and the plugin's commands go through its
// generated bindings (ADR-0034, decision 9). A sign-in opens the
// authorization server's page in the browser through the core's checked
// links (ADR-0043, decision 9), after the settings show its host.
import { call } from '@mannie/plugin-sdk/ipc';
import { type LinkFailure, openLink } from '@mannie/plugin-sdk/links';

import {
  commands,
  type ConfiguredServerView,
  type ImportOutcome,
  type LocatedProgramView,
  type ServerLog,
  type ServerTransport,
  type ServerView,
  type ToolView,
} from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import { DEFAULT_TIMEOUT_SECONDS } from '../generated/plugin.js';
import type { McpFailure } from '../types/mcp.js';
import { isServerGone } from '../utils/mcp-messages.js';
import { inputOf, mustBeSecret, type PlainRow, rowsToMove, type SecretRow } from '../utils/server-form.js';

import { serverList } from './servers.svelte.js';

/** The ID of the MCP section of the settings. */
export const SETTINGS_SECTION = 'mcp.settings';

/** One opening of the server dialog, for a new server or one to edit. */
export class ServerRequest {
  argumentsText = $state('');
  clientId = $state('');
  /** A new client secret; empty keeps the stored one. */
  clientSecret = $state('');
  /** Why the latest attempt failed. */
  failure: McpFailure | null = $state(null);
  folder = $state('');
  forgetClientSecret = $state(false);
  images = $state(true);
  instructions = $state(true);
  /** Where the core finds the program, after the owner asked. */
  located: LocatedProgramView | null = $state(null);
  /** Why the program was not found. */
  locateFailure: McpFailure | null = $state(null);
  locating = $state(false);
  /** The names that moved from the plain values to the secrets. */
  moved: string[] = $state([]);
  name = $state('');
  oauth = $state(false);
  plain: PlainRow[] = $state([]);
  prefix = $state('');
  program = $state('');
  saving = $state(false);
  secrets: SecretRow[] = $state([]);
  /** The server to edit, or `null` for a new one. */
  readonly server: ServerView | null;
  timeoutSeconds = $state(DEFAULT_TIMEOUT_SECONDS);
  transport: ServerTransport = $state('stdio');
  url = $state('');

  constructor(server: ServerView | null) {
    this.server = server;
    if (server === null) {
      return;
    }
    this.transport = server.transport;
    this.name = server.name;
    this.prefix = server.prefix;
    this.program = server.program ?? '';
    this.argumentsText = server.arguments.join('\n');
    this.folder = server.folder ?? '';
    this.url = server.url ?? '';
    this.plain = server.plain.map((value) => ({ name: value.name, value: value.value }));
    this.secrets = server.secretNames.map((name) => ({
      name,
      stored: !server.missingSecrets.includes(name),
      value: '',
    }));
    this.oauth = server.oauth;
    this.clientId = server.clientId ?? '';
    this.images = server.images;
    this.instructions = server.instructions;
    this.timeoutSeconds = server.timeoutSeconds;
  }
}

/** One opening of the tools of a server. */
export class ToolsRequest {
  /** Why loading or switching failed. */
  failure: McpFailure | null = $state(null);
  loading = $state(true);
  readonly server: ServerView;
  /** The tool that is being switched, by name. */
  switching: string | null = $state(null);
  tools: ToolView[] = $state([]);

  constructor(server: ServerView) {
    this.server = server;
  }
}

/** One opening of the log of a server. */
export class LogRequest {
  failure: McpFailure | null = $state(null);
  loading = $state(true);
  log: ServerLog | null = $state(null);
  readonly server: ServerView;

  constructor(server: ServerView) {
    this.server = server;
  }
}

/** One opening of the removal confirmation. */
export class RemoveRequest {
  failure: McpFailure | null = $state(null);
  removing = $state(false);
  readonly server: ServerView;

  constructor(server: ServerView) {
    this.server = server;
  }
}

/** One opening of Paste JSON: the text, its servers and the chosen ones. */
export class PasteRequest {
  adding = $state(false);
  /** The keys of the servers to add. */
  chosen: string[] = $state([]);
  failure: McpFailure | null = $state(null);
  outcome: ImportOutcome | null = $state(null);
  /** The servers of the text that was read last, or `null` before. */
  preview: ConfiguredServerView[] | null = $state(null);
  reading = $state(false);
  text = $state('');
}

/** A sign-in that waits for the browser. */
export interface SignIn {
  /** The authorization server's host. */
  readonly host: string;
  /** Why the core did not open the page. */
  readonly linkFailure: LinkFailure | null;
  /** Why the sign-in did not start. */
  readonly startFailure: McpFailure | null;
  readonly status: 'failed' | 'starting' | 'waiting';
  /** The page that the browser opens. */
  readonly url: string;
}

/** A sign-in that starts. */
const STARTING: SignIn = { host: '', linkFailure: null, startFailure: null, status: 'starting', url: '' };

/** The owner's actions on MCP servers, and the dialogs they open. */
export class McpActions {
  /** Why the latest action on a server's row failed, by server ID. */
  actionFailures: Partial<Record<string, McpFailure>> = $state({});
  /** The latest server dialog, which stays while it closes. */
  edit: ServerRequest | null = $state(null);
  editOpen = $state(false);
  log: LogRequest | null = $state(null);
  logOpen = $state(false);
  paste: PasteRequest | null = $state(null);
  pasteOpen = $state(false);
  removeOpen = $state(false);
  removing: RemoveRequest | null = $state(null);
  /** The sign-ins that the settings started, by server ID. */
  signIns: Partial<Record<string, SignIn>> = $state({});
  tools: ToolsRequest | null = $state(null);
  toolsOpen = $state(false);

  /** Adds a row of plain values to the server dialog. */
  addPlain(request: ServerRequest): void {
    request.plain = [...request.plain, { name: '', value: '' }];
  }

  /** Adds a row of secret values to the server dialog. */
  addSecret(request: ServerRequest): void {
    request.secrets = [...request.secrets, { name: '', stored: false, value: '' }];
  }

  /** Closes every dialog, as the plugin turns off. */
  closeAll(): void {
    this.editOpen = false;
    this.logOpen = false;
    this.pasteOpen = false;
    this.removeOpen = false;
    this.toolsOpen = false;
    this.signIns = {};
    this.actionFailures = {};
  }

  /** Closes the tools of a server, and clears the marks of its new tools once the owner saw them. */
  closeTools(request: ToolsRequest): void {
    const hasNew = request.tools.some((tool) => tool.new);
    if (hasNew) {
      void call(() => commands.acknowledgeTools({ serverId: request.server.id }));
    }
  }

  /** Adds or changes the server of a server dialog, which closes once the core kept it. */
  async confirmServer(request: ServerRequest): Promise<void> {
    this.moveSecrets(request);
    request.saving = true;
    request.failure = null;
    const input = inputOf(request);
    const outcome = await call(() =>
      request.server === null
        ? commands.addServer(input)
        : commands.updateServer({ server: input, serverId: request.server.id }),
    );
    request.saving = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }
    request.clientSecret = '';
    for (const row of request.secrets) {
      row.value = '';
    }
    serverList.upsert(outcome.data);
    if (this.edit === request) {
      this.editOpen = false;
    }
  }

  /** Removes the server of a removal confirmation, which closes once the core removed it. */
  async confirmRemove(request: RemoveRequest): Promise<void> {
    request.removing = true;
    request.failure = null;
    const serverId = request.server.id;
    const outcome = await call(() => commands.removeServer({ serverId }));
    request.removing = false;
    if (!outcome.ok && !isServerGone(outcome.failure)) {
      request.failure = outcome.failure;
      return;
    }
    serverList.forget(serverId);
    if (this.removing === request) {
      this.removeOpen = false;
    }
  }

  /** Adds the chosen servers of a pasted configuration. */
  async importChosen(request: PasteRequest): Promise<void> {
    request.adding = true;
    request.failure = null;
    const keys = [...request.chosen];
    const outcome = await call(() => commands.importConfiguration({ keys, text: request.text }));
    request.adding = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }
    request.outcome = outcome.data;
    for (const server of outcome.data.added) {
      serverList.upsert(server);
    }
  }

  /** Loads the log of a log request. */
  async loadLog(request: LogRequest): Promise<void> {
    request.loading = true;
    request.failure = null;
    const outcome = await call(() => commands.serverLog({ serverId: request.server.id }));
    request.loading = false;
    if (outcome.ok) {
      request.log = outcome.data;
      return;
    }
    request.failure = outcome.failure;
  }

  /** Loads the tools of a tools request. */
  async loadTools(request: ToolsRequest): Promise<void> {
    request.loading = true;
    request.failure = null;
    const outcome = await call(() => commands.serverTools({ serverId: request.server.id }));
    request.loading = false;
    if (outcome.ok) {
      request.tools = outcome.data;
      return;
    }
    request.failure = outcome.failure;
  }

  /** Asks the core where it finds the program of the server dialog. */
  async locate(request: ServerRequest): Promise<void> {
    const program = request.program.trim();
    if (program === '') {
      return;
    }
    request.locating = true;
    request.located = null;
    request.locateFailure = null;
    const outcome = await call(() => commands.locateProgram({ program }));
    request.locating = false;
    if (outcome.ok) {
      request.located = outcome.data;
      return;
    }
    request.locateFailure = outcome.failure;
  }

  /** Moves the plain values whose names must be secret among the secrets, as the core requires. */
  moveSecrets(request: ServerRequest): void {
    const names = rowsToMove(request);
    if (names.length === 0) {
      return;
    }
    const moving = request.plain.filter((row) => mustBeSecret(request.transport, row.name.trim()));
    request.plain = request.plain.filter((row) => !mustBeSecret(request.transport, row.name.trim()));
    request.secrets = [
      ...request.secrets,
      ...moving.map((row) => ({ name: row.name.trim(), stored: false, value: row.value })),
    ];
    request.moved = [...request.moved, ...names];
  }

  /** Reads the servers of a pasted configuration, and chooses those that can be added. */
  async readPaste(request: PasteRequest): Promise<void> {
    request.reading = true;
    request.failure = null;
    request.outcome = null;
    const outcome = await call(() => commands.parseConfiguration({ text: request.text }));
    request.reading = false;
    if (!outcome.ok) {
      request.preview = null;
      request.failure = outcome.failure;
      return;
    }
    request.preview = outcome.data;
    request.chosen = outcome.data.filter((server) => server.refusal === null).map((server) => server.key);
  }

  /** Removes a row of plain values from the server dialog. */
  removePlain(request: ServerRequest, index: number): void {
    request.plain = request.plain.toSpliced(index, 1);
  }

  /** Removes a row of secret values from the server dialog. */
  removeSecret(request: ServerRequest, index: number): void {
    request.secrets = request.secrets.toSpliced(index, 1);
  }

  /** Connects to a server again. */
  async restart(server: ServerView): Promise<void> {
    this.#clearFailure(server.id);
    const outcome = await call(() => commands.restartServer({ serverId: server.id }));
    if (!outcome.ok) {
      this.actionFailures = { ...this.actionFailures, [server.id]: outcome.failure };
    }
  }

  /** Turns a server on or off. */
  async setEnabled(server: ServerView, enabled: boolean): Promise<void> {
    this.#clearFailure(server.id);
    const outcome = await call(() => commands.setServerEnabled({ enabled, serverId: server.id }));
    if (outcome.ok) {
      serverList.upsert(outcome.data);
      return;
    }
    this.actionFailures = { ...this.actionFailures, [server.id]: outcome.failure };
  }

  /** Lets agents use a tool of the tools request or not. */
  async setToolEnabled(request: ToolsRequest, tool: ToolView, enabled: boolean): Promise<void> {
    request.switching = tool.name;
    request.failure = null;
    const outcome = await call(() =>
      commands.setToolEnabled({ enabled, serverId: request.server.id, tool: tool.name }),
    );
    request.switching = null;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }
    request.tools = request.tools.map((known) =>
      known.name === tool.name ? { ...known, enabled, new: false } : known,
    );
  }

  /** Starts a sign-in and opens the authorization server's page in the browser. */
  async signIn(server: ServerView): Promise<void> {
    this.signIns = { ...this.signIns, [server.id]: STARTING };
    const page = {
      failed: m.mcp_sign_in_page_failed(),
      signedIn: m.mcp_sign_in_page_signed_in(),
      title: m.mcp_sign_in_page_title(),
    };
    const started = await call(() => commands.beginSignIn({ page, serverId: server.id }));
    if (!started.ok) {
      this.signIns = { ...this.signIns, [server.id]: { ...STARTING, startFailure: started.failure, status: 'failed' } };
      return;
    }
    const { host, url } = started.data;
    this.signIns = { ...this.signIns, [server.id]: { ...STARTING, host, status: 'waiting', url } };
    await this.openSignIn(server.id);
  }

  /** Opens the page of a waiting sign-in again. */
  async openSignIn(serverId: string): Promise<void> {
    const signIn = this.signIns[serverId];
    if (signIn?.status !== 'waiting') {
      return;
    }
    const failure = await openLink(signIn.url);
    if (failure !== null) {
      this.signIns = { ...this.signIns, [serverId]: { ...signIn, linkFailure: failure, status: 'failed' } };
    }
  }

  /** Forgets a server's OAuth tokens. */
  async signOut(server: ServerView): Promise<void> {
    this.#clearFailure(server.id);
    const outcome = await call(() => commands.signOut({ serverId: server.id }));
    if (outcome.ok) {
      serverList.upsert(outcome.data);
      return;
    }
    this.actionFailures = { ...this.actionFailures, [server.id]: outcome.failure };
  }

  /** Opens the server dialog with empty fields. */
  startAdd(): void {
    this.edit = new ServerRequest(null);
    this.editOpen = true;
  }

  /** Opens the server dialog for a server to edit. */
  startEdit(server: ServerView): void {
    this.edit = new ServerRequest(server);
    this.editOpen = true;
  }

  /** Opens the log of a server and loads it. */
  startLog(server: ServerView): void {
    const request = new LogRequest(server);
    this.log = request;
    this.logOpen = true;
    void this.loadLog(request);
  }

  /** Opens Paste JSON. */
  startPaste(): void {
    this.paste = new PasteRequest();
    this.pasteOpen = true;
  }

  /** Opens the removal confirmation for a server. */
  startRemove(server: ServerView): void {
    this.removing = new RemoveRequest(server);
    this.removeOpen = true;
  }

  /** Opens the tools of a server and loads them. */
  startTools(server: ServerView): void {
    const request = new ToolsRequest(server);
    this.tools = request;
    this.toolsOpen = true;
    void this.loadTools(request);
  }

  #clearFailure(serverId: string): void {
    const { [serverId]: _cleared, ...others } = this.actionFailures;
    this.actionFailures = others;
  }
}

/** The owner's actions on MCP servers. */
export const mcpActions = new McpActions();
