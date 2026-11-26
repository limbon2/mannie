// Settings actions and dialogs for Proxmox VE servers: adding/editing a
// server, TLS-only certificate inspection and leaf pinning, choosing which
// QEMU VMs are allowed for Mannie (fail-closed pre-configure step), testing
// connections, and removing servers (ADR-0080).
import { call } from '@mannie/plugin-sdk/ipc';
import { openConversationId } from '@mannie/plugin-sdk/navigation';

import {
  type AllowedVmEntry,
  type CertificateInfo,
  commands,
  type DiscoverableVmView,
  type ServerView,
  type TestConnectionResult,
} from '../generated/bindings.js';
import { DEFAULT_PORT } from '../generated/plugin.js';
import type { ProxmoxFailure } from '../types/proxmox.js';
import { isCertificateFailure, isServerGone } from '../utils/proxmox-messages.js';
import { openVncPanel } from '../utils/proxmox-panel.js';

import { serverList } from './servers.svelte.js';

/** Settings section ID for the Proxmox plugin. */
export const SETTINGS_SECTION = 'proxmox.settings';

/** Options when opening the TLS certificate inspection dialog. */
export interface InspectCertificateOptions {
  readonly continueToVms?: boolean;
}

/** One opening of the Add / Edit Proxmox server dialog. */
export class ServerRequest {
  failure: ProxmoxFailure | null = $state(null);
  host = $state('');
  name = $state('');
  note = $state('');
  password = $state('');
  port = $state(DEFAULT_PORT);
  saving = $state(false);
  readonly server: ServerView | null;
  username = $state('');

  constructor(server: ServerView | null) {
    this.server = server;
    if (server !== null) {
      this.host = server.host;
      this.name = server.name;
      this.note = server.note;
      this.port = server.port;
      this.username = server.username;
    }
  }
}

/** One opening of the TLS certificate inspection and pinning dialog. */
export class CertificateRequest {
  clearing = $state(false);
  readonly continueToVms: boolean;
  failure: ProxmoxFailure | null = $state(null);
  info: CertificateInfo | null = $state(null);
  inspecting = $state(false);
  pinned = $state(false);
  pinning = $state(false);
  server: ServerView;

  constructor(server: ServerView, continueToVms: boolean) {
    this.server = server;
    this.continueToVms = continueToVms;
  }
}

/** One opening of the pre-configure allowed VMs dialog. */
export class AllowedVmsRequest {
  failure: ProxmoxFailure | null = $state(null);
  loading = $state(false);
  saving = $state(false);
  selectedIds: number[] = $state([]);
  server: ServerView;
  vms: DiscoverableVmView[] = $state([]);

  constructor(server: ServerView) {
    this.server = server;
    this.selectedIds = server.allowedVms.map((vm) => vm.vmid);
  }

  clearAll(): void {
    this.selectedIds = [];
  }

  isSelected(vmid: number): boolean {
    return this.selectedIds.includes(vmid);
  }

  selectAll(): void {
    this.selectedIds = this.vms.map((vm) => vm.vmid);
  }

  toggleVm(vmid: number, checked: boolean): void {
    if (checked) {
      if (!this.selectedIds.includes(vmid)) {
        this.selectedIds = [...this.selectedIds, vmid];
      }
      return;
    }
    this.selectedIds = this.selectedIds.filter((id) => id !== vmid);
  }
}

/** One opening of the Remove Proxmox server confirmation dialog. */
export class RemoveServerRequest {
  failure: ProxmoxFailure | null = $state(null);
  removing = $state(false);
  readonly server: ServerView;

  constructor(server: ServerView) {
    this.server = server;
  }
}

/** Outcome of an inline connection test in Settings. */
export interface ConnectionTestState {
  readonly failure: ProxmoxFailure | null;
  readonly result: TestConnectionResult | null;
  readonly status: 'failed' | 'passed' | 'testing';
}

