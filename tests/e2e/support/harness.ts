// Calls into the validation harness of the app (apps/desktop/src/lib/
// validation/harness.ts), which only validation builds contain.
/** A Content Security Policy violation the webview reported. */
export interface CspViolation {
  readonly blockedUri: string;
  readonly directive: string;
  readonly sample: string;
  readonly source: string;
}

/** Which ways of running script the page allowed. */
export interface ScriptProbe {
  readonly evalRan: boolean;
  readonly functionConstructorRan: boolean;
  readonly inlineHandlerRan: boolean;
  readonly inlineScriptRan: boolean;
}

interface Distribution {
  readonly max: number;
  readonly p50: number;
  readonly p95: number;
}

/** What the webview received for a channel load. */
export interface ChannelLoadReport {
  readonly batches: number;
  readonly batchIntervals: Distribution;
  readonly deliveryLatency: Distribution;
  readonly deltas: number;
  readonly gaps: number;
  readonly outOfOrder: number;
  readonly sent: { readonly batches: number; readonly deltas: number } | null;
}

interface ChannelLoad {
  readonly deltasPerSecond: number;
  readonly durationMillis: number;
  readonly runs: number;
}

/** A provider profile, as `create_provider_profile` takes it. */
export interface ProfileInput {
  readonly credential: string | null;
  readonly draft: {
    readonly baseUrl: string;
    readonly contextWindow: number;
    readonly interface: 'chat_completions' | 'responses';
    readonly model: string;
    readonly name: string;
    readonly outputLimit: number;
    readonly reasoningEffort: 'high' | 'low' | 'max' | 'medium' | 'minimal' | 'none' | 'server_default' | 'xhigh';
    readonly useTools: boolean;
  };
}

/** Whether the main window shows and whether the tray exists. */
export interface WindowState {
  readonly tray: boolean;
  readonly visible: boolean;
}

/** A Browser page window as the core reports it (V13). */
export interface BrowserSurface {
  readonly cursor: readonly [number, number] | null;
  readonly cursorAccent: readonly [number, number, number] | null;
  readonly enabled: boolean;
  readonly focused: boolean;
  readonly height: number;
  readonly tab: string;
  readonly visible: boolean;
  readonly width: number;
  readonly x: number;
  readonly y: number;
}

/** Why a check for updates or a download failed, as the core reports it. */
type UpdateFailureCode =
  | 'download_failed'
  | 'no_package_for_this_installation'
  | 'no_update_information'
  | 'signature_invalid'
  | 'unreachable';

/** What the fake source of a validation build's updates answers a check with. */
export type UpdateOffer =
  | { readonly code: UpdateFailureCode; readonly kind: 'check_fails' }
  | { readonly code: UpdateFailureCode; readonly kind: 'download_fails'; readonly version: string }
  | { readonly kind: 'nothing' }
  | { readonly kind: 'version'; readonly version: string };

/** How the last install ended, as a start after it finds it. */
export interface UpdateOutcome {
  readonly fromVersion: string;
  readonly result: 'completed' | 'failed';
  readonly toVersion: string;
}

interface Harness {
  browserEvaluate(conversationId: string, tabId: string, expression: string): Promise<string | null>;
  browserSurface(conversationId: string): Promise<BrowserSurface[]>;
  callUndeclaredPluginCommand(): Promise<string>;
  callUngrantedCommand(): Promise<string>;
  closeMainWindow(): Promise<void>;
  createProviderProfile(input: ProfileInput): Promise<string>;
  cspViolations(): readonly CspViolation[];
  deleteProviderProfiles(): Promise<number>;
  expireChatGptToken(): Promise<void>;
  insertHostileHtml(payloads: readonly string[]): Promise<boolean>;
  openedLinks(): Promise<string[]>;
  probeScripts(): Promise<ScriptProbe>;
  processId(): Promise<number>;
  runChannelLoad(load: ChannelLoad): Promise<ChannelLoadReport>;
  seedConversation(turns: number): Promise<string>;
  setChatGptMock(issuer: string, apiBase: string): Promise<void>;
  setFileChoice(files: string[]): Promise<void>;
  setFolderChoice(folder: string | null): Promise<void>;
  setSkillsFolder(folder: string | null): Promise<void>;
  setUpdateOffer(offer: UpdateOffer): Promise<void>;
  setUpdateOutcome(outcome: UpdateOutcome | null): Promise<void>;
  trayMenu(item: 'open' | 'quit'): Promise<void>;
  updateInstalls(): Promise<string[]>;
  windowState(): Promise<WindowState>;
}

