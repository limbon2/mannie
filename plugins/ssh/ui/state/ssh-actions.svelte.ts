// What the owner does with SSH hosts (plugins/ssh/docs/README.md,
// settings): adding and editing a host, checking and trusting its key,
// testing its login and removing it. After a host is added, its key
// dialog opens and reads the key; once the owner trusts the key, the
// dialog tests the login. The dialogs are mounted once, as the plugin's
// overlay, and the plugin's commands go through its generated bindings
// (ADR-0034, decision 9).
import { call } from '@mannie/plugin-sdk/ipc';

import { commands, type HostKeyScan, type HostView } from '../generated/bindings.js';
import { DEFAULT_PORT } from '../generated/plugin.js';
import type { SshFailure } from '../types/ssh.js';
import { isHostGone } from '../utils/ssh-messages.js';

import { hostList } from './hosts.svelte.js';

/** The ID of the SSH section of the settings. */
export const SETTINGS_SECTION = 'ssh.settings';

/** One opening of the host dialog, for a new host or one to edit. */
export class HostRequest {
  address = $state('');
  /** Why the latest attempt failed. */
  failure: SshFailure | null = $state(null);
  /** The host to edit, or `null` for a new one. */
  readonly host: HostView | null;
  name = $state('');
  note = $state('');
  /** A new password; empty keeps the stored one of a host to edit. */
  password = $state('');
  port = $state(DEFAULT_PORT);
  saving = $state(false);
  username = $state('');

  constructor(host: HostView | null) {
    this.host = host;
    if (host !== null) {
      this.address = host.address;
      this.name = host.name;
      this.note = host.note;
      this.port = host.port;
      this.username = host.username;
    }
  }
}

/** One opening of the key dialog: the key that the host shows, trusting it and the login test that follows. */
export class KeyRequest {
  /** Why reading, trusting or testing failed. */
  failure: SshFailure | null = $state(null);
  /** Which step failed, which words the failure's title. */
  failedStep: 'scan' | 'test' | 'trust' | null = $state(null);
  host: HostView;
  /** What the latest scan found. */
  scan: HostKeyScan | null = $state(null);
  scanning = $state(false);
  /** Whether the login test after trusting passed. */
  tested = $state(false);
  testing = $state(false);
  trusting = $state(false);

  constructor(host: HostView) {
    this.host = host;
  }
}

/** One opening of the removal confirmation. */
export class RemoveRequest {
  /** Why the latest attempt failed. */
  failure: SshFailure | null = $state(null);
  readonly host: HostView;
  removing = $state(false);

  constructor(host: HostView) {
    this.host = host;
  }
}

/** A login test that the settings started for a host, with its outcome. */
export interface LoginTest {
  readonly failure: SshFailure | null;
  readonly status: 'failed' | 'passed' | 'testing';
}

/** The owner's actions on SSH hosts, and the dialogs they open. */
export class SshActions {
  /** The latest host dialog, which stays while it closes. */
  edit: HostRequest | null = $state(null);
  editOpen = $state(false);
  /** The latest key dialog, which stays while it closes. */
  key: KeyRequest | null = $state(null);
  keyOpen = $state(false);
  /** The latest removal confirmation, which stays while it closes. */
  removing: RemoveRequest | null = $state(null);
  removeOpen = $state(false);
  /** The login tests that the settings started, by host ID. */
  tests: Partial<Record<string, LoginTest>> = $state({});

  /** Closes every dialog, as the plugin turns off. */
  closeAll(): void {
    this.editOpen = false;
    this.keyOpen = false;
    this.removeOpen = false;
    this.tests = {};
  }