/** Coordinates Proxmox Settings dialogs and commands. */
export class ProxmoxActions {
  cert: CertificateRequest | null = $state(null);
  certOpen = $state(false);
  consoleFailure: ProxmoxFailure | null = $state(null);
  consoleFailureOpen = $state(false);
  edit: ServerRequest | null = $state(null);
  editOpen = $state(false);
  removeOpen = $state(false);
  removing: RemoveServerRequest | null = $state(null);
  tests: Partial<Record<string, ConnectionTestState>> = $state({});
  vms: AllowedVmsRequest | null = $state(null);
  vmsOpen = $state(false);

  /** Clears a server's pinned certificate so connections use OS trust. */
  async clearCertificatePin(request: CertificateRequest): Promise<void> {
    request.clearing = true;
    request.failure = null;
    const received = serverList.received;
    const outcome = await call(() => commands.clearCertificatePin({ serverId: request.server.id }));

    request.clearing = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }
    request.server = outcome.data;
    request.pinned = false;
    serverList.upsert(outcome.data, received);
    await this.inspectCertificate(request);
  }

  /** Closes all Proxmox Settings dialogs when the plugin stops. */
  closeAll(): void {
    this.editOpen = false;
    this.certOpen = false;
    this.consoleFailureOpen = false;
    this.consoleFailure = null;
    this.vmsOpen = false;
    this.removeOpen = false;
    this.tests = {};
  }

  /** Saves the owner's selected VM allowlist for a server. */
  async confirmAllowedVms(request: AllowedVmsRequest): Promise<void> {
    request.saving = true;
    request.failure = null;
    const entries: AllowedVmEntry[] = request.vms
      .filter((vm) => request.isSelected(vm.vmid))
      .map((vm) => ({ name: vm.name, node: vm.node, vmid: vm.vmid }));
    const received = serverList.received;
    const outcome = await call(() => commands.setAllowedVms({ serverId: request.server.id, vms: entries }));

    request.saving = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }

    request.server = outcome.data;
    serverList.upsert(outcome.data, received);
    if (this.vms === request) {
      this.vmsOpen = false;
    }
  }

  /** Pins the inspected leaf certificate fingerprint, then continues to VM selection if requested. */
  async confirmPinCertificate(request: CertificateRequest): Promise<void> {
    if (request.info === null) {
      return;
    }

    request.pinning = true;
    request.failure = null;
    const sha256 = request.info.sha256;
    const received = serverList.received;
    const outcome = await call(() => commands.pinCertificate({ serverId: request.server.id, sha256 }));

    request.pinning = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }

    request.server = outcome.data;
    request.pinned = true;
    request.info = { ...request.info, matchesPin: true };
    serverList.upsert(outcome.data, received);
    const isActiveDialog = this.cert === request;
    if (request.continueToVms && isActiveDialog) {
      this.certOpen = false;
      this.startChooseVms(outcome.data);
    }
  }

  /** Removes a server and deletes its stored password. */
  async confirmRemove(request: RemoveServerRequest): Promise<void> {
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

  /** Adds or updates a Proxmox server; new servers immediately enter VM selection (or cert pin first). */
  async confirmServer(request: ServerRequest): Promise<void> {
    request.saving = true;
    request.failure = null;
    const fields = {
      host: request.host.trim(),
      name: request.name.trim(),
      note: request.note,
      port: request.port,
      username: request.username.trim(),
    };
    const password = request.password;
    const received = serverList.received;
    const previousEndpoint = request.server?.endpoint ?? null;
    const outcome = await call(() =>
      request.server === null
        ? commands.addServer({ ...fields, password })
        : commands.updateServer({
            ...fields,
            password: password === '' ? null : password,
            serverId: request.server.id,
          }),
    );

    request.saving = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }

    request.password = '';
    serverList.upsert(outcome.data, received);
    if (this.edit === request) {
      this.editOpen = false;
    }

    const isNew = request.server === null;
    const endpointChanged = previousEndpoint !== null && previousEndpoint !== outcome.data.endpoint;
    if (isNew || endpointChanged) {
      this.startChooseVms(outcome.data, true);
    }
  }

  /** Runs a TLS-only certificate inspection for a server without sending credentials. */
  async inspectCertificate(request: CertificateRequest): Promise<void> {
    request.inspecting = true;
    request.failure = null;
    request.info = null;
    request.pinned = false;
    const outcome = await call(() => commands.inspectCertificate({ serverId: request.server.id }));

    request.inspecting = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }

    request.info = outcome.data;
  }

  /** Loads all QEMU VMs visible to the server account for the allowlist dialog. */
  async loadDiscoverableVms(request: AllowedVmsRequest, redirectCertOnTlsError = false): Promise<void> {
    request.loading = true;
    request.failure = null;
    const outcome = await call(() => commands.discoverServerVms({ serverId: request.server.id }));

    request.loading = false;
    if (!outcome.ok) {
      const isCertError = isCertificateFailure(outcome.failure);
      const isCurrentDialog = this.vms === request;
      if (redirectCertOnTlsError && isCertError && isCurrentDialog) {
        this.vmsOpen = false;
        this.startInspectCert(request.server, { continueToVms: true });
        return;
      }

      request.failure = outcome.failure;
      return;
    }

    request.vms = outcome.data;
    request.selectedIds = outcome.data.filter((vm) => vm.allowed).map((vm) => vm.vmid);
  }

  /** Opens a live VNC session and its side panel for an allowed VM in a conversation. */
  async openConsole(
    conversationId: string,
    serverId: string,
    vmid: number,
    vmName: string,
    opener?: HTMLElement | null,
  ): Promise<void> {
    const outcome = await call(() => commands.openVncSession({ conversationId, serverId, vmid }));
    if (!outcome.ok) {
      if (openConversationId() === conversationId) {
        this.consoleFailure = outcome.failure;
        this.consoleFailureOpen = true;
      }
      return;
    }

    await openVncPanel({ conversationId, serverId, vmid }, vmName, { opener });
  }

  /** Opens the Add Proxmox server dialog. */
  startAdd(): void {
    this.edit = new ServerRequest(null);
    this.editOpen = true;
  }

  /** Opens the pre-configure allowed VMs dialog and fetches QEMU VMs from the server. */
  startChooseVms(server: ServerView, redirectCertOnTlsError = false): void {
    const request = new AllowedVmsRequest(server);

    this.vms = request;
    this.vmsOpen = true;
    void this.loadDiscoverableVms(request, redirectCertOnTlsError);
  }

  /** Opens the Edit Proxmox server dialog. */
  startEdit(server: ServerView): void {
    this.edit = new ServerRequest(server);
    this.editOpen = true;
  }

  /** Opens the TLS certificate inspection dialog and starts a TLS-only probe. */
  startInspectCert(server: ServerView, options: InspectCertificateOptions = {}): void {
    const request = new CertificateRequest(server, options.continueToVms === true);

    this.cert = request;
    this.certOpen = true;
    void this.inspectCertificate(request);
  }

  /** Opens the Remove server confirmation dialog. */
  startRemove(server: ServerView): void {
    this.removing = new RemoveServerRequest(server);
    this.removeOpen = true;
  }

  /** Tests TLS trust, authentication, and VM visibility for a server. */
  async testConnection(server: ServerView): Promise<void> {
    if (this.tests[server.id]?.status === 'testing') {
      return;
    }
    this.tests = { ...this.tests, [server.id]: { failure: null, result: null, status: 'testing' } };
    const outcome = await call(() => commands.testConnection({ serverId: server.id }));
    const next: ConnectionTestState = outcome.ok
      ? { failure: null, result: outcome.data, status: 'passed' }
      : { failure: outcome.failure, result: null, status: 'failed' };

    this.tests = { ...this.tests, [server.id]: next };
  }
}

/** Shared Proxmox Settings action coordinator. */
export const proxmoxActions = new ProxmoxActions();