declare global {
  interface Window {
    /** Set by any hostile payload that manages to run. */
    compromised?: boolean;
    mannieValidation?: Harness;
  }
}

/**
 * SvelteKit 3 fetches `_app/version.json` when the window gains focus, to
 * notice a new deployment; the policy's connect-src refuses it. The fetch
 * cannot succeed or matter in a desktop app, and the V1 record lists it.
 */
function isSvelteKitVersionCheck(violation: CspViolation): boolean {
  const isConnect = violation.directive === 'connect-src';
  const isVersionFile = violation.blockedUri.endsWith('/_app/version.json');
  return isConnect && isVersionFile;
}

// The functions given to browser.execute run in the webview, so each one
// looks the harness up itself.

/** The Content Security Policy violations so far, without SvelteKit's version checks. */
export async function cspViolations(): Promise<CspViolation[]> {
  const violations = await browser.execute(() => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.cspViolations();
  });
  return violations.filter((violation) => !isSvelteKitVersionCheck(violation));
}

/** Calls the command that no capability grants. Returns `allowed`, or Tauri's refusal. */
/**
 * Calls a command that the fixture plugin does not declare, through the
 * plugin transport, and returns the code of the refusal, or `allowed`.
 */
export async function callUndeclaredPluginCommand(): Promise<string> {
  return browser.execute(async () => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.callUndeclaredPluginCommand();
  });
}

export async function callUngrantedCommand(): Promise<string> {
  return browser.execute(async () => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.callUngrantedCommand();
  });
}

/**
 * Tries eval, the Function constructor, an inline script and an inline event
 * handler from the app's own code, where the page's policy applies. Code that
 * WebDriver injects runs outside the policy, so it cannot show this.
 */
export async function probeScripts(): Promise<ScriptProbe> {
  return browser.execute(async () => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.probeScripts();
  });
}

/** Inserts each payload into the page from the app's own code. Returns whether any payload ran. */
export async function insertHostileHtml(payloads: readonly string[]): Promise<boolean> {
  return browser.execute(
    async (inserted) => {
      const harness = window.mannieValidation;
      if (harness === undefined) {
        throw new Error('the validation harness is missing: run the validation build');
      }
      return harness.insertHostileHtml(inserted);
    },
    [...payloads],
  );
}

/** Streams a synthetic load from the core and reports what arrived. */
export async function runChannelLoad(load: ChannelLoad): Promise<ChannelLoadReport> {
  return browser.execute(async (requested) => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.runChannelLoad(requested);
  }, load);
}

/** Closes the main window as its close button does: the core decides what happens. */
export async function closeMainWindow(): Promise<void> {
  await browser.execute(async () => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    await harness.closeMainWindow();
  });
}

/** Adds a provider profile without the settings view, and returns its ID. */
export async function createProviderProfile(input: ProfileInput): Promise<string> {
  return browser.execute(async (profile) => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.createProviderProfile(profile);
  }, input);
}

/** Deletes every provider profile and the credentials they stored. Returns how many there were. */
export async function deleteProviderProfiles(): Promise<number> {
  return browser.execute(async () => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.deleteProviderProfiles();
  });
}

/** The links the app would have opened in the browser, in order. */
export async function openedLinks(): Promise<string[]> {
  return browser.execute(async () => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.openedLinks();
  });
}

/** The ID of the app's process. */
export async function processId(): Promise<number> {
  return browser.execute(async () => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.processId();
  });
}