  /** Adds or changes the host of a host dialog, which closes once the core kept it. A new host's key dialog opens next. */
  async confirmHost(request: HostRequest): Promise<void> {
    request.saving = true;
    request.failure = null;
    const fields = {
      address: request.address.trim(),
      name: request.name.trim(),
      note: request.note,
      port: request.port,
      username: request.username.trim(),
    };
    const password = request.password;
    const outcome = await call(() =>
      request.host === null
        ? commands.addHost({ ...fields, password })
        : commands.updateHost({ ...fields, hostId: request.host.id, password: password === '' ? null : password }),
    );
    request.saving = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      return;
    }
    request.password = '';
    hostList.upsert(outcome.data);
    if (this.edit === request) {
      this.editOpen = false;
    }
    const isNew = request.host === null;
    const keyIsUnchecked = outcome.data.status === 'key_not_checked';
    if (isNew || keyIsUnchecked) {
      this.startCheckKey(outcome.data);
    }
  }

  /** Removes the host of a removal confirmation, which closes once the core removed it. */
  async confirmRemove(request: RemoveRequest): Promise<void> {
    request.removing = true;
    request.failure = null;
    const hostId = request.host.id;
    const outcome = await call(() => commands.removeHost({ hostId }));
    request.removing = false;
    if (!outcome.ok && !isHostGone(outcome.failure)) {
      request.failure = outcome.failure;
      return;
    }
    hostList.forget(hostId);
    if (this.removing === request) {
      this.removeOpen = false;
    }
  }

  /** Trusts the key that the key dialog shows, then tests the login with it. */
  async confirmTrust(request: KeyRequest): Promise<void> {
    if (request.scan === null) {
      return;
    }
    request.trusting = true;
    request.failure = null;
    request.failedStep = null;
    const scanned = request.scan;
    const fingerprint = scanned.key.fingerprint;
    const trusted = await call(() => commands.trustHostKey({ fingerprint, hostId: request.host.id }));
    request.trusting = false;
    if (!trusted.ok) {
      request.failure = trusted.failure;
      request.failedStep = 'trust';
      return;
    }
    request.host = trusted.data;
    // The key that the host showed is the trusted one now.
    request.scan = { ...scanned, comparison: 'same' };
    hostList.upsert(trusted.data);
    request.testing = true;
    const tested = await call(() => commands.testHost({ hostId: request.host.id }));
    request.testing = false;
    if (!tested.ok) {
      request.failure = tested.failure;
      request.failedStep = 'test';
      return;
    }
    request.tested = true;
  }

  /** Reads the key that the host of a key dialog shows. */
  async scan(request: KeyRequest): Promise<void> {
    request.scanning = true;
    request.failure = null;
    request.failedStep = null;
    request.scan = null;
    request.tested = false;
    const outcome = await call(() => commands.scanHostKey({ hostId: request.host.id }));
    request.scanning = false;
    if (!outcome.ok) {
      request.failure = outcome.failure;
      request.failedStep = 'scan';
      return;
    }
    request.scan = outcome.data;
  }

  /** Opens the host dialog with empty fields. */
  startAdd(): void {
    this.edit = new HostRequest(null);
    this.editOpen = true;
  }

  /** Opens the key dialog for a host and reads its key. */
  startCheckKey(host: HostView): void {
    const request = new KeyRequest(host);
    this.key = request;
    this.keyOpen = true;
    void this.scan(request);
  }

  /** Opens the host dialog for a host to edit. */
  startEdit(host: HostView): void {
    this.edit = new HostRequest(host);
    this.editOpen = true;
  }

  /** Opens the removal confirmation for a host. */
  startRemove(host: HostView): void {
    this.removing = new RemoveRequest(host);
    this.removeOpen = true;
  }

  /** Tests the login to a host, and keeps the outcome for the settings to show. */
  async test(host: HostView): Promise<void> {
    if (this.tests[host.id]?.status === 'testing') {
      return;
    }
    this.tests = { ...this.tests, [host.id]: { failure: null, status: 'testing' } };
    const outcome = await call(() => commands.testHost({ hostId: host.id }));
    const test: LoginTest = outcome.ok
      ? { failure: null, status: 'passed' }
      : { failure: outcome.failure, status: 'failed' };
    this.tests = { ...this.tests, [host.id]: test };
  }
}

/** The owner's actions on SSH hosts. */
export const sshActions = new SshActions();
