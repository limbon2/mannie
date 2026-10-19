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

interface Harness {
  callUndeclaredPluginCommand(): Promise<string>;
  callUngrantedCommand(): Promise<string>;
  closeMainWindow(): Promise<void>;
  createProviderProfile(input: ProfileInput): Promise<string>;
  cspViolations(): readonly CspViolation[];
  deleteProviderProfiles(): Promise<number>;
  insertHostileHtml(payloads: readonly string[]): Promise<boolean>;
  openedLinks(): Promise<string[]>;
  probeScripts(): Promise<ScriptProbe>;
  processId(): Promise<number>;
  runChannelLoad(load: ChannelLoad): Promise<ChannelLoadReport>;
  seedConversation(turns: number): Promise<string>;
  setFileChoice(files: string[]): Promise<void>;
  setFolderChoice(folder: string | null): Promise<void>;
  trayMenu(item: 'open' | 'quit'): Promise<void>;
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