/** Adds a conversation with this many turns, each with a message and a reply, and returns its ID. */
export async function seedConversation(turns: number): Promise<string> {
  return browser.execute(async (count) => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.seedConversation(count);
  }, turns);
}

/**
 * Sets what the next folder dialog answers, since WebDriver cannot reach the
 * system's dialog: a folder, or `null` for a dialog that the owner closed.
 */
export async function setFolderChoice(folder: string | null): Promise<void> {
  await browser.execute(async (chosen) => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    await harness.setFolderChoice(chosen);
  }, folder);
}

/**
 * Names the folder of this computer's skills, or `null` for none, since a
 * validation build never reads the owner's `~/.agents/skills`.
 */
/**
 * Points the app's ChatGPT sign-in and its calls on the ChatGPT plan at the
 * mock of OpenAI in the provider server (ADR-0062).
 */
export async function setChatGptMock(issuer: string, apiBase: string): Promise<void> {
  await browser.execute(
    async (mockIssuer, mockApi) => {
      const harness = window.mannieValidation;
      if (harness === undefined) {
        throw new Error('the validation harness is missing: run the validation build');
      }
      await harness.setChatGptMock(mockIssuer, mockApi);
    },
    issuer,
    apiBase,
  );
}

/** Makes the ChatGPT account's access token expire, as an hour of use would. */
export async function expireChatGptToken(): Promise<void> {
  await browser.execute(async () => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    await harness.expireChatGptToken();
  });
}

export async function setSkillsFolder(folder: string | null): Promise<void> {
  await browser.execute(async (chosen) => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    await harness.setSkillsFolder(chosen);
  }, folder);
}

/**
 * Sets the files that the next file dialog answers, since WebDriver cannot
 * reach the system's dialog: none for a dialog that the owner closed.
 */
export async function setFileChoice(files: readonly string[]): Promise<void> {
  await browser.execute(async (chosen) => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    await harness.setFileChoice(chosen);
  }, files as string[]);
}

/**
 * Chooses an item of the tray's menu. "Quit" without active replies ends the
 * app, and with it the WebDriver session, so the call does not wait for an
 * answer.
 */
export async function chooseTrayItem(item: 'open' | 'quit'): Promise<void> {
  await browser.execute((chosen) => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    void harness.trayMenu(chosen);
  }, item);
}

/** Whether the main window shows and whether the tray exists. */
export async function windowState(): Promise<WindowState> {
  return browser.execute(async () => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.windowState();
  });
}

/** Where each Browser page window of a conversation is (V13). */
export async function browserSurfaces(conversationId: string): Promise<BrowserSurface[]> {
  return browser.execute(async (id) => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.browserSurface(id);
  }, conversationId);
}

/** Evaluates an expression in a Browser page and returns its value, or `null`. */
export async function browserEvaluate(conversationId: string, tabId: string, expression: string): Promise<unknown> {
  const json = await browser.execute(
    async (id, tab, code) => {
      const harness = window.mannieValidation;
      if (harness === undefined) {
        throw new Error('the validation harness is missing: run the validation build');
      }
      return harness.browserEvaluate(id, tab, code);
    },
    conversationId,
    tabId,
    expression,
  );
  return json === null ? null : (JSON.parse(json) as unknown);
}

/**
 * Sets what the next checks for updates find. A validation build reads a fake
 * source and never the releases (ADR-0063, decision 5).
 */
export async function setUpdateOffer(offer: UpdateOffer): Promise<void> {
  await browser.execute(async (chosen) => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    await harness.setUpdateOffer(chosen);
  }, offer);
}

/**
 * Sets how the last install ended, as a start after it finds it, or drops
 * that with `null`. The tests cannot install a version.
 */
export async function setUpdateOutcome(outcome: UpdateOutcome | null): Promise<void> {
  await browser.execute(async (chosen) => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    await harness.setUpdateOutcome(chosen);
  }, outcome);
}

/**
 * The versions that "Restart and update" would have installed, in order. A
 * validation build records the request and keeps running.
 */
export async function updateInstalls(): Promise<string[]> {
  return browser.execute(async () => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.updateInstalls();
  });
}
